//! Well-known filesystem locations for the agent.
//!
//! Everything lives under `%ProgramData%\FleetView` so that the LocalSystem
//! service and the user-session helper can both reach it. The installer tightens
//! the ACL on this directory to SYSTEM + Administrators (see installer/agent.iss
//! and docs/SECURITY.md) because the config holds the enrollment secret.

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    // %ProgramData% is typically C:\ProgramData.
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("FleetView")
}

pub fn ensure_data_dir() -> std::io::Result<PathBuf> {
    let d = data_dir();
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

pub fn config_path() -> PathBuf {
    data_dir().join("agent.config.json")
}

pub fn log_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Written by the service, read by the helper: loopback IPC endpoint + token,
/// and current connection status for the tray tooltip.
pub fn status_path() -> PathBuf {
    data_dir().join("status.json")
}

/// Directory where pushed installers/files are staged before execution.
pub fn staging_dir() -> PathBuf {
    data_dir().join("staging")
}
