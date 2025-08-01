use crate::address::BitcoinAddress;
use crate::logs::{P0, P1};
use crate::management::{fetch_btc_exchange_rate, get_siwb_principal};
use crate::memo::MintMemo;
use crate::state::{mutate_state, read_state, UtxoCheckStatus};
use crate::tasks::{schedule_now, TaskType};
use candid::{CandidType, Deserialize, Nat, Principal};
use ic_btc_interface::{GetUtxosError, GetUtxosResponse, OutPoint, Utxo};
use ic_canister_log::log;
use ic_ckbtc_kyt::Error as KytError;
use ic_xrc_types::ExchangeRateError;
use icrc_ledger_client_cdk::{CdkRuntime, ICRC1Client};
use icrc_ledger_types::icrc1::{
    account::{Account, Subaccount},
    transfer::{Memo, TransferArg, TransferError}
};
use num_traits::ToPrimitive;
use serde::Serialize;
use super::get_btc_address::{GetBoxAddressArgs, SyronOperation};
use super::get_withdrawal_account::compute_subaccount;
use super::retrieve_btc::{balance_of, SyronLedger};
use crate::{
    guard::{balance_update_guard, GuardError},
    management::{fetch_utxo_alerts, get_utxos, CallError, CallSource},
    state,
    tx::{DisplayAmount, DisplayOutpoint},
    updates::get_btc_address,
};

/// The argument of the [update_balance] endpoint.
#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpdateBalanceArgs {
    /// The owner of the account on the ledger.
    /// The minter uses the caller principal if the owner is None.
    pub owner: Option<Principal>,
    /// The desired subaccount on the ledger, if any.
    pub subaccount: Option<Subaccount>,
}

/// The outcome of UTXO processing.
#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum UtxoStatus {
    /// The UTXO might have a transfer inscription
    TransferInscription(Utxo),
    /// The UTXO value does not cover the KYT check cost.
    ValueTooSmall(Utxo),
    /// The KYT check found issues with the deposited UTXO.
    Tainted(Utxo),
    /// The deposited UTXO passed the KYT check, but the minter failed to mint ckBTC on the ledger.
    /// The caller should retry the [update_balance] call.
    Checked(Utxo),
    /// The minter accepted the UTXO and minted ckBTC tokens on the ledger.
    Minted {
        /// The MINT transaction index on the ledger.
        block_index: u64,
        /// The minted amount (UTXO value minus fees).
        minted_amount: u64,
        /// The UTXO that caused the balance update.
        utxo: Utxo,
    },
    Read(Utxo)
}

pub enum ErrorCode {
    ConfigurationError = 1,
    UnsupportedOperation = 2,
    InsufficientAmount = 3
}

#[derive(CandidType, Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PendingUtxo {
    pub outpoint: OutPoint,
    pub value: u64,
    pub confirmations: u32,
}

#[derive(CandidType, Clone, Debug, Deserialize, PartialEq, Eq)]
pub enum UpdateBalanceError {
    /// The minter experiences temporary issues, try the call again later.
    TemporarilyUnavailable(String),
    /// There is a concurrent [update_balance] invocation from the same caller.
    AlreadyProcessing,
    /// The minter didn't discover new UTXOs with enough confirmations.
    NoNewUtxos {
        /// If there are new UTXOs that do not have enough
        /// confirmations yet, this field will contain the number of
        /// confirmations as observed by the minter.
        current_confirmations: Option<u32>,
        /// The minimum number of UTXO confirmation required for the minter to accept a UTXO.
        required_confirmations: u32,
        /// List of utxos that don't have enough confirmations yet to be processed.
        pending_utxos: Option<Vec<PendingUtxo>>,
    },
    GenericError {
        error_code: u64,
        error_message: String,
    },
    CallError {
        method: String,
        reason: String
    },
    SystemError {
        method: String,
        reason: String
    },
}

impl From<GuardError> for UpdateBalanceError {
    fn from(e: GuardError) -> Self {
        match e {
            GuardError::AlreadyProcessing => Self::AlreadyProcessing,
            GuardError::TooManyConcurrentRequests => {
                Self::TemporarilyUnavailable("too many concurrent requests".to_string())
            }
        }
    }
}

impl From<GetUtxosError> for UpdateBalanceError {
    fn from(e: GetUtxosError) -> Self {
        Self::GenericError {
            error_code: ErrorCode::ConfigurationError as u64,
            error_message: format!("failed to get UTXOs from the Bitcoin canister: {}", e),
        }
    }
}

impl From<TransferError> for UpdateBalanceError {
    fn from(e: TransferError) -> Self {
        Self::GenericError {
            error_code: ErrorCode::ConfigurationError as u64,
            error_message: format!("failed to mint tokens on the ledger: {:?}", e),
        }
    }
}

impl From<CallError> for UpdateBalanceError {
    fn from(e: CallError) -> Self {
        Self::TemporarilyUnavailable(e.to_string())
    }
}

impl From<ExchangeRateError> for UpdateBalanceError {
    fn from(e: ExchangeRateError) -> Self {
        Self::TemporarilyUnavailable(format!("failed to fetch the current exchange rate: {:?}", e))
    }
}

#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CollateralizedAccount {
    exchange_rate: u64,
    pub collateral_ratio: u64,
    pub btc_1: u64,
    pub susd_1: u64,
    pub susd_2: u64,
    pub susd_3: u64
}

/// Notifies the ckBTC minter to update the balance of the user subaccount.
// pub async fn update_balance(
//     args: UpdateBalanceArgs,
// ) -> Result<Vec<u64/*UtxoStatus*/>, UpdateBalanceError> {
//     //let controller = args.owner.unwrap_or_else(ic_cdk::caller);
//     let minter = ic_cdk::id();
//     // if controller == ic_cdk::id() {
//     //     ic_cdk::trap("cannot update minter's balance");
//     // }

