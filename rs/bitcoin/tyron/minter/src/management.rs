//! This module contains async functions for interacting with the management canister.

use crate::logs::P0;
use crate::tx;
use crate::ECDSAPublicKey;
use crate::state::read_state;
use candid::{CandidType, Principal};
use ic_btc_interface::{
    Address, GetCurrentFeePercentilesRequest, GetUtxosRequest, GetUtxosResponse,
    MillisatoshiPerByte, Network, Utxo, UtxosFilterInRequest,
};
use ic_canister_log::log;
use ic_cdk::api::call::RejectionCode;
use ic_ckbtc_kyt::{DepositRequest, Error as KytError, FetchAlertsResponse, WithdrawalAttempt};
use ic_management_canister_types::{
    DerivationPath, ECDSAPublicKeyArgs, ECDSAPublicKeyResponse, EcdsaCurve, EcdsaKeyId,
    SignWithECDSAArgs, SignWithECDSAReply,
};
use serde::de::DeserializeOwned;
use std::fmt;
use ic_xrc_types::{Asset, AssetClass, GetExchangeRateRequest, GetExchangeRateResult, ExchangeRateError};
use serde_bytes::ByteBuf;

/// Represents an error from a management canister call, such as
/// `sign_with_ecdsa` or `bitcoin_send_transaction`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallError {
    method: String,
    reason: Reason,
}

impl CallError {
    /// Returns the name of the method that resulted in this error.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the failure reason.
    pub fn reason(&self) -> &Reason {
        &self.reason
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            fmt,
            "management call '{}' failed: {}",
            self.method, self.reason
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The reason for the management call failure.
pub enum Reason {
    /// Failed to send a signature request because the local output queue is
    /// full.
    QueueIsFull,
    /// The canister does not have enough cycles to submit the request.
    OutOfCycles,
    /// The call failed with an error.
    CanisterError(String),
    /// The management canister rejected the signature request (not enough
    /// cycles, the ECDSA subnet is overloaded, etc.).
    Rejected(String),
    Other(String)
}

impl fmt::Display for Reason {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueIsFull => write!(fmt, "the canister queue is full"),
            Self::OutOfCycles => write!(fmt, "the canister is out of cycles"),
            Self::CanisterError(msg) => write!(fmt, "canister error: {}", msg),
            Self::Rejected(msg) => {
                write!(fmt, "the management canister rejected the call: {}", msg)
            },
            Self::Other(msg) => {
                write!(fmt, " call rejected - reason: {}", msg)
            }

        }
    }
}

impl Reason {
    fn from_reject(reject_code: RejectionCode, reject_message: String) -> Self {
        match reject_code {
            RejectionCode::SysTransient => Self::QueueIsFull,
            RejectionCode::CanisterError => Self::CanisterError(reject_message),
            RejectionCode::CanisterReject => Self::Rejected(reject_message),
            _ => Self::Other(reject_message),
        }
    }

    fn to_string(&self) -> String {
        match self {
            Self::QueueIsFull => "the canister queue is full".to_string(),
            Self::OutOfCycles => "the canister is out of cycles".to_string(),
            Self::CanisterError(msg) => format!("canister error: {}", msg),
            Self::Rejected(msg) => {
                format!("the management canister rejected the call: {}", msg)
            },
            Self::Other(msg) => {
                format!(" call rejected - reason: {}", msg)
            }
        }
    }
}

async fn call<I, O>(method: &str, payment: u64, input: &I) -> Result<O, CallError>
where
    I: CandidType,
    O: CandidType + DeserializeOwned,
{
    let balance = ic_cdk::api::canister_balance128();
    if balance < payment as u128 {
        log!(
            P0,
            "Failed to call {}: need {} cycles, the balance is only {}",
            method,
            payment,
            balance
        );

        return Err(CallError {
            method: method.to_string(),
            reason: Reason::OutOfCycles,
        });
    }

    let res: Result<(O,), _> = ic_cdk::api::call::call_with_payment(
        Principal::management_canister(),
        method,
        (input,),
        payment,
    )
    .await;

    match res {
        Ok((output,)) => Ok(output),
        Err((code, msg)) => Err(CallError {
            method: method.to_string(),
            reason: Reason::from_reject(code, msg),
        }),
    }
}

#[derive(Clone, Copy)]
pub enum CallSource {
    /// The client initiated the call.
    Client,
    /// The minter initiated the call for internal bookkeeping.
    Minter,
}

