//! Default filesystem locations for rtcom profiles.

use std::path::PathBuf;

use directories::ProjectDirs;

/// Returns the default path at which rtcom reads / writes its profile.
///
/// Resolves to the platform config directory via XDG / Apple / Known Folder
/// conventions, e.g. `$XDG_CONFIG_HOME/rtcom/default.toml` on Linux.
/// Returns `None` when no home directory can be determined (unusual — only
/// hits sandboxed environments without `HOME`/`USERPROFILE`).
#[must_use]
pub fn default_profile_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rtcom").map(|dirs| dirs.config_dir().join("default.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_path_has_rtcom_and_default_toml() {
        // env may lack HOME/XDG_CONFIG_HOME in CI sandboxes; tolerate None.
        if let Some(p) = default_profile_path() {
            assert!(p.ends_with("default.toml"), "tail: {}", p.display());
            let as_str = p.to_string_lossy();
            assert!(
                as_str.contains("rtcom"),
                "path must include rtcom: {as_str}"
            );
        }
    }
}