//     // state::read_state(|s| s.mode.is_deposit_available_for(&controller))
//     //     .map_err(UpdateBalanceError::TemporarilyUnavailable)?;

//     init_ecdsa_public_key().await;
    
//     //let _guard = balance_update_guard(controller)?; @review (guard)

//     //@syron Deposit account = Withdrawal account
    
//     let ssi_subaccount = compute_subaccount(PrincipalId(minter), 1, &args.ssi);
    
//     let caller_account = Account {
//         owner: minter,
//         subaccount: Some(ssi_subaccount),
//     };

//     ic_cdk::println!("Account: {}", caller_account);

//     let address = state::read_state(|s| {
//         get_btc_address::account_to_p2wpkh_address_from_state(s, &caller_account, &args.ssi)
//     });

//     ic_cdk::println!("SSI Vault: {}", address);

//     let (btc_network, min_confirmations) =
//         state::read_state(|s| (s.btc_network, s.min_confirmations));

//     let utxos = get_utxos(btc_network, &address, min_confirmations, CallSource::Client)
//         .await?
//         .utxos;

//     let new_utxos = state::read_state(|s| s.new_utxos_for_account(utxos, &caller_account));

//     // Remove pending finalized transactions for the affected principal.
//     state::mutate_state(|s| s.finalized_utxos.remove(&caller_account.owner));

//     let satoshis_to_mint = new_utxos.iter().map(|u| u.value).sum::<u64>();

//     if satoshis_to_mint == 0 {
//         // We bail out early if there are no UTXOs to avoid creating a new entry
//         // in the UTXOs map. If we allowed empty entries, malicious callers
//         // could exhaust the canister memory.

//         // We get the entire list of UTXOs again with a zero
//         // confirmation limit so that we can indicate the approximate
//         // wait time to the caller.
//         let GetUtxosResponse {
//             tip_height,
//             mut utxos,
//             ..
//         } = get_utxos(
//             btc_network,
//             &address,
//             /*min_confirmations=*/ 0,
//             CallSource::Client,
//         )
//         .await?;

//         utxos.retain(|u| {
//             tip_height
//                 < u.height
//                     .checked_add(min_confirmations)
//                     .expect("bug: this shouldn't overflow")
//                     .checked_sub(1)
//                     .expect("bug: this shouldn't underflow")
//         });
//         let pending_utxos: Vec<PendingUtxo> = utxos
//             .iter()
//             .map(|u| PendingUtxo {
//                 outpoint: u.outpoint.clone(),
//                 value: u.value,
//                 confirmations: tip_height - u.height + 1,
//             })
//             .collect();

//         let current_confirmations = pending_utxos.iter().map(|u| u.confirmations).max();

//         return Err(UpdateBalanceError::NoNewUtxos {
//             current_confirmations,
//             required_confirmations: min_confirmations,
//             pending_utxos: Some(pending_utxos),
//         });
//     }

//     let token_name = match btc_network {
//         ic_ic00_types::BitcoinNetwork::Mainnet => "ckBTC",
//         _ => "ckTESTBTC",
//     };

//     let kyt_fee = 0;//read_state(|s| s.kyt_fee); @review(kyt)
    
//     let mut utxo_statuses: Vec<UtxoStatus> = vec![];
    
//     let mut ckbtc_amount: u64 = 0;
//     for utxo in new_utxos {
//         if utxo.value <= kyt_fee {
//             mutate_state(|s| crate::state::audit::ignore_utxo(s, utxo.clone()));
//             log!(
//                 P1,
//                 "Ignored UTXO {} for account {caller_account} because UTXO value {} is lower than the KYT fee {}",
//                 DisplayOutpoint(&utxo.outpoint),
//                 DisplayAmount(utxo.value),
//                 DisplayAmount(kyt_fee),
//             );
//             utxo_statuses.push(UtxoStatus::ValueTooSmall(utxo));
//             continue;
//         }
//         let (uuid, status, kyt_provider) = kyt_check_utxo(caller_account.owner, &utxo).await?;
        
//         // @review(kyt) state change implications
//         mutate_state(|s| {
//             crate::state::audit::mark_utxo_checked(s, &utxo, uuid.clone(), status, kyt_provider);
//         });
//         if status == UtxoCheckStatus::Tainted {
//             utxo_statuses.push(UtxoStatus::Tainted(utxo.clone()));
//             continue;
//         }
//         let amount = utxo.value - kyt_fee;
//         // let memo = MintMemo::Convert {
//         //     txid: Some(utxo.outpoint.txid.as_ref()),
//         //     vout: Some(utxo.outpoint.vout),
//         //     kyt_fee: Some(kyt_fee),
//         // };
//         ckbtc_amount += amount
//     }

//     let res = mint(ckbtc_amount, caller_account, /*crate::memo::encode(&memo).into(),*/).await;
    
//     // @review (mint) log
//     // match res {
//     //     Ok(res) => {
//     //         log!(
//     //             P1,
//     //             "Minted {ckbtc_amount} {token_name} for account {caller_account}"// corresponding to utxo {} with value {}",
//     //             // DisplayOutpoint(&utxo.outpoint),
//     //             // DisplayAmount(utxo.value),
//     //         );

//     //         // @review (utxo)
//     //         // state::mutate_state(|s| {
//     //         //     state::audit::add_utxos(
//     //         //         s,
//     //         //         Some(block_index),
//     //         //         caller_account,
//     //         //         vec![utxo.clone()],
//     //         //     )
//     //         // });
//     //         // utxo_statuses.push(UtxoStatus::Minted {
//     //         //     block_index,
//     //         //     utxo,
//     //         //     minted_amount: ckbtc_amount,
//     //         // });
//     //     }
//     //     Err(err) => {
//     //         log!(
//     //             P0,
//     //             "Failed to mint SUSD - Error: {:?}",
//     //             // DisplayOutpoint(&utxo.outpoint),
//     //             err
//     //         );

