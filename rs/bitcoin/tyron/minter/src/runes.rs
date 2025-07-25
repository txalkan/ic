use crate::state;
use crate::management;
use crate::updates::UpdateBalanceError;
use crate::https::outcall::call_indexer_runes_balance;
use crate::Utxo;
use serde_json::Value;

/// Update runes minter balance
pub async fn check_runes_minter_utxos() -> Result<(Vec<Utxo>, Vec<Utxo>), UpdateBalanceError> {
    // @dev get minter utxos
    let (runes_minter, network, min_confirmations) = state::read_state(|s: &state::MinterState| (s.dao_addr[2].display(s.btc_network), s.btc_network, s.min_confirmations));
    let utxos_response = management::get_utxos(network, &runes_minter, min_confirmations, management::CallSource::Client).await?;
    let mut minter_utxos: Vec<Utxo> = utxos_response.utxos;

    // @dev iterate over the utxos and send each transaction id to the outcall

    let mut utxos1: Vec<Utxo> = Vec::new();
    let mut utxos2: Vec<Utxo> = Vec::new();
    
    for utxo in &mut minter_utxos {
        let outcall = call_indexer_runes_balance(utxo.clone(), 136_000_000, 0).await?; // @dev review (alpha) cycles_cost and provider
        ic_cdk::println!("runes minter utxo balance outcall ({:?}) for utxo ({:?})", outcall, utxo);

        let outcall_json: Value = match serde_json::from_str(&outcall) {
            Ok(json) => json,
            Err(e) => {
                ic_cdk::println!("Failed to parse runes balance response: {:?}, response: {:?}", e, outcall);
                return Err(UpdateBalanceError::CallError {
                    method: "check_runes_minter_utxos".to_string(),
                    reason: format!("Failed to parse runes balance response: {:?}, response: {:?}", e, outcall),
                })
            }
        };

        let amount_str = outcall_json["amount"].as_str().expect("amount should be a string");
        let amount_u64: u64 = amount_str.parse().expect("amount should be a valid u64");

        if amount_u64 == 0 {
            utxos1.push(utxo.clone());
        } else {
            utxo.value = amount_u64;
            utxos2.push(utxo.clone());
        }
    }

    return Ok((utxos1, utxos2));
}
