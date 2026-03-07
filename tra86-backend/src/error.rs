use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("process not running")]
    ProcessNotRunning,
    #[error("not attached")]
    NotAttached,
    #[error("io error: {0}")]
    Io(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("timeout while waiting for backend")]
    Timeout,
    #[error("internal backend error: {0}")]
    Internal(String),
}
