//! UI preferences: theme, alert thresholds and saved PowerShell snippets.
//! Stored next to the master settings (never holds secrets).

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    pub name: String,
    pub script: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub dark: bool,
    /// Alert when a disk is fuller than this (%).
    pub disk_warn_pct: f32,
    /// Alert when memory use is above this (%).
    pub mem_warn_pct: f32,
    /// Alert when CPU use is above this (%).
    pub cpu_warn_pct: f32,
    /// Alert when a PC has been offline longer than this (days, 0 = never).
    pub stale_days: u32,
    pub snippets: Vec<Snippet>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            dark: true,
            disk_warn_pct: 90.0,
            mem_warn_pct: 90.0,
            cpu_warn_pct: 95.0,
            stale_days: 14,
            snippets: Vec::new(),
        }
    }
}

impl Prefs {
    pub fn load() -> Self {
        std::fs::read_to_string(crate::paths::prefs_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = crate::paths::ensure_data_dir();
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let path = crate::paths::prefs_path();
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

/// Ready-made scripts for common admin tasks.
pub fn builtin_snippets() -> Vec<Snippet> {
    let s = |name: &str, script: &str| Snippet {
        name: name.into(),
        script: script.trim().into(),
    };
    vec![
        s(
            "System summary",
            r#"
$os = Get-CimInstance Win32_OperatingSystem
$cs = Get-CimInstance Win32_ComputerSystem
$bios = Get-CimInstance Win32_BIOS
[pscustomobject]@{
  Model        = "$($cs.Manufacturer) $($cs.Model)"
  Serial       = $bios.SerialNumber
  OS           = "$($os.Caption) $($os.Version)"
  LastBoot     = $os.LastBootUpTime
  Domain       = $cs.Domain
} | Format-List
"#,
        ),
        s(
            "Pending reboot?",
            r#"
$keys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending',
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired'
$pending = $keys | Where-Object { Test-Path $_ }
if ($pending) { "Reboot pending" } else { "No reboot pending" }
"#,
        ),
        s(
            "Top 10 processes by memory",
            "Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 10 Name, Id, @{n='MB';e={[math]::Round($_.WorkingSet64/1MB)}} | Format-Table -AutoSize",
        ),
        s(
            "Stopped automatic services",
            "Get-CimInstance Win32_Service -Filter \"StartMode='Auto' AND State<>'Running'\" | Select-Object Name, DisplayName, State | Format-Table -AutoSize",
        ),
        s(
            "Local administrators",
            "Get-LocalGroupMember -SID S-1-5-32-544 | Select-Object Name, ObjectClass, PrincipalSource | Format-Table -AutoSize",
        ),
        s(
            "Recent Windows updates",
            "Get-HotFix | Sort-Object InstalledOn -Descending | Select-Object -First 15 HotFixID, Description, InstalledOn | Format-Table -AutoSize",
        ),
        s(
            "BitLocker status",
            "Get-BitLockerVolume | Select-Object MountPoint, VolumeStatus, ProtectionStatus, EncryptionPercentage | Format-Table -AutoSize",
        ),
        s(
            "Group Policy update",
            "gpupdate /force",
        ),
        s(
            "Flush DNS cache",
            "ipconfig /flushdns",
        ),
        s(
            "Clean temp folders",
            r#"
$before = (Get-PSDrive C).Free
Get-ChildItem "$env:windir\Temp", "C:\Users\*\AppData\Local\Temp" -Force -ErrorAction SilentlyContinue |
  Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
$freed = ((Get-PSDrive C).Free - $before) / 1MB
"Freed {0:N0} MB on C:" -f $freed
"#,
        ),
        s(
            "Disk health",
            "Get-PhysicalDisk | Select-Object FriendlyName, MediaType, HealthStatus, OperationalStatus, @{n='GB';e={[math]::Round($_.Size/1GB)}} | Format-Table -AutoSize",
        ),
        s(
            "Network adapters",
            "Get-NetIPConfiguration | Select-Object InterfaceAlias, @{n='IPv4';e={$_.IPv4Address.IPAddress}}, @{n='Gateway';e={$_.IPv4DefaultGateway.NextHop}}, @{n='DNS';e={$_.DNSServer.ServerAddresses -join ','}} | Format-Table -AutoSize",
        ),
    ]
}
