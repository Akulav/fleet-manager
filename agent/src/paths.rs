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
/// service then runs as SYSTEM. Setup does this too; repeating it at service
/// start makes it robust against manual installs and ACL drift.
///
/// Only the folder itself gets explicit entries; everything inside is reset to
/// inherit them. (Applying `/inheritance:r /grant:r …(OI)(CI)F` recursively
/// with `/T` — as 0.2.0 and earlier did — strips each *file's* inherited
/// entries and then can't apply the folder-only `(OI)(CI)` grant, leaving files
/// with an empty ACL that not even SYSTEM can read: the service then can't
/// load its config and never starts the tray helper. The reset below also
/// repairs files damaged that way.)
pub fn lock_down_data_dir() {
    let dir = data_dir();
    icacls(
        &dir.as_os_str().to_string_lossy(),
        &[
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",     // NT AUTHORITY\SYSTEM
            "*S-1-5-32-544:(OI)(CI)F", // BUILTIN\Administrators
            "/C",
            "/Q",
        ],
    );
    // Children: drop explicit / broken ACLs and inherit from the folder.
    icacls(
        &dir.join("*").as_os_str().to_string_lossy(),
        &["/reset", "/T", "/C", "/Q"],
    );
}

fn icacls(target: &str, args: &[&str]) {
    let result = std::process::Command::new("icacls.exe")
        .arg(target)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match result {
        Ok(st) if st.success() => {}
        // Exit code 1 with /C just means "some file was skipped" (e.g. empty
        // folder for the wildcard); worth a note, not an error.
        Ok(st) => tracing::warn!("icacls {target} exited with {st}"),
        Err(e) => tracing::warn!("cannot run icacls on {target}: {e}"),
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
