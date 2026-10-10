//! Locating the `cms` binary that the tracking hooks must invoke.
//!
//! Order: the sidecar next to the running executable (bundled app, or `cms`
//! itself), then `~/.local/bin/cms` (dev installs). Pure candidate building
//! is separated from the filesystem probe.

use std::path::{Path, PathBuf};

fn binary_name(windows: bool) -> &'static str {
    if windows {
        "cms.exe"
    } else {
        "cms"
    }
}

/// Candidate locations, best first.
pub fn candidates(exe_dir: &Path, home: &Path, windows: bool) -> Vec<PathBuf> {
    let name = binary_name(windows);
    vec![
        exe_dir.join(name),
        home.join(".local").join("bin").join(name),
    ]
}

/// First candidate that is an existing file.
pub fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// Resolve the `cms` binary for the current process, or `None` (tracking hooks
/// cannot be installed; callers surface a warning, never crash).
pub fn resolve() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))?;
    let home = dirs::home_dir().unwrap_or_else(|| exe_dir.clone());
    first_existing(&candidates(&exe_dir, &home, cfg!(windows)))
}
