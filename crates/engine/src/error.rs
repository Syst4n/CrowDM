use std::time::Duration;

use crate::model::{ErrorKind, JobError};

/// Error raised while transferring. Converted to [`JobError`] for storage.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct TransferError {
    pub kind: ErrorKind,
    pub message: String,
    /// Server-requested delay before retrying.
    pub retry_after: Option<Duration>,
    pub status: Option<u16>,
}

impl TransferError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        TransferError { kind, message: message.into(), retry_after: None, status: None }
    }

    pub fn io(context: &str, e: std::io::Error) -> Self {
        // ERROR_DISK_FULL (112) / ERROR_HANDLE_DISK_FULL (39) / ENOSPC (28)
        let disk_full = matches!(e.raw_os_error(), Some(112) | Some(39)) || (cfg!(unix) && e.raw_os_error() == Some(28));
        if disk_full || e.kind() == std::io::ErrorKind::StorageFull {
            return TransferError::new(ErrorKind::DiskFull, "The destination drive is full.");
        }
        TransferError::new(ErrorKind::Io, format!("{context}: {e}"))
    }

    pub fn changed(detail: &str) -> Self {
        TransferError::new(
            ErrorKind::ResourceChanged,
            format!("The file changed on the server since this download started ({detail}). Restart the download to get the new version."),
        )
    }

    pub fn to_job_error(&self) -> JobError {
        JobError { kind: self.kind, message: self.message.clone() }
    }
}

/// Errors from engine API calls.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("download not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Transfer(#[from] TransferError),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = EngineError> = std::result::Result<T, E>;