//     //         // @review (utxo)
//     //         // utxo_statuses.push(UtxoStatus::Checked(utxo));
//     //     }
//     // }

//     schedule_now(TaskType::ProcessLogic);

//     return res;
//     // Ok(utxo_statuses)
// }

/// Notifies the minter to update the balance of the user subaccount.
pub async fn update_ssi_balance(
    args: GetBoxAddressArgs,
) -> Result<Vec<UtxoStatus>, UpdateBalanceError> {
    let minter = ic_cdk::id();
    // @dev get user ssi account
    let ssi_subaccount = compute_subaccount(0, &args.ssi);
    let ssi_account = Account {
        owner: minter,
        subaccount: Some(ssi_subaccount)
    };

    state::read_state(|s| s.mode.is_deposit_available_for(&ssi_account))
        .map_err(UpdateBalanceError::TemporarilyUnavailable)?;

    // @review (guard) the guard was moved to the upstream function
    // let _guard = balance_update_guard(ssi_account.clone())?;

    let ssi_box_subaccount = compute_subaccount(1, &args.ssi);
    
    let mut utxo_statuses: Vec<UtxoStatus> = vec![];

    match args.op {
        SyronOperation::GetSyron => {
            let ssi_box_account = Account {
                owner: minter,
                subaccount: Some(ssi_box_subaccount)
            };
            
            let ssi_balance_subaccount = compute_subaccount(2, &args.ssi);
            let ssi_balance_account = Account {
                owner: minter,
                subaccount: Some(ssi_balance_subaccount)
            };
        
            let box_address = state::read_state(|s| {
                get_btc_address::ssi_account_to_p2wpkh_address_from_state(s, &ssi_box_account, &args.ssi)
            });
        
            let (btc_network, min_confirmations) =
                state::read_state(|s| (s.btc_network, s.min_confirmations));
        
            let utxos = get_utxos(btc_network, &box_address, min_confirmations, CallSource::Client)
                .await?
                .utxos;
        
            let new_utxos = state::read_state(|s| s.new_utxos_for_account(utxos, &ssi_box_account));
        
            // Remove pending finalized transactions
            state::mutate_state(|s| s.finalized_utxos.remove(&ssi_box_account));
        
            let btc_deposit = new_utxos.iter().map(|u| u.value).sum::<u64>();
        
            if btc_deposit == 0 {
                // We bail out early if there are no UTXOs to avoid creating a new entry
                // in the UTXOs map. If we allowed empty entries, malicious callers
                // could exhaust the canister memory.
        
                // We get the entire list of UTXOs again with a zero
                // confirmation limit so that we can indicate the approximate
                // wait time to the caller.
                let GetUtxosResponse {
                    tip_height,
                    mut utxos,
                    ..
                } = get_utxos(
                    btc_network,
                    &box_address,
                    /*min_confirmations=*/ 0,
                    CallSource::Client,
                )
                .await?;
        
                utxos.retain(|u| {
                    tip_height
                        < u.height
                            .checked_add(min_confirmations)
                            .expect("bug: this shouldn't overflow")
                            .checked_sub(1)
                            .expect("bug: this shouldn't underflow")
                });
                let pending_utxos: Vec<PendingUtxo> = utxos
                    .iter()
                    .map(|u| PendingUtxo {
                        outpoint: u.outpoint.clone(),
                        value: u.value,
                        confirmations: tip_height - u.height + 1,
                    })
                    .collect();
        
                let current_confirmations = pending_utxos.iter().map(|u| u.confirmations).max();
        
                return Err(UpdateBalanceError::NoNewUtxos {
                    current_confirmations,
                    required_confirmations: min_confirmations,
                    pending_utxos: Some(pending_utxos),
                });
            }
        
            let token_name = match btc_network {
                ic_management_canister_types::BitcoinNetwork::Mainnet => "syron",
                _ => "tSyron",
            };

            let kyt_fee = read_state(|s| s.kyt_fee);
            let min_deposit = read_state(|s| s.min_btc_deposit);

            for utxo in new_utxos {
                if utxo.value < min_deposit {
                    mutate_state(|s| crate::state::audit::ignore_utxo(s, utxo.clone()));
                    log!(
                        P1,
                        "Ignored UTXO {} for account {ssi_box_account} because UTXO value {} is lower than the minimum deposit amount {}",
                        DisplayOutpoint(&utxo.outpoint),
                        DisplayAmount(utxo.value),
                        DisplayAmount(min_deposit),
                    );
                    utxo_statuses.push(UtxoStatus::ValueTooSmall(utxo));
                    continue;
                }
                
                // @review (kyt)
                // let (uuid, status, kyt_provider) = kyt_check_utxo(caller_account.owner, &utxo).await?;
                // mutate_state(|s| {
                //     crate::state::audit::mark_utxo_checked(s, &utxo, uuid.clone(), status, kyt_provider);
                // });
                // if status == UtxoCheckStatus::Tainted {
                //     utxo_statuses.push(UtxoStatus::Tainted(utxo.clone()));
                //     continue;
                // }
                let amount = utxo.value - kyt_fee;
                let memo = MintMemo::Convert {
                    txid: Some(utxo.outpoint.txid.as_ref()),
                    vout: Some(utxo.outpoint.vout),
                    kyt_fee: Some(kyt_fee),
                };
        
                match mint(&args.ssi, amount, ssi_box_account, crate::memo::encode(&memo).into(), ssi_balance_account).await {
                    Ok(block_index) => {
                        log!(
                            P1,
                            "Minted {amount} {token_name} for account {ssi_box_account} corresponding to utxo {} with value {}",
                            DisplayOutpoint(&utxo.outpoint),
                            DisplayAmount(utxo.value),
                        );

                        // @dev save UTXO to state to prevent double spending
                        state::mutate_state(|s| {
                            state::audit::add_utxos(
                                false,
                                s,
                                Some(block_index[0]),
                                ssi_box_account,
                                vec![utxo.clone()],
                                Some(args.ssi.clone()),
                            )
                        });
                        utxo_statuses.push(UtxoStatus::Minted {
                            block_index: block_index[0],
                            utxo,
                            minted_amount: amount,
                        });
                    }
                    Err(err) => {
                        log!(
                            P0,
                            "Failed to mint for UTXO {}: {:?}",
                            DisplayOutpoint(&utxo.outpoint),
                            err
                        );
                        utxo_statuses.push(UtxoStatus::Checked(utxo));
                        return Err(err);
                    }
                }
            }
        
            // let res = match mint(satoshis_to_mint, caller_account).await {
            //     Ok(res) => Ok(utxo_statuses),
            //     Err(res) => Err(res)
            // };
            // return res
        },
        SyronOperation::RedeemBitcoin => {
            let minter_account = Account{
                owner: minter,
                subaccount: None
            };

            let btc_1 = balance_of(SyronLedger::BTC, &args.ssi, 1).await.unwrap_or(0);
            let susd_1 = balance_of(SyronLedger::SYRON, &args.ssi, 1).await.unwrap_or(0);
    
            // @dev Throw an error if the bitcoin collateral balance is zero
            if btc_1 == 0 {
                return Err(UpdateBalanceError::GenericError {
                    error_code: ErrorCode::UnsupportedOperation as u64,
                    error_message: "@update_ssi_balance: Invalid balance to redeem BTC".to_string()
                });
            }

            // Syron bitcoin ledger
            let sbtc_client = ICRC1Client {
                runtime: CdkRuntime,
                ledger_canister_id: state::read_state(|s| s.ledger_id.get().into()),
            };
            sbtc_client
                .transfer(TransferArg {
                    from_subaccount: Some(ssi_box_subaccount),
                    to: minter_account,
                    fee: None,
                    created_at_time: None,
                    memo: None,
                    amount: Nat::from(btc_1),
                })
                .await
                .map_err(|(code, msg)| {
                    UpdateBalanceError::TemporarilyUnavailable(format!(
                        "update_ssi_balance: Cannot redeem bitcoin: {} (reject_code = {})",
                        msg, code
                    ))
                })??;
        
            // Syron stablecoin ledger
            if susd_1 != 0 {
                let susd_client = ICRC1Client {
                    runtime: CdkRuntime,
                    ledger_canister_id: state::read_state(|s| s.susd_id.get().into()),
                };

                susd_client
                .transfer(TransferArg {
                    from_subaccount: Some(ssi_box_subaccount),
                    to: minter_account,
                    fee: None,
                    created_at_time: None,
                    memo: None,
                    amount: Nat::from(susd_1),
                })
                .await
                .map_err(|(code, msg)| {
                    UpdateBalanceError::TemporarilyUnavailable(format!(
                        "update_ssi_balance: Cannot grant loan repayment: {} (reject_code = {})",
                        msg, code
                    ))
                })??;
            }
        },
        SyronOperation::Liquidation => {
            // invalid operation, throw error
            return Err(UpdateBalanceError::GenericError {  
                error_code: ErrorCode::UnsupportedOperation as u64,
                error_message: "@update_ssi_balance: Invalid liquidation operation".to_string()
            });
        },
        SyronOperation::Payment => {
            // invalid operation, throw error
            return Err(UpdateBalanceError::GenericError {  
                error_code: ErrorCode::UnsupportedOperation as u64,
                error_message: "@update_ssi_balance: Invalid payment operation".to_string()
            });
        },
        SyronOperation::DepositSyron => {
            let current_runes_deposit = balance_of(SyronLedger::SYRON, &args.ssi, 5).await.unwrap_or(0);
            // @dev throw error if zero
            if current_runes_deposit == 0 {
                return Err(UpdateBalanceError::GenericError {
                    error_code: ErrorCode::InsufficientAmount as u64,
                    error_message: "@update_ssi_balance: No runes deposit balance available".to_string()
                });
            }
            let _index = syron_update(&args.ssi, 5, Some(2), current_runes_deposit).await?;
        }
    }
    schedule_now(TaskType::ProcessLogic);
        
    Ok(utxo_statuses)
}

