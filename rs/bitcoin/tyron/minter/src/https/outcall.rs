use crate::updates::UpdateBalanceError;
use ic_btc_interface::Utxo;
use num_traits::ToPrimitive;
use ic_cdk::api::management_canister::http_request::{
    HttpHeader, HttpMethod, TransformContext, CanisterHttpRequestArgument, HttpResponse
};
use super:: types::{ServiceProvider, ResolvedServiceProvider, ServiceError, ServiceResult, HttpOutcallError};
use super::provider::resolve_service_provider;
use serde_json::Value;

/// Extract Runes amount from parsed JSON with comprehensive validation
fn extract_runes_amount_from_json(outcall_json: Value) -> Result<u64, UpdateBalanceError> {
    // @dev get runes amount with proper error handling
    let amount_str = match outcall_json["amount"].as_str() {
        Some(amount) => amount,
        None => {
            ic_cdk::println!("Missing 'amount' field in outcall response: {:?}", outcall_json);
            return Err(UpdateBalanceError::CallError {
                method: "extract_runes_amount_from_json".to_string(),
                reason: "Missing 'amount' field in JSON response".to_string(),
            });
        }
    };
    
    // Check if the amount string contains commas or dots, which would indicate it's not in satoshis
    if amount_str.contains(',') || amount_str.contains('.') {
        return Err(UpdateBalanceError::CallError {
            method: "extract_runes_amount_from_json".to_string(),
            reason: format!("Amount '{}' contains commas or dots, indicating it's not in satoshis format", amount_str),
        });
    }

    let amount_u64: u64 = amount_str.parse().unwrap_or(0);

    Ok(amount_u64)
}

/// Get Runes balance for a specific UTXO with comprehensive error handling
pub async fn call_indexer_runes_balance(
    utxo: Utxo,
    cycles_cost: u128,
    provider: u64,
) -> Result<u64, UpdateBalanceError> {
    // @dev convert utxo outpoint to bitcoin transaction id and vout/index
    let txid_bytes = utxo.outpoint.txid.as_ref().iter().rev().map(|n| *n as u8).collect::<Vec<u8>>();
    let txid = hex::encode(txid_bytes);
    let index = utxo.outpoint.vout.to_string();

    // @dev build api endpoint url
    let endpoint = format!("get-unisat-runes-balance?txid={}&index={}", txid, index);

    // @dev execute https outcall @review (alpha) max_response_bytes, add var to state?
    let outcall = match web3_request(ServiceProvider::Provider(provider), &endpoint, "", 2048, cycles_cost).await {
        Ok(result) => result,
        Err(err) => {
            return Err(UpdateBalanceError::CallError {
                method: "call_indexer_runes_balance".to_string(),
                reason: format!("HTTPS Outcall failed with error: {:?}", err),
            });
        }
    };

    // @dev validate response is not HTML error page
    if outcall.trim_start().starts_with("<!DOCTYPE html>") {
        ic_cdk::println!("Received HTML error page for UTXO {}: {}", 
            format!("{}:{}", txid, index), outcall);
        return Err(UpdateBalanceError::CallError {
            method: "call_indexer_runes_balance".to_string(),
            reason: "Received HTML error page instead of JSON".to_string(),
        });
    }

    let outcall_json: Value = match serde_json::from_str(&outcall) {
        Ok(json) => json,
        Err(e) => {
            ic_cdk::println!("Failed to parse runes balance response with error: {:?}, for outcall response: {:?}", e, outcall);
            return Err(UpdateBalanceError::CallError {
                method: "check_runes_minter_utxos".to_string(),
                reason: format!("Failed to parse runes balance response: {:?}, response: {:?}", e, outcall),
            })
        }
    };

    ic_cdk::println!("runes balance outcall ({:?}) for utxo ({:?})", outcall_json, utxo);
    extract_runes_amount_from_json(outcall_json)
}

pub async fn web3_request(
    service: ServiceProvider,
    endpoint: &str,
    payload: &str,
    max_response_bytes: u64,
    cycles_cost: u128
) -> Result<String, ServiceError> {
    let response = do_request(
        resolve_service_provider(service)?,
        endpoint,
        payload,
        max_response_bytes,
        cycles_cost
    )
    .await?;
    get_http_response_body(response)
}

async fn do_request(
    service: ResolvedServiceProvider,
    endpoint: &str,
    payload: &str,
    max_response_bytes: u64,
    cycles_cost: u128
) -> ServiceResult<HttpResponse> {
    let api = service.api();
    let mut request_headers = vec![HttpHeader {
        name: "Content-Type".to_string(),
        value: "application/json".to_string(),
    }];
    if let Some(headers) = api.headers {
        request_headers.extend(headers);
    }

    let mut method = HttpMethod::GET;
    let mut body = None;
    if !payload.is_empty() {
        method = HttpMethod::POST;
        body = Some(payload.as_bytes().to_vec());
    }
    // Match service provider to the appropriate transform function
    let transform_fn: Option<TransformContext> = match service {
        ResolvedServiceProvider::Provider(provider) => {
            match provider.provider_id {
                0 | 1 => Some(TransformContext::from_name(
                    "transform_request".to_string(),
                    vec![],
                )),
                2 | 3 => Some(TransformContext::from_name(
                    "transform_unisat_request".to_string(),
                    vec![],
                )),
                id => {
                    // Log or handle unknown provider IDs
                    ic_cdk::println!("Warning: Unknown provider_id {} in transform selection", id);
                    None
                }
            }
        }
    };
    let request = CanisterHttpRequestArgument {
        url: api.url + endpoint,
        max_response_bytes: Some(max_response_bytes),
        method,
        headers: request_headers,
        body,
        transform: transform_fn,
    };
    match ic_cdk::api::management_canister::http_request::http_request(request, cycles_cost).await {
        Ok((response,)) => {
            Ok(response)
        }
        Err((code, message)) => {
            Err(HttpOutcallError::IcError{code, message}.into())
        }
    }
}

fn get_http_response_body(response: HttpResponse) -> Result<String, ServiceError> {
    String::from_utf8(response.body).map_err(|e| {
        HttpOutcallError::InvalidHttpJsonRpcResponse {
            status: get_http_response_status(response.status),
            body: "".to_string(),
            parsing_error: Some(format!("{e}")),
        }
        .into()
    })
}

pub fn get_http_response_status(status: candid::Nat) -> u16 {
    // If status.0 cannot be converted to u16, return u16::MAX (65535) as a fallback
    status.0.to_u16().unwrap_or(u16::MAX)
}
