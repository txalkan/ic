use crate::logs::{P0, P1};
use crate::management::get_exchange_rate;
use crate::memo::MintMemo;
use crate::state::{mutate_state, read_state, UtxoCheckStatus};
use crate::tasks::{schedule_now, TaskType};
use candid::{CandidType, Deserialize, Nat, Principal};
use ic_base_types::PrincipalId;
use ic_btc_interface::{GetUtxosError, GetUtxosResponse, OutPoint, Utxo};
use ic_canister_log::log;
use ic_ckbtc_kyt::Error as KytError;
use icrc_ledger_client_cdk::{CdkRuntime, ICRC1Client};
use icrc_ledger_types::icrc1::account::{Account, Subaccount};
// use icrc_ledger_types::icrc1::transfer::Memo;
use icrc_ledger_types::icrc1::transfer::{TransferArg, TransferError};
use num_traits::ToPrimitive;
use serde::Serialize;
use super::get_btc_address::init_ecdsa_public_key;
use super::get_withdrawal_account::compute_subaccount;
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
    //pub owner: Option<Principal>,
    /// The desired subaccount on the ledger, if any.
    //pub subaccount: Option<Subaccount>,

    pub ssi: String
}

/// The outcome of UTXO processing.
#[derive(CandidType, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum UtxoStatus {
    /// The UTXO has a transfer inscription
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
}

pub enum ErrorCode {
    ConfigurationError = 1,
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
//     //             "Failed to mint SU$D - Error: {:?}",
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

/// Notifies the ckBTC minter to update the balance of the user subaccount.
pub async fn update_balance(
    args: UpdateBalanceArgs,
) -> Result<Vec<UtxoStatus>, UpdateBalanceError> {
    let minter = ic_cdk::id();
    // let caller = ic_cdk::caller();
    // if args.owner.unwrap_or(caller) == ic_cdk::id() {
    //     ic_cdk::trap("cannot update minter's balance");
    // }

    
    state::read_state(|s| s.mode.is_deposit_available_for(&minter))
        .map_err(UpdateBalanceError::TemporarilyUnavailable)?;

    init_ecdsa_public_key().await;
    let _guard = balance_update_guard(minter/*args.owner.unwrap_or(caller)*/)?;

    let ssi_subaccount = compute_subaccount(1, &args.ssi);
    
    let caller_account = Account {
        owner: minter,//args.owner.unwrap_or(caller),
        subaccount: Some(ssi_subaccount)//args.subaccount,
    };

    let address = state::read_state(|s| {
        get_btc_address::account_to_p2wpkh_address_from_state(s, &caller_account, &args.ssi)
    });

    let (btc_network, min_confirmations) =
        state::read_state(|s| (s.btc_network, s.min_confirmations));

    let utxos = get_utxos(btc_network, &address, min_confirmations, CallSource::Client)
        .await?
        .utxos;

    let new_utxos = state::read_state(|s| s.new_utxos_for_account(utxos, &caller_account));

    // Remove pending finalized transactions for the affected principal.
    state::mutate_state(|s| s.finalized_utxos.remove(&caller_account.owner));

    let satoshis_to_mint = new_utxos.iter().map(|u| u.value).sum::<u64>();

    if satoshis_to_mint == 0 {
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
            &address,
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
        ic_ic00_types::BitcoinNetwork::Mainnet => "ckBTC",
        _ => "ckTESTBTC",
    };

    let kyt_fee = read_state(|s| s.kyt_fee);
    let mut utxo_statuses: Vec<UtxoStatus> = vec![];
    for utxo in new_utxos {
        if utxo.value <= kyt_fee {
            mutate_state(|s| crate::state::audit::ignore_utxo(s, utxo.clone()));
            log!(
                P1,
                "Ignored UTXO {} for account {caller_account} because UTXO value {} is lower than the KYT fee {}",
                DisplayOutpoint(&utxo.outpoint),
                DisplayAmount(utxo.value),
                DisplayAmount(kyt_fee),
            );
            utxo_statuses.push(UtxoStatus::ValueTooSmall(utxo));
            continue;
        }

        if utxo.value == 546 {
            mutate_state(|s| crate::state::audit::ignore_utxo(s, utxo.clone()));
            utxo_statuses.push(UtxoStatus::TransferInscription(utxo));
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

        // @review (mint) consider using satoshis_to_mint
        match mint(amount, caller_account, /*crate::memo::encode(&memo).into()*/).await {
            Ok(block_index) => {
                log!(
                    P1,
                    "Minted {amount} {token_name} for account {caller_account} corresponding to utxo {} with value {}",
                    DisplayOutpoint(&utxo.outpoint),
                    DisplayAmount(utxo.value),
                );
                state::mutate_state(|s| {
                    state::audit::add_utxos(
                        s,
                        Some(block_index[0]),
                        caller_account,
                        vec![utxo.clone()],
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
                    "Failed to mint ckBTC for UTXO {}: {:?}",
                    DisplayOutpoint(&utxo.outpoint),
                    err
                );
                utxo_statuses.push(UtxoStatus::Checked(utxo));
            }
        }
    }

    schedule_now(TaskType::ProcessLogic);
    
    // let res = match mint(satoshis_to_mint, caller_account).await {
    //     Ok(res) => Ok(utxo_statuses),
    //     Err(res) => Err(res)
    // };
    // return res
    
    Ok(utxo_statuses)
}

async fn kyt_check_utxo(
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

/// Mint an amount of SU$D to an Account & Lock the BTC collateral
pub(crate) async fn mint(satoshis: u64, to: Account, /*memo: Memo,*/) -> Result<Vec<u64 /*UtxoStatus*/>, UpdateBalanceError> {
    // debug_assert!(memo.0.len() <= crate::LEDGER_MEMO_SIZE as usize);
    let client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.ledger_id.get().into()),
    };

    let block_index = client
        .transfer(TransferArg {
            from_subaccount: None,
            to,
            fee: None,
            created_at_time: None,
            memo: None,//Some(memo),
            amount: Nat::from(satoshis),
        })
        .await
        .map_err(|(code, msg)| {
            UpdateBalanceError::TemporarilyUnavailable(format!(
                "cannot mint ckbtc: {} (reject_code = {})",
                msg, code
            ))
        })??;

    // @dev (susd)
    let susd_client = ICRC1Client {
        runtime: CdkRuntime,
        ledger_canister_id: state::read_state(|s| s.susd_id.get().into()),
    };
    
    // @xrc
    let xr = get_exchange_rate().await?.unwrap();
    let susd: u64 = satoshis * xr.rate / 1_000_000_000 / 15 * 10; //@review (xrc) over-collateralization ratio (1.5)

    let block_index_susd = susd_client
        .transfer(TransferArg {
            from_subaccount: None,
            to,
            fee: None,
            created_at_time: None,
            memo: None,
            amount: Nat::from(susd),
        })
        .await
        .map_err(|(code, msg)| {
            UpdateBalanceError::TemporarilyUnavailable(format!(
                "cannot mint su$d: {} (reject_code = {})",
                msg, code
            ))
        })??;

    // return Err(
    //     UpdateBalanceError::TemporarilyUnavailable(format!(
    //         "satoshis: {}, xc: {}, su$d: {}",
    //         satoshis, xr.rate, susd
    //     ))
    // );
    
    log!(
        P0,
        "Minted {susd} SU$D with {satoshis} BTC for account {to} at XR: {}",
        DisplayAmount(xr.rate),
    );

    let res = [block_index.0.to_u64().expect("nat does not fit into u64"), block_index_susd.0.to_u64().expect("nat does not fit into u64")];
    Ok(res.to_vec())
}
