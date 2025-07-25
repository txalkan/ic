use candid::{CandidType, Decode, Deserialize, Encode, Principal};
use serde::Serialize;
use ic_cdk::api::management_canister::http_request::HttpHeader;
use ic_stable_structures::{Storable, storable::Bound};
use std::borrow::Cow;
use ic_cdk::api::call::RejectionCode;
use thiserror::Error;

// @dev Principal storable

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct PrincipalStorable(pub Principal);

impl Storable for PrincipalStorable {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::from(self.0.as_slice())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Self(Principal::from_slice(&bytes))
    }
    const BOUND: Bound = Bound::Bounded {
        max_size: 29,
        is_fixed_size: false,
    };
}

// @dev Service provider

#[derive(Clone, Debug, CandidType, Deserialize)]
pub struct Metadata {
    pub next_provider_id: u64,
    pub open_rpc_access: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            next_provider_id: 0,
            open_rpc_access: true,
        }
    }
}

impl Storable for Metadata {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }
    const BOUND: Bound = Bound::Bounded {
        max_size: 1024,
        is_fixed_size: false,
    };
}

#[derive(Clone, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, CandidType)]
pub struct ProviderApi {
    pub url: String,
    pub headers: Option<Vec<HttpHeader>>,
}

#[derive(Clone, CandidType, Deserialize)]
pub struct Provider {
    #[serde(rename = "providerId")]
    pub provider_id: u64,
    pub owner: Principal,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub hostname: String,
    #[serde(rename = "credentialPath")]
    pub credential_path: String,
    #[serde(rename = "credentialHeaders")]
    pub credential_headers: Vec<HttpHeader>,
    #[serde(rename = "cyclesPerCall")]
    pub cycles_per_call: u64,
    #[serde(rename = "cyclesPerMessageByte")]
    pub cycles_per_message_byte: u64,
    #[serde(rename = "cyclesOwed")]
    pub cycles_owed: u128,
    pub primary: bool,
}

impl Provider {
    pub fn api(&self) -> ProviderApi {
        ProviderApi {
            url: format!("https://{}{}", self.hostname, self.credential_path),
            headers: if self.credential_headers.is_empty() {
                None
            } else {
                Some(self.credential_headers.clone())
            },
        }
    }
}

impl Storable for Provider {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(&bytes, Self).unwrap()
    }

    const BOUND: Bound = Bound::Bounded {
        max_size: 1024,
        is_fixed_size: false,
    };
}

#[derive(Clone, CandidType, Deserialize)]
pub struct RegisterProviderArgs {
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub hostname: String,
    #[serde(rename = "credentialPath")]
    pub credential_path: String,
    #[serde(rename = "credentialHeaders")]
    pub credential_headers: Option<Vec<HttpHeader>>,
    #[serde(rename = "cyclesPerCall")]
    pub cycles_per_call: u64,
    #[serde(rename = "cyclesPerMessageByte")]
    pub cycles_per_message_byte: u64,
}

#[derive(Clone, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, CandidType)]
pub enum ServiceProvider {
    Chain(u64),
    Provider(u64),
}

impl std::fmt::Debug for ServiceProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceProvider::Chain(chain_id) => write!(f, "Chain({})", chain_id),
            ServiceProvider::Provider(provider_id) => write!(f, "Provider({})", provider_id),
        }
    }
}

// @dev Storable service provider

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct StorableServiceProvider(Vec<u8>);

impl TryFrom<StorableServiceProvider> for ServiceProvider {
    type Error = serde_json::Error;
    fn try_from(value: StorableServiceProvider) -> Result<Self, Self::Error> {
        serde_json::from_slice(&value.0)
    }
}

impl StorableServiceProvider {
    pub fn new(service: &ServiceProvider) -> Self {
        // Store as JSON string to remove the possibility of RPC services getting mixed up
        // if we make changes to `RpcService`, `EthMainnetService`, etc.
        Self(
            serde_json::to_vec(service)
                .expect("BUG: unexpected error while serializing RpcService"),
        )
    }
}