/// Notifies the minter to update its own balance of syron runes.
pub async fn update_runes_balance(utxos: (Vec<Utxo>, Vec<Utxo>)) -> Result<Vec<UtxoStatus>, UpdateBalanceError> {
    // @dev get minter runes address
    let dao_addr = state::read_state(|s| s.dao_addr.clone());
    
    // @note the ssi address for the runes minter is the treasury address (nonce 1)
    let treasury_address = &dao_addr[1];

    let (btc_network, min_confirmations) =
        state::read_state(|s| (s.btc_network, s.min_confirmations));
        
    let treasury_addr = treasury_address.display(btc_network); 
    let runes_minter_subaccount = compute_subaccount(1, &treasury_addr);
    let runes_minter_account = Account {
        owner: ic_cdk::id(),
        subaccount: Some(runes_minter_subaccount)
    };
    state::read_state(|s| s.mode.is_deposit_available_for(&runes_minter_account))
        .map_err(UpdateBalanceError::TemporarilyUnavailable)?;

    let _guard = balance_update_guard(runes_minter_account.clone())?;

    let mut utxo_statuses: Vec<UtxoStatus> = vec![];

    let new_sats_utxos = state::read_state(|s| s.new_utxos_for_account(utxos.0, &runes_minter_account));
    let new_runes_utxos = state::read_state(|s| s.new_utxos_for_account(utxos.1, &runes_minter_account));
    let mut total_utxos = new_sats_utxos.clone();
    total_utxos.extend(new_runes_utxos.clone());

    // Remove pending finalized transactions for the account
    state::mutate_state(|s| s.finalized_utxos.remove(&runes_minter_account));

    // @dev add new sats to runes minter balance
    let btc_deposit = new_sats_utxos.iter().map(|u| u.value).sum::<u64>();

    // @dev the runes minter address (nonce 2)
    let runes_minter_addr = &dao_addr[2].display(btc_network);

    if btc_deposit == 0 {
        // We bail out early if there are no UTXOs to avoid creating a new entry
        // in the UTXOs map. If we allowed empty entries, malicious callers
        // could exhaust the canister memory.

        // We get the entire list of UTXOs again with a zero
        // confirmation limit so that we can indicate the approximate
        // wait time to the caller.
        let GetUtxosResponse {
            tip_height,
            mut utxos,
            ..
        } = get_utxos(
            btc_network,
            &runes_minter_addr,
            min_confirmations,
            CallSource::Client,
        )
        .await?;

        utxos.retain(|u| {
            tip_height
                < u.height
                    .checked_add(min_confirmations)
                    .expect("bug: this shouldn't overflow")
                    .checked_sub(1)
                    .expect("bug: this shouldn't underflow")
        });
        let pending_utxos: Vec<PendingUtxo> = utxos
            .iter()
            .map(|u| PendingUtxo {
                outpoint: u.outpoint.clone(),
                value: u.value,
                confirmations: tip_height - u.height + 1,
            })
            .collect();

        let current_confirmations = pending_utxos.iter().map(|u| u.confirmations).max();

        return Err(UpdateBalanceError::NoNewUtxos {
            current_confirmations,
            required_confirmations: min_confirmations,
            pending_utxos: Some(pending_utxos),
        });
    }

    // @dev use box subaccount for gas and runes subaccount for stablecoin balances of the runes minter
    let runes_subaccount = compute_subaccount(4, &treasury_addr);
    let runes_ledger_account = Account {
        owner: ic_cdk::id(),
        subaccount: Some(runes_subaccount)
    };

    for utxo in new_sats_utxos {
        let memo = MintMemo::Convert {
            txid: Some(utxo.outpoint.txid.as_ref()),
            vout: Some(utxo.outpoint.vout),
            kyt_fee: None,
        };

        match count_runes_minter(utxo.value, runes_minter_account, crate::memo::encode(&memo).into()).await {
            Ok(block_index) => {
                state::mutate_state(|s| {
                    state::audit::add_utxos(
                        false,
                        s,
                        Some(block_index),
                        runes_minter_account,
                        vec![utxo.clone()],
                        None,
                    )
                });
                utxo_statuses.push(UtxoStatus::Read(utxo));
            }
            Err(err) => {
                log!(
                    P0,
                    "Failed to mint for UTXO {}: {:?}",
                    DisplayOutpoint(&utxo.outpoint),
                    err
                );
                utxo_statuses.push(UtxoStatus::Checked(utxo));
                return Err(err);
            }
        }
    }

    for utxo in new_runes_utxos {
        let memo = MintMemo::Convert {
            txid: Some(utxo.outpoint.txid.as_ref()),
            vout: Some(utxo.outpoint.vout),
            kyt_fee: None,
        };

        match count_runes_minter(utxo.value, runes_ledger_account, crate::memo::encode(&memo).into()).await {
            Ok(block_index) => {
                state::mutate_state(|s| {
                    state::audit::add_utxos(
                        true,
                        s,
                        Some(block_index),
                        runes_minter_account,
                        vec![utxo.clone()],
                        None,
                    )
                });
                utxo_statuses.push(UtxoStatus::Read(utxo));
            }
            Err(err) => {
                log!(
                    P0,
                    "Failed to mint for UTXO {}: {:?}",
                    DisplayOutpoint(&utxo.outpoint),
                    err
                );
                utxo_statuses.push(UtxoStatus::Checked(utxo));
                return Err(err);
            }
        }
    }

    schedule_now(TaskType::ProcessLogic);

    Ok(utxo_statuses)
}

