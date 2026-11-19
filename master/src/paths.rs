//! Master data locations, under `%APPDATA%\FleetView` (or `FLEETVIEW_DATA_DIR`
//! when set, which demo mode and tests use to stay out of the real data).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FLEETVIEW_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("FleetView")
}

pub fn ensure_data_dir() -> std::io::Result<PathBuf> {
    let d = data_dir();
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

pub fn settings_path() -> PathBuf {
    data_dir().join("master.settings.json")
}

pub fn known_agents_path() -> PathBuf {
    data_dir().join("known_agents.json")
}

/// UI preferences, alert thresholds and saved script snippets.
pub fn prefs_path() -> PathBuf {
    data_dir().join("ui.prefs.json")
}
