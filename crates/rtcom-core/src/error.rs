//! Error types for `rtcom-core`.
//!
//! The crate deliberately avoids [`anyhow`](https://docs.rs/anyhow) at library
//! boundaries — callers (including `rtcom-cli`) need to match on specific
//! failure domains to drive reconnection, user-visible diagnostics, and
//! exit-code selection.

use std::io;

use thiserror::Error;

/// Convenience alias for results returned by `rtcom-core` APIs.
pub type Result<T> = std::result::Result<T, Error>;

/// All fallible operations in `rtcom-core` funnel into this enum.
///
/// New variants may be added in minor releases; match with a trailing `_`
/// arm when you care about forward-compatibility.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// I/O error from the host OS, typically while reading from or writing to
    /// the serial device.
    #[error("serial I/O error: {0}")]
    Io(#[from] io::Error),

    /// Error reported by the underlying [`serialport`] / [`tokio_serial`]
    /// backend (for example, port not found, busy, or unsupported setting).
    #[error("serial backend error: {0}")]
    Backend(#[from] serialport::Error),

    /// The supplied [`SerialConfig`](crate::SerialConfig) value is invalid —
    /// e.g. a baud rate of zero.
    #[error("invalid serial configuration: {0}")]
    InvalidConfig(String),
}