async fn count_runes_minter(runes: u64, to: Account, memo: Memo) -> Result<u64, UpdateBalanceError> {
    let btc_client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.ledger_id.get().into()),
    };

    let block_index = btc_client
        .transfer(TransferArg {
            from_subaccount: None,
            to,
            fee: None,
            created_at_time: None,
            memo: None, //Some(memo), // @review (alpha) 'the memo field size of 39 bytes is above the allowed limit of 32 bytes'.
            amount: Nat::from(runes),
        })
        .await
        .map_err(|(code, msg)| {
            UpdateBalanceError::TemporarilyUnavailable(format!(
                "@count_runes_minter: Cannot add runes balance: {} (reject_code = {})",
                msg, code
            ))
            })??;

    let res = block_index.0.to_u64()
        .ok_or_else(|| UpdateBalanceError::SystemError{
            method: "count_runes_minter".to_string(),
            reason: "Block index too large for u64".to_string()
        })?;
    Ok(res)
}

async fn _kyt_check_utxo(
    caller: Principal,
    utxo: &Utxo,
) -> Result<(String, UtxoCheckStatus, Principal), UpdateBalanceError> {
    let kyt_principal = read_state(|s| {
        s.kyt_principal
            .expect("BUG: upgrade procedure must ensure that the KYT principal is set")
            .get()
            .into()
    });

    if let Some((uuid, status, api_key_owner)) = read_state(|s| s.checked_utxos.get(utxo).cloned())
    {
        return Ok((uuid, status, api_key_owner));
    }

    match fetch_utxo_alerts(kyt_principal, caller, utxo)
        .await
        .map_err(|call_err| {
            UpdateBalanceError::TemporarilyUnavailable(format!(
                "Failed to call KYT canister: {}",
                call_err
            ))
        })? {
        Ok(response) => {
            if !response.alerts.is_empty() {
                log!(
                    P0,
                    "Discovered a tainted UTXO {} (external id {})",
                    DisplayOutpoint(&utxo.outpoint),
                    response.external_id
                );
                Ok((
                    response.external_id,
                    UtxoCheckStatus::Tainted,
                    response.provider,
                ))
            } else {
                Ok((
                    response.external_id,
                    UtxoCheckStatus::Clean,
                    response.provider,
                ))
            }
        }
        Err(KytError::TemporarilyUnavailable(reason)) => {
            log!(
                P1,
                "The KYT provider is temporarily unavailable: {}",
                reason
            );
            Err(UpdateBalanceError::TemporarilyUnavailable(format!(
                "The KYT provider is temporarily unavailable: {}",
                reason
            )))
        }
    }
}

