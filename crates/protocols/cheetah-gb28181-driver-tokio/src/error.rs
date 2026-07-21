//! Driver errors.

use cheetah_gb28181_core::SipError;
use std::io;

/// Errors returned by the UDP driver.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// An underlying I/O operation failed.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    /// A SIP datagram could not be parsed.
    #[error("SIP parse error: {0}")]
    Parse(#[from] SipError),
    /// The access state machine rejected processing.
    #[error("access error: {0}")]
    Access(Box<dyn std::error::Error + Send + Sync + 'static>),
    /// Binding the UDP socket failed.
    #[error("failed to bind socket: {0}")]
    Bind(io::Error),
    /// The access state lock was poisoned.
    #[error("access lock poisoned")]
    AccessLock,
}
