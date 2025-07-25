use crate::updates::UpdateBalanceError;
use ic_btc_interface::Utxo;
use num_traits::ToPrimitive;
use ic_cdk::api::management_canister::http_request::{
    HttpHeader, HttpMethod, TransformContext, CanisterHttpRequestArgument, HttpResponse
};
use super:: types::{ServiceProvider, ResolvedServiceProvider, ServiceError, ServiceResult, HttpOutcallError};
use super::provider::resolve_service_provider;

pub async fn call_indexer_runes_balance(
    utxo: Utxo,
    cycles_cost: u128,
    provider: u64,
) -> Result<String, UpdateBalanceError> {
    let txid_bytes = utxo.outpoint.txid.as_ref().iter().rev().map(|n| *n as u8).collect::<Vec<u8>>();
    let txid = hex::encode(txid_bytes);
    let index = utxo.outpoint.vout.to_string();
    let endpoint = format!("get-unisat-runes-balance?txid={}&index={}", txid, index);
    let outcall = match web3_request(ServiceProvider::Provider(provider), &endpoint, "", 8192, cycles_cost).await { // @review (alpha) max_response_bytes
        Ok(result) => result,
        Err(err) => {
            return Err(UpdateBalanceError::GenericError{
                error_code: 1001,
                error_message: format!("HTTPS Outcall failed in call_indexer_runes_balance: {:?}", err),
            });
        }
    };
    Ok(outcall)
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