/// Registers the amount of bitcoin collateral, the syron loan, and the available balance.
pub(crate) async fn mint(ssi: &str, satoshis: u64, to: Account, memo: Memo, account: Account) -> Result<Vec<u64 /*UtxoStatus*/>, UpdateBalanceError> {
    let collateralized_account = get_collateralized_account(ssi).await?;
    let exchange_rate = collateralized_account.exchange_rate;

    // @notice We assume that the current collateral ratio is >= 15,000 basis points.
    let mut susd: u64 = satoshis * exchange_rate / 15 * 10; //@review (mainnet) over-collateralization ratio (1.5)

    // if the collateral ratio is less than 15000 basis points, then the user cannot withdraw SUSD amount, can withdraw an amount of SUSD so that the collateral ratio is at least 15000 basis points
    if collateralized_account.collateral_ratio < 15000 {
        // calculate the amount of satoshis required so that the collateral ratio is at least 15000 basis points
        let sats = ((1.5 * collateralized_account.susd_1 as f64 / exchange_rate as f64) as u64 - collateralized_account.btc_1).max(0);

        let accepted_deposit = (satoshis - sats).max(0);

        // calculate the maximum amount of susd that can be withdrawn
        if accepted_deposit > 0 {
            // @runes
            // susd = accepted_deposit * exchange_rate / 15 * 10;
        } else {
            // all satoshis are deposited but no new SUSD can be minted
            susd = 0;
        }
    }

    let client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.ledger_id.get().into()),
    };

    // debug_assert!(memo.0.len() <= crate::LEDGER_MEMO_SIZE as usize); @review (mainnet)
    // Canister called `ic0.trap` with message: the memo field size of 39 bytes is above the allowed limit of 32 bytes (reject_code = 5)"
    let block_index_btc1 = client
        .transfer(TransferArg {
            from_subaccount: None,
            to,
            fee: None,
            created_at_time: None,
            memo: None,//Some(memo.clone()), @review (alpha) 'the memo field size of 39 bytes is above the allowed limit of 32 bytes'.
            amount: Nat::from(satoshis),
        })
        .await
        .map_err(|(code, msg)| {
            UpdateBalanceError::TemporarilyUnavailable(format!(
                "@mint: Cannot register bitcoin collateral due to error ({} - reject_code = {})",
                msg, code
            ))
        })??;

    let mut res: Vec<u64> = Vec::new();
    res.push(block_index_btc1.0.to_u64().expect("@mint: Nat does not fit into u64"));

    if susd != 0 {
        // @dev SUSD
    
        let susd_client = ICRC1Client {
            runtime: CdkRuntime,
            ledger_canister_id: state::read_state(|s| s.susd_id.get().into()),
        };

        let block_index_susd1 = susd_client
            .transfer(TransferArg {
                from_subaccount: None,
                to,
                fee: None,
                created_at_time: None,
                memo: None, //Some(memo.clone()), @review (alpha) 'the memo field size of 39 bytes is above the allowed limit of 32 bytes'.
                amount: Nat::from(susd),
            })
            .await
            .map_err(|(code, msg)| {
                UpdateBalanceError::TemporarilyUnavailable(format!(
                    "@mint: Cannot grant syron loan due to error ({} - reject_code = {})",
                    msg, code
                ))
            })??;

        let block_index_susd2 = susd_client
            .transfer(TransferArg {
                from_subaccount: None,
                to: account,
                fee: None,
                created_at_time: None,
                memo: None, //Some(memo.clone()), @review (alpha) 'the memo field size of 39 bytes is above the allowed limit of 32 bytes'.
                amount: Nat::from(susd),
            })
            .await
            .map_err(|(code, msg)| {
                UpdateBalanceError::TemporarilyUnavailable(format!(
                    "@mint: Cannot update syron balance due to error ({} - reject_code = {})", // @review (alpha) if it fails, make sure that the fn fails entirely (revert previous updates in collateral and loan)
                    msg, code
                ))
            })??;

        // return Err(
        //     UpdateBalanceError::TemporarilyUnavailable(format!(
        //         "satoshis: {}, xr: {}, SUSD: {}",
        //         satoshis, xr.rate, susd
        //     ))
        // );
        
        log!(
            P0,
            "Minted {susd} (SUSD) with {satoshis} (BTC) for account {to} at XR: {}",
            DisplayAmount(exchange_rate),
        );

        res.push(block_index_susd1.0.to_u64().expect("@mint: Nat does not fit into u64"));
        res.push(block_index_susd2.0.to_u64().expect("@mint: Nat does not fit into u64"));
    }

    Ok(res)
}

