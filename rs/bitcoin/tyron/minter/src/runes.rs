use crate::guard::balance_update_guard;
use crate::state;
use crate::management;
use crate::updates::get_withdrawal_account::compute_subaccount;
use crate::updates::UpdateBalanceError;
use crate::https::outcall::call_indexer_runes_balance;
use crate::Utxo;
use icrc_ledger_types::icrc1::account::Account;

/// Update runes minter balance
pub async fn check_runes_minter_utxos(mut minter_utxos: Vec<Utxo>) -> Result<(Vec<Utxo>, Vec<Utxo>), UpdateBalanceError> {
    // @dev iterate over the utxos and send each transaction id to the outcall
    let mut utxos1: Vec<Utxo> = Vec::new();
    let mut utxos2: Vec<Utxo> = Vec::new();

    for utxo in &mut minter_utxos {
        let amount_u64 = call_indexer_runes_balance(utxo.clone(), 72_000_000, 0).await?; // @dev review (alpha) cycles_cost and provider

        if amount_u64 == 0 {
            utxos1.push(utxo.clone());
        } else {
            utxo.value = amount_u64;
            utxos2.push(utxo.clone());
        }
    }

    return Ok((utxos1, utxos2));
}

pub async fn is_new_runes_minter_utxos() -> Result<Vec<Utxo>, UpdateBalanceError> {
    // @dev only check runes minter utxos if there are unregistered utxos to process
    let (treasury_addr, runes_minter, network, min_confirmations) = state::read_state(|s: &state::MinterState| (s.dao_addr[1].display(s.btc_network), s.dao_addr[2].display(s.btc_network), s.btc_network, s.min_confirmations));
    let runes_minter_account = Account {
        owner: ic_cdk::id(),
        subaccount: Some(compute_subaccount(1, &treasury_addr)),
    };
    
    state::read_state(|s| s.mode.is_deposit_available_for(&runes_minter_account))
        .map_err(UpdateBalanceError::TemporarilyUnavailable)?;
    
    // @dev guard minter runes account
    let _guard = balance_update_guard(runes_minter_account.clone())?;
    
    // @dev get utxos from bitcoin canister (query)
    // let utxos_response_query = match management::get_utxos_query(network, &runes_minter, min_confirmations).await {
    //     Ok(response) => response,
    //     Err(e) => {
    //         ic_cdk::println!("[ProcessLogic]: Failed to get Runes Minter UTXOs from Bitcoin Canister: {:?}", e);
    //         return Err(UpdateBalanceError::GenericError {
    //             error_code: 1001,
    //             error_message: format!("Failed to get UTXOs query: {:?}", e),
    //         });
    //     }
    // };
    
    // Check for new UTXOs using the existing state management
    // let new_utxos_query = state::read_state(|s| s.new_utxos_for_account(utxos_response_query.utxos, &runes_minter_account));
    
    // @dev if new utxos are found, get utxos from bitcoin canister
    // if !new_utxos_query.is_empty() {
        // @dev get verified utxos from bitcoin canister
        let utxos_response = match management::get_utxos(network, &runes_minter, min_confirmations, management::CallSource::Client).await {
            Ok(response) => response,
            Err(e) => {
                ic_cdk::println!("[ProcessLogic]: Failed to get Runes Minter UTXOs from Bitcoin Canister: {:?}", e);
                return Err(UpdateBalanceError::GenericError {
                    error_code: 1002,
                    error_message: format!("Failed to get UTXOs: {:?}", e),
                });
            }
        };
        let new_utxos = state::read_state(|s| s.new_utxos_for_account(utxos_response.utxos, &runes_minter_account));
        
        return Ok(new_utxos);
    // } else {
    //     // @dev if no new utxos are found, return an empty vector
    //     return Ok((vec![], runes_minter_account));
    // }
}

pub async fn check_runes_and_sats_utxos(mut utxos: Vec<Utxo>) -> Result<(Vec<Utxo>, Vec<Utxo>), UpdateBalanceError> {
    // @dev iterate over the utxos and send each transaction id to the outcall
    let mut utxos1: Vec<Utxo> = Vec::new();
    let mut utxos2: Vec<Utxo> = Vec::new();

    for utxo in &mut utxos {
        let amount_u64 = call_indexer_runes_balance(utxo.clone(), 72_000_000, 0).await?; // @dev review (alpha) cycles_cost and provider

        if amount_u64 == 0 {
            utxos1.push(utxo.clone());
        } else {
            utxos2.push(utxo.clone());
        }
    }

    return Ok((utxos1, utxos2));
}