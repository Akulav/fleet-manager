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

/// Remove inherited ACEs from the data directory and grant only SYSTEM and
/// Administrators. `%ProgramData%` lets ordinary users read files and create
/// files/folders in subdirectories by default, which would expose the
/// enrollment secret and let a user tamper with staged installers that the
/// service then runs as SYSTEM. The installer does this too; repeating it at
/// service start makes it robust against manual installs and ACL drift.
pub fn lock_down_data_dir() {
    let dir = data_dir();
    let result = std::process::Command::new("icacls.exe")
        .arg(&dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",     // NT AUTHORITY\SYSTEM
            "*S-1-5-32-544:(OI)(CI)F", // BUILTIN\Administrators
            "/T",
            "/C",
            "/Q",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match result {
        Ok(st) if st.success() => {}
        Ok(st) => tracing::warn!("icacls on {} exited with {st}", dir.display()),
        Err(e) => tracing::warn!("cannot run icacls on {}: {e}", dir.display()),
    }
}

/// Stable per-machine agent id (kept separate from the installer-provided config).
pub fn agent_id_path() -> PathBuf {
    data_dir().join("agent.id")
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