pub async fn syron_update(ssi: &str, from: u64, to: Option<u64>, amt: u64) -> Result<u64, UpdateBalanceError> {
    let from_subaccount = Some(compute_subaccount(from, ssi));
    
    let to_account: Account = match to {
        Some(to) => {
            let to_subaccount = compute_subaccount(to, ssi);
            Account {
                owner: ic_cdk::id(),
                subaccount: Some(to_subaccount)
            }
        },
        None => Account { // means burning the stablecoin
            owner: ic_cdk::id(),
            subaccount: None
        }
    };

    let susd_client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.susd_id.get().into()),
    };
    let block_index_susd = susd_client
    .transfer(TransferArg {
        from_subaccount,
        to: to_account,
        fee: None,
        created_at_time: None,
        memo: None,
        amount: Nat::from(amt),
    })
    .await
    .map_err(|(code, msg)| {
        UpdateBalanceError::GenericError{
            error_code: code as u64,
            error_message: format!(
            "@syron_update: Cannot update syron balance to subaccount with nonce ({}) due to message: {}",
            to.map(|t| t.to_string()).unwrap_or_else(|| "None".to_string()),
            msg)
        }
    })??;
    
    let res = block_index_susd.0.to_u64()
        .ok_or_else(|| UpdateBalanceError::SystemError{
            method: "syron_update".to_string(),
            reason: "Block index too large for u64".to_string()
        })?;
    Ok(res)
}

// @dev add pending runes balance
// explicitly revert if requested
pub async fn syron_runes_deposit(ssi: &str, amt: u64, revert: bool) -> Result<u64, UpdateBalanceError> {
    if amt == 0 {
        return Err(UpdateBalanceError::CallError{
            method: "syron_runes_deposit".to_string(),
            reason: "Amount cannot be zero".to_string()
        });
    }
    
    // @dev use nonce 5 for runes pending deposits
    let minter = ic_cdk::id(); 
    let pending_subaccount = compute_subaccount(5, ssi);
    let pending_account: Account = Account {
        owner: minter,
        subaccount: Some(pending_subaccount)
    };

    let susd_client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.susd_id.get().into()),
    };
    let block_index_susd = match revert {
        true => {
            let minter_account: Account = Account{
                owner: minter,
                subaccount: None
            };
            susd_client
            .transfer(TransferArg {
                from_subaccount: Some(pending_subaccount),
                to: minter_account,
                fee: None,
                created_at_time: None,
                memo: None,
                amount: Nat::from(amt),
            })
            .await
            .map_err(|(code, msg)| {
                UpdateBalanceError::GenericError{
                    error_code: code as u64,
                    error_message: format!(
                    "@syron_runes_deposit: Could not revert syron runes deposit balance: {}",
                    msg)
                }
            })?? 
        }
        false => {
            susd_client
            .transfer(TransferArg {
                from_subaccount: None,
                to: pending_account,
                fee: None,
                created_at_time: None,
                memo: None,
                amount: Nat::from(amt),
            })
            .await
            .map_err(|(code, msg)| {
                UpdateBalanceError::GenericError{
                    error_code: code as u64,
                    error_message: format!(
                    "@syron_runes_deposit: Could not update syron runes deposit balance: {}",
                    msg)
                }
            })??
        }
    };
    
    let res = block_index_susd.0.to_u64()
        .ok_or_else(|| UpdateBalanceError::SystemError{
            method: "syron_runes_deposit".to_string(),
            reason: "Block index too large for u64".to_string()
        })?;
    Ok(res)
}

pub async fn btc_bal_update(ssi: &str, from: u64, to: Option<u64>, amt: u64) -> Result<Vec<u64>, UpdateBalanceError> {
    let from_subaccount = Some(compute_subaccount(from, ssi));
    
    let to_account: Account = match to {
        Some(to) => {
            let to_subaccount = compute_subaccount(to, ssi);
            Account {
                owner: ic_cdk::id(),
                subaccount: Some(to_subaccount)
            }
        },
        None => Account {
            owner: ic_cdk::id(),
            subaccount: None
        }
    };
    
    let sbtc_client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.ledger_id.get().into()),
    };
    let block_index_btc = sbtc_client
    .transfer(TransferArg {
        from_subaccount,
        to: to_account,
        fee: None,
        created_at_time: None,
        memo: None,
        amount: Nat::from(amt),
    })
    .await
    .map_err(|(code, msg)| {
        UpdateBalanceError::GenericError{
            error_code: code as u64,
            error_message: format!(
            "cannot update SBTC balance: {}",
            msg)
        }
    })??;
    
    let res = [
        block_index_btc.0.to_u64()
            .ok_or_else(|| UpdateBalanceError::SystemError{
                method: "btc_bal_update".to_string(),
                reason: "Block index too large for u64".to_string()
            })?
    ];
    Ok(res.to_vec())
}

pub async fn get_collateralized_account(ssi: &str) -> Result<CollateralizedAccount, UpdateBalanceError> {
    let xr = fetch_btc_exchange_rate("USD".to_string()).await??;
    let btc_1 = balance_of(SyronLedger::BTC, ssi, 1).await.unwrap_or(0);
    let susd_1 = balance_of(SyronLedger::SYRON, ssi, 1).await.unwrap_or(0);
    let susd_2 = balance_of(SyronLedger::SYRON, ssi, 2).await.unwrap_or(0);
    let susd_3 = balance_of(SyronLedger::SYRON, ssi, 3).await.unwrap_or(0);
    
    let exchange_rate: u64 = xr.rate / 1_000_000_000;
    
    // if dummy {
    //     if btc_1 != 0 {
    //         (1.15 * susd_1 as f64 / btc_1 as f64) as u64
    //     } else {
    //         xr.rate / 1_000_000_000 / 137 * 100
    //     }
    // } else {
    //     xr.rate / 1_000_000_000
    // };

    let collateral_ratio = if btc_1 == 0 || susd_1 == 0 {
        15000 // 150%
    } else {
        ((btc_1 as f64 * exchange_rate as f64 / susd_1 as f64) * 10000.0) as u64
    };

    Ok(CollateralizedAccount{
        exchange_rate,
        collateral_ratio,
        btc_1,
        susd_1,
        susd_2,
        susd_3
    })
}