impl Storable for StorableServiceProvider {
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        StorableServiceProvider(bytes.to_vec())
    }

    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(self.0.to_owned())
    }

    const BOUND: Bound = Bound::Bounded {
        max_size: 1024,
        is_fixed_size: false,
    };
}

// @dev Resolved provider

pub enum ResolvedServiceProvider {
    Provider(Provider),
}

impl ResolvedServiceProvider {
    pub fn api(&self) -> ProviderApi {
        match self {
            Self::Provider(provider) => provider.api(),
        }
    }
}

// @dev Provider errors

#[derive(Clone, Debug, PartialEq, Eq, CandidType, Deserialize)]
pub enum ProviderError {
    // #[error("no permission")]
    NoPermission,
    // #[error("too few cycles (expected {expected}, received {received})")]
    TooFewCycles { expected: u128, received: u128 },
    // #[error("provider not found")]
    ProviderNotFound,
    // #[error("missing required provider")]
    MissingRequiredProvider,
}

#[derive(Clone, Hash, Debug, PartialEq, Eq, PartialOrd, Ord, CandidType, Deserialize)]
pub enum ValidationError {
    // #[error("{0}")]
    Custom(String),
    // #[error("invalid hex data: {0}")]
    InvalidHex(String),
    // #[error("URL parse error: {0}")]
    UrlParseError(String),
    // #[error("hostname not allowed: {0}")]
    HostNotAllowed(String),
    // #[error("credential path not allowed")]
    CredentialPathNotAllowed,
    // #[error("credential header not allowed")]
    CredentialHeaderNotAllowed,
}

#[derive(
    Clone, Hash, Debug, PartialEq, Eq, PartialOrd, Ord, CandidType, Serialize, Deserialize, Error,
)]
#[error("code {code}: {message}")]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, CandidType, Deserialize)]
pub enum ServiceError {
    // #[error("Service provider error")]
    ProviderError(/* #[source] */ ProviderError),
    // #[error("HTTPS outcall error")]
    HttpOutcallError(/* #[source] */ HttpOutcallError),
    // #[error("JSON-RPC error")]
    JsonRpcError(/* #[source] */ JsonRpcError),
    // #[error("data format error")]
    ValidationError(/* #[source] */ ValidationError),
}

impl From<ProviderError> for ServiceError {
    fn from(err: ProviderError) -> Self {
        ServiceError::ProviderError(err)
    }
}

impl From<HttpOutcallError> for ServiceError {
    fn from(err: HttpOutcallError) -> Self {
        ServiceError::HttpOutcallError(err)
    }
}

impl From<JsonRpcError> for ServiceError {
    fn from(err: JsonRpcError) -> Self {
        ServiceError::JsonRpcError(err)
    }
}

impl From<ValidationError> for ServiceError {
    fn from(err: ValidationError) -> Self {
        ServiceError::ValidationError(err)
    }
}

#[derive(Clone, Hash, Debug, PartialEq, Eq, PartialOrd, Ord, CandidType, Deserialize)]
pub enum HttpOutcallError {
    /// Error from the IC system API.
    // #[error("IC system error code {}: {message}", *.code as i32)]
    IcError {
        code: RejectionCode,
        message: String,
    },
    /// Response is not a valid JSON-RPC response,
    /// which means that the response was not successful (status other than 2xx)
    /// or that the response body could not be deserialized into a JSON-RPC response.
    // #[error("invalid JSON-RPC response {status}: {})", .parsing_error.as_deref().unwrap_or(.body))]
    InvalidHttpJsonRpcResponse {
        status: u16,
        body: String,
        #[serde(rename = "parsingError")]
        parsing_error: Option<String>,
    },
}

pub fn is_response_too_large(code: &RejectionCode, message: &str) -> bool {
    code == &RejectionCode::SysFatal && message.contains("size limit")
}

impl HttpOutcallError {
    pub fn is_response_too_large(&self) -> bool {
        match self {
            Self::IcError { code, message } => is_response_too_large(code, message),
            _ => false,
        }
    }
}

pub type ServiceResult<T> = Result<T, ServiceError>;
