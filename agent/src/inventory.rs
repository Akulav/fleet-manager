//! Installed-software inventory, read from the Windows uninstall registry keys.

use fleetview_shared::protocol::SoftwareEntry;
use winreg::enums::*;
use winreg::RegKey;

pub fn installed_software() -> Vec<SoftwareEntry> {
    let mut out = Vec::new();
    let roots: [(isize, &str); 3] = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    for (hive, path) in roots {
        let root = RegKey::predef(hive);
        let Ok(uninstall) = root.open_subkey_with_flags(path, KEY_READ) else {
            continue;
        };
        for name in uninstall.enum_keys().flatten() {
            let Ok(sub) = uninstall.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            // Skip system components and updates.
            let system_component: u32 = sub.get_value("SystemComponent").unwrap_or(0);
            if system_component == 1 {
                continue;
            }
            let display_name: String = match sub.get_value("DisplayName") {
                Ok(n) => n,
                Err(_) => continue,
            };
            if display_name.trim().is_empty() {
                continue;
            }
            out.push(SoftwareEntry {
                name: display_name,
                version: sub.get_value("DisplayVersion").ok(),
                publisher: sub.get_value("Publisher").ok(),
                uninstall_string: sub.get_value("UninstallString").ok(),
            });
        }
    }

    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| a.name == b.name && a.version == b.version);
    out
}