pub async fn syron_payment(sender: BitcoinAddress, receiver: BitcoinAddress, amt: u64, btc: Option<u64>) -> Result<Vec<u64>, UpdateBalanceError> {
    // @dev Syron amount cannot be lower than 20 cents @governance
    if amt < 20_000_000 {
        return Err(UpdateBalanceError::GenericError{
            error_code: ErrorCode::InsufficientAmount as u64,
            error_message: format!("@syron_payment: Syron amount ({}) is below the minimum", amt),
        });
    }

    let network = read_state(|s| (s.btc_network));
    let ssi = &sender.display(network);
    let recipient = &receiver.display(network);
    
    let principal = get_siwb_principal(ssi).await?;
    ic_cdk::println!("@syron_payment: SIWB Internet Identity = {:?}", principal);
    
    let mut res = vec![];

    match btc {
        Some(btc) => {
            // @dev BTC amount cannot be lower than 200 sats @governance
            if btc < 200 {
                return Err(UpdateBalanceError::GenericError{
                    error_code: ErrorCode::InsufficientAmount as u64,
                    error_message: format!("@syron_payment: BTC amount ({}) is below the minimum", btc),
                });
            }

            let xr = fetch_btc_exchange_rate("USD".to_string()).await??;
            let exchange_rate: u64 = xr.rate / 1_000_000_000;
            let bitcoin_amount = (amt as f64 / exchange_rate as f64) as u64;
            
            // "bitcoin_amount" must be at least the minimum BTC amount requested by the user ("btc")
            if bitcoin_amount < btc {
                return Err(UpdateBalanceError::GenericError{
                    error_code: ErrorCode::InsufficientAmount as u64,
                    error_message: format!(
                        "@syron_payment: Insufficient BTC amount. Computed amount: {} sats, Minimum Required: {} sats, Exchange Rate: {}",
                        bitcoin_amount, btc, exchange_rate
                    )
                });
            }
            
            // @dev Use subaccount 0 in SBTC ledger for swap credit
            let swap_subaccount = compute_subaccount(0, ssi);
            let swap_account = Account {
                owner: ic_cdk::id(),
                subaccount: Some(swap_subaccount)
            };

            // Syron BTC Ledger
            let sbtc_client = ICRC1Client {
                runtime: CdkRuntime,
                ledger_canister_id: state::read_state(|s| s.ledger_id.get().into()),
            };
            let block_index_btc = sbtc_client
            .transfer(TransferArg {
                from_subaccount: None,
                to: swap_account,
                fee: None,
                created_at_time: None,
                memo: None,
                amount: Nat::from(bitcoin_amount),
            })
            .await
            .map_err(|(code, msg)| {
                UpdateBalanceError::GenericError{
                    error_code: code as u64,
                    error_message: format!(
                    "@syron_payment: Could not update BTC swap credit: {}",
                    msg)
                }
            })??;
        
            res.push(
                block_index_btc.0.to_u64()
                .ok_or_else(|| UpdateBalanceError::SystemError{
                    method: "syron_payment".to_string(),
                    reason: "Block index too large for u64".to_string()
                })?
            );
            ic_cdk::println!("@syron_payment: The user has been credited {:?} satoshis", bitcoin_amount);
        },
        None => {}  
    } 
    
    let from_subaccount = Some(compute_subaccount(2, ssi));
    let to_subaccount = compute_subaccount(2, recipient);
    
    let to_account = Account {
        owner: ic_cdk::id(),
        subaccount: Some(to_subaccount)
    };

    let susd_client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.susd_id.get().into()),
    };
    let block_index_susd = susd_client
    .transfer(TransferArg {
        from_subaccount,
        to: to_account,
        fee: None,
        created_at_time: None,
        memo: None,
        amount: Nat::from(amt),
    })
    .await
    .map_err(|(code, msg)| {
        UpdateBalanceError::GenericError{
            error_code: code as u64,
            error_message: format!(
            "@syron_payment: Could not update the Syron transfer balance: {}",
            msg)
        }
    })??;
    
    res.push(
        block_index_susd.0.to_u64()
        .ok_or_else(|| UpdateBalanceError::CallError{
            method: "syron_payment".to_string(),
            reason: "Block index too large for u64".to_string()
        })?
    );
    ic_cdk::println!("@syron_payment: The user has sent {:?} susd-sats", amt);
    
    Ok(res)
}

pub async fn syron_payment_icp(sender: BitcoinAddress, receiver: Account, amt: u64) -> Result<Vec<u64>, UpdateBalanceError> {
    // @dev Syron amount cannot be lower than 20 cents @governance
    if amt < 20_000_000 {
        return Err(UpdateBalanceError::GenericError{
            error_code: ErrorCode::InsufficientAmount as u64,
            error_message: format!("@syron_payment_icp: Syron amount ({}) is below the minimum", amt),
        });
    }

    let network = read_state(|s| (s.btc_network));
    let ssi = &sender.display(network);
    
    let principal = get_siwb_principal(ssi).await?;
    ic_cdk::println!("@syron_payment_icp: SIWB Internet Identity = {:?}", principal);
    
    let from_subaccount = Some(compute_subaccount(2, ssi));

    let syron_client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.susd_id.get().into()),
    };
    let block_index_susd = syron_client
    .transfer(TransferArg {
        from_subaccount,
        to: receiver,
        fee: None,
        created_at_time: None,
        memo: None,
        amount: Nat::from(amt),
    })
    .await
    .map_err(|(code, msg)| {
        UpdateBalanceError::GenericError{
            error_code: code as u64,
            error_message: format!(
            "@syron_payment_icp: Could not update the Syron transfer balance: {}",
            msg)
        }
    })??;
    
    let res = vec![
        block_index_susd.0.to_u64()
        .ok_or_else(|| UpdateBalanceError::SystemError{
            method: "syron_payment_icp".to_string(),
            reason: "Block index too large for u64".to_string()
        })?
    ];
    ic_cdk::println!("@syron_payment_icp: The user has sent {:?} syron-sats to account: {:?}", amt, receiver);
    
    Ok(res)
}