/// Fetches the full list of UTXOs for the specified address.
pub async fn get_utxos(
    network: Network,
    address: &Address,
    min_confirmations: u32,
    source: CallSource,
) -> Result<GetUtxosResponse, CallError> {
    // NB. The minimum number of cycles that need to be sent with the call is 10B (4B) for
    // Bitcoin mainnet (Bitcoin testnet):
    // https://internetcomputer.org/docs/current/developer-docs/integrations/bitcoin/bitcoin-how-it-works#api-fees--pricing
    let get_utxos_cost_cycles = match network {
        Network::Mainnet => 10_000_000_000,
        Network::Testnet | Network::Regtest => 4_000_000_000,
    };

    // Calls "bitcoin_get_utxos" method with the specified argument on the
    // management canister.
    async fn bitcoin_get_utxos(
        req: &GetUtxosRequest,
        cycles: u64,
        source: CallSource,
    ) -> Result<GetUtxosResponse, CallError> {
        match source {
            CallSource::Client => &crate::metrics::GET_UTXOS_CLIENT_CALLS,
            CallSource::Minter => &crate::metrics::GET_UTXOS_MINTER_CALLS,
        }
        .with(|cell| cell.set(cell.get() + 1));
        call("bitcoin_get_utxos", cycles, req).await
    }

    let mut response = bitcoin_get_utxos(
        &GetUtxosRequest {
            address: address.to_string(),
            network: network.into(),
            filter: Some(UtxosFilterInRequest::MinConfirmations(min_confirmations)),
        },
        get_utxos_cost_cycles,
        source,
    )
    .await?;

    let mut utxos = std::mem::take(&mut response.utxos);

    // Continue fetching until there are no more pages.
    while let Some(page) = response.next_page {
        response = bitcoin_get_utxos(
            &GetUtxosRequest {
                address: address.to_string(),
                network: network.into(),
                filter: Some(UtxosFilterInRequest::Page(page)),
            },
            get_utxos_cost_cycles,
            source,
        )
        .await?;

        utxos.append(&mut response.utxos);
    }

    response.utxos = utxos;

    Ok(response)
}

/// Returns the current fee percentiles on the bitcoin network.
pub async fn get_current_fees(network: Network) -> Result<Vec<MillisatoshiPerByte>, CallError> {
    let cost_cycles = match network {
        Network::Mainnet => 100_000_000,
        Network::Testnet => 40_000_000,
        Network::Regtest => 0,
    };

    call(
        "bitcoin_get_current_fee_percentiles",
        cost_cycles,
        &GetCurrentFeePercentilesRequest {
            network: network.into(),
        },
    )
    .await
}

/// Sends the transaction to the network the management canister interacts with.
pub async fn send_transaction(
    transaction: &tx::SignedTransaction,
    network: Network,
) -> Result<(), CallError> {
    use ic_cdk::api::management_canister::bitcoin::BitcoinNetwork;

    let cdk_network = match network {
        Network::Mainnet => BitcoinNetwork::Mainnet,
        Network::Testnet => BitcoinNetwork::Testnet,
        Network::Regtest => BitcoinNetwork::Regtest,
    };

    let tx_bytes = transaction.serialize();

    ic_cdk::api::management_canister::bitcoin::bitcoin_send_transaction(
        ic_cdk::api::management_canister::bitcoin::SendTransactionRequest {
            transaction: tx_bytes,
            network: cdk_network,
        },
    )
    .await
    .map_err(|(code, msg)| CallError {
        method: "bitcoin_send_transaction".to_string(),
        reason: Reason::from_reject(code, msg),
    })
}

/// Fetches the ECDSA public key of the canister.
pub async fn ecdsa_public_key(
    key_name: String,
    derivation_path: DerivationPath,
) -> Result<ECDSAPublicKey, CallError> {
    // Retrieve the public key of this canister at the given derivation path
    // from the ECDSA API.
    call(
        "ecdsa_public_key",
        /*payment=*/ 0,
        &ECDSAPublicKeyArgs {
            canister_id: None,
            derivation_path,
            key_id: EcdsaKeyId {
                curve: EcdsaCurve::Secp256k1,
                name: key_name,
            },
        },
    )
    .await
    .map(|response: ECDSAPublicKeyResponse| ECDSAPublicKey {
        public_key: response.public_key,
        chain_code: response.chain_code,
    })
}

/// Signs a message hash using the tECDSA API.
pub async fn sign_with_ecdsa(
    key_name: String,
    derivation_path: DerivationPath,
    message_hash: [u8; 32],
) -> Result<Vec<u8>, CallError> {
    const CYCLES_PER_SIGNATURE: u64 = 30_000_000_000;

    let reply: SignWithECDSAReply = call(
        "sign_with_ecdsa",
        CYCLES_PER_SIGNATURE,
        &SignWithECDSAArgs {
            message_hash,
            derivation_path,
            key_id: EcdsaKeyId {
                curve: EcdsaCurve::Secp256k1,
                name: key_name.clone(),
            },
        },
    )
    .await?;
    Ok(reply.signature)
}

