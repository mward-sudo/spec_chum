use serde::Serialize;
use spec_chum_host::HostError;
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
    #[error(transparent)]
    Host(#[from] HostError),
}

impl ApiError {
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Unauthorized => 401,
            Self::BadRequest(_) => 400,
            Self::NoMachine => 409,
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
