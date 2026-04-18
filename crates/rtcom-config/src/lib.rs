//! Profile persistence for rtcom.
#![forbid(unsafe_code)]

pub mod paths;
pub mod profile;

use std::path::Path;

pub use paths::default_profile_path;
pub use profile::{ModalStyle, Profile};

/// Errors produced by profile IO.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying filesystem error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse error.
    #[error("parse error: {0}")]
    Parse(#[from] toml::de::Error),
    /// TOML serialize error.
    #[error("serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Read a profile from a TOML file at `path`.
///
/// # Errors
///
/// Returns [`Error::Io`] if the file cannot be opened (including
/// `NotFound`) and [`Error::Parse`] if the TOML is malformed.
pub fn read(path: &Path) -> Result<Profile, Error> {
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

/// Serialize `profile` to TOML and write it to `path`, creating any missing
/// parent directories.
///
/// # Errors
///
/// Returns [`Error::Io`] on filesystem failures and [`Error::Serialize`] if
/// the profile cannot be serialized (currently unreachable, but kept for
/// schema additions that may later hit serde's serialize path).
pub fn write(path: &Path, profile: &Profile) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(profile)?;
    std::fs::write(path, text)?;
    Ok(())
}
