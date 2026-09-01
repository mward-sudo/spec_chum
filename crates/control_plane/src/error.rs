use serde::Serialize;
use spec_chum_host::{HostError, MachineConfigError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{0}")]
    Message(String),
    #[error("no machine loaded")]
    NoMachine,
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("png encode: {0}")]
    Png(String),
    /// Feature needs an attached host / is temporarily unavailable (#239).
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Host(HostError),
}

impl From<HostError> for ApiError {
    fn from(value: HostError) -> Self {
        match value {
            HostError::NoMachine => Self::NoMachine,
            other => Self::Host(other),
        }
    }
}

impl From<MachineConfigError> for ApiError {
    fn from(value: MachineConfigError) -> Self {
        Self::BadRequest(value.to_string())
    }
}

impl ApiError {
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Unauthorized => 401,
            Self::BadRequest(_) => 400,
            Self::NoMachine => 409,
            Self::Unavailable(_) => 503,
            Self::Message(_) | Self::Host(_) | Self::Png(_) => 500,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl From<ApiError> for ErrorBody {
    fn from(value: ApiError) -> Self {
        Self {
            error: value.to_string(),
        }
    }
}