/// Requests alerts for the given UTXO.
pub async fn fetch_utxo_alerts(
    kyt_principal: Principal,
    caller: Principal,
    utxo: &Utxo,
) -> Result<Result<FetchAlertsResponse, KytError>, CallError> {
    let (res,): (Result<FetchAlertsResponse, KytError>,) = ic_cdk::api::call::call(
        kyt_principal,
        "fetch_utxo_alerts",
        (DepositRequest {
            caller,
            txid: utxo.outpoint.txid.into(),
            vout: utxo.outpoint.vout,
        },),
    )
    .await
    .map_err(|(code, message)| CallError {
        method: "fetch_utxo_alerts".to_string(),
        reason: Reason::from_reject(code, message),
    })?;
    Ok(res)
}

/// Requests alerts for the given Bitcoin address.
pub async fn fetch_withdrawal_alerts(
    kyt_principal: Principal,
    caller: Principal,
    address: String,
    amount: u64,
) -> Result<Result<FetchAlertsResponse, KytError>, CallError> {
    let now = ic_cdk::api::time();
    let id = format!("{caller}:{address}:{amount}:{now}");
    let (res,): (Result<FetchAlertsResponse, KytError>,) = ic_cdk::api::call::call(
        kyt_principal,
        "fetch_withdrawal_alerts",
        (WithdrawalAttempt {
            caller,
            id,
            amount,
            address,
            timestamp_nanos: now,
        },),
    )
    .await
    .map_err(|(code, message)| CallError {
        method: "fetch_withdrawal_alerts".to_string(),
        reason: Reason::from_reject(code, message),
    })?;
    Ok(res)
}

pub async fn fetch_btc_exchange_rate(symbol: String) -> Result<GetExchangeRateResult, CallError> {
    let btc = Asset {
        symbol: "BTC".to_string(),
        class: AssetClass::Cryptocurrency,
    };
    let quote_asset = Asset {
        symbol,
        class: AssetClass::FiatCurrency,
    };

    let request = GetExchangeRateRequest {
        base_asset: btc,
        quote_asset,
        timestamp: None,
    };

    let method = "get_exchange_rate";
    // let (res,): (GetExchangeRateResult,) = ic_cdk::api::call::call(
    //     read_state(|s| s.xrc_id.get().into()),
    //     method,
    //     (input,)
    // )
    // .await
    // .map_err(|(code, msg)| CallError {
    //         method: method.to_string(),
    //         reason: Reason::from_reject(code, msg),
    //     })?;
    // Ok(res);

    let payment = 10_000_000_000;

    let balance = ic_cdk::api::canister_balance128();
    
    if balance < payment as u128 {
        log!(
            P0,
            "Failed to call {}: need {} cycles, the balance is only {}",
            method,
            payment,
            balance
        );

        return Err(CallError {
            method: method.to_string(),
            reason: Reason::OutOfCycles,
        });
    }

    let res: Result<(GetExchangeRateResult,), _> = ic_cdk::api::call::call_with_payment(
        read_state(|s| s.xrc_id.get().into()),
        method,
        (request,),
        payment,
    )
    .await;

    match res {
        Ok((output,)) => Ok(output),
        Err((code, msg)) => Err(CallError {
            method: method.to_string(),
            reason: Reason::from_reject(code, msg),
        }),
    }
}

pub(crate) async fn get_siwb_principal(ssi: &str) -> Result<Principal, CallError> {
    let caller = ic_cdk::caller();

    let method = "get_principal";

    let res: Result<(Result<ByteBuf, String>,), _> = ic_cdk::api::call::call(
        read_state(|s| s.siwb_id.get().into()),
        method,
        (ssi,),
    )
    .await;

    match res {
        Ok((output,)) => {
            match output {
                Ok(byte_buf) => {
                    match Principal::try_from_slice(&byte_buf) {
                        Ok(principal) => {
                            // @dev the principal must be equal to the caller or throw an error
                            if principal == caller {
                                Ok(principal)
                            } else {
                                Err(CallError {
                                    method: method.to_string(),
                                    reason: Reason::Other("Invalid caller".to_string()),
                                })
                            }
                        }
                        Err(e) => {
                            let err = format!("Failed to decode principal with error: {:?}", e); 
                            ic_cdk::println!("{}", &err);
                            Err(CallError {
                                method: method.to_string(),
                                reason: Reason::Other(err),
                            })
                        }
                    }
                }
                Err(e) => {
                    let err = format!("The call returned an error: {:?}", e);
                    ic_cdk::println!("{}", &err);
                    Err(CallError {
                        method: method.to_string(),
                        reason: Reason::Other(err),
                    })                    
                }
            }
        },
        Err((code, msg)) => Err(CallError {
            method: method.to_string(),
            reason: Reason::from_reject(code, msg),
        }),
    }
}
