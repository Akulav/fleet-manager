//! Master data locations, under `%APPDATA%\FleetView`.

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
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
