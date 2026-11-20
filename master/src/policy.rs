//! Fleet standards: one local admin account, one desktop background and one
//! app package that every PC should have, plus PC naming.
//!
//! Everything here is applied with ordinary `RunPowerShell` / file pushes, so
//! it works with agents of any version. Stored in `policy.json` in the master
//! data folder. That file holds the standard admin password: it sits next to
//! `master.settings.json`, whose enrollment secret and TLS key already give
//! SYSTEM on every agent, so it adds no new exposure — but back it up and keep
//! the folder private all the same.

use serde::{Deserialize, Serialize};

/// Keys in the per-agent "applied" map.
pub const KEY_ADMIN: &str = "admin";
pub const KEY_WALLPAPER: &str = "wallpaper";

/// Biggest wallpaper image accepted (it travels inside the script).
pub const MAX_WALLPAPER_BYTES: u64 = 12 * 1024 * 1024;

/// One installer in a package or install queue.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct AppItem {
    pub path: String,
    pub args: String,
    pub is_msi: bool,
    /// Skip PCs whose installed-software list has a program whose name
    /// contains this (case-insensitive). Empty = always install.
    #[serde(default)]
    pub detect: String,
    /// Detected installer type, for display.
    #[serde(default)]
    pub kind: String,
}

impl AppItem {
    /// Build an entry for a file with its silent switches auto-detected.
    pub fn from_path(path: &std::path::Path) -> Self {
        let kind = crate::installer::detect(path);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        Self {
            path: path.display().to_string(),
            args: kind.silent_args().to_string(),
            is_msi: kind == crate::installer::InstallerKind::Msi,
            detect: guess_detect_name(&stem),
            kind: kind.label().to_string(),
        }
    }

    pub fn file_name(&self) -> String {
        std::path::Path::new(&self.path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.clone())
    }

    /// Key in the per-agent "applied" map.
    pub fn key(&self) -> String {
        format!("app:{}", self.file_name().to_lowercase())
    }

    /// Changes when the file on disk changes (a new version of the installer).
    pub fn revision(&self) -> Option<String> {
        file_revision(&self.path)
    }

    pub fn args_vec(&self) -> Vec<String> {
        self.args.split_whitespace().map(str::to_string).collect()
    }

    /// Whether `software` already lists this app.
    pub fn installed_in(&self, software: &[fleetview_shared::protocol::SoftwareEntry]) -> bool {
        let d = self.detect.trim().to_lowercase();
        !d.is_empty() && software.iter().any(|s| s.name.to_lowercase().contains(&d))
    }
}

/// "size-mtime" of a file, or None when it can't be read.
pub fn file_revision(path: &str) -> Option<String> {
    let m = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some(format!("{}-{mtime}", m.len()))
}

/// "ChromeSetup-x64_v120.exe" → "chrome". Only a starting point; the admin
/// can edit it.
fn guess_detect_name(stem: &str) -> String {
    let word: String = stem
        .chars()
        .take_while(|c| c.is_alphabetic() || *c == ' ')
        .collect();
    let lower = word.trim().to_lowercase();
    let lower = lower
        .trim_end_matches("setup")
        .trim_end_matches("installer")
        .trim_end_matches("install")
        .trim();
    if lower.len() >= 3 {
        lower.to_string()
    } else {
        String::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum WallStyle {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}

impl WallStyle {
    pub const ALL: [WallStyle; 6] = [
        WallStyle::Fill,
        WallStyle::Fit,
        WallStyle::Stretch,
        WallStyle::Center,
        WallStyle::Tile,
        WallStyle::Span,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WallStyle::Fill => "Fill",
            WallStyle::Fit => "Fit",
            WallStyle::Stretch => "Stretch",
            WallStyle::Center => "Center",
            WallStyle::Tile => "Tile",
            WallStyle::Span => "Span",
        }
    }

    /// (Control Panel\Desktop WallpaperStyle, TileWallpaper, policy WallpaperStyle)
    fn reg_values(self) -> (&'static str, &'static str, &'static str) {
        match self {
            WallStyle::Fill => ("10", "0", "4"),
            WallStyle::Fit => ("6", "0", "3"),
            WallStyle::Stretch => ("2", "0", "2"),
            WallStyle::Center => ("0", "0", "0"),
            WallStyle::Tile => ("0", "1", "1"),
            WallStyle::Span => ("22", "0", "5"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub admin_user: String,
    pub admin_password: String,
    /// Also take every other local account out of Administrators.
    pub admin_demote_others: bool,
    pub admin_auto: bool,
    pub admin_rev: u64,

    pub wallpaper_path: String,
    pub wallpaper_style: WallStyle,
    /// Enforce it (users can't change it) instead of only setting it.
    pub wallpaper_lock: bool,
    pub wallpaper_auto: bool,
    pub wallpaper_rev: u64,
    /// Size + mtime of the image when saved, to notice a replaced file.
    pub wallpaper_file: String,

    pub apps: Vec<AppItem>,
    pub apps_auto: bool,

    /// e.g. "ANSC-###": each `#` run becomes a zero-padded number.
    pub name_pattern: String,
    pub name_start: u32,
    pub name_restart: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            admin_user: String::new(),
            admin_password: String::new(),
            admin_demote_others: false,
            admin_auto: false,
            admin_rev: 1,
            wallpaper_path: String::new(),
            wallpaper_style: WallStyle::Fill,
            wallpaper_lock: false,
            wallpaper_auto: false,
            wallpaper_rev: 1,
            wallpaper_file: String::new(),
            apps: Vec::new(),
            apps_auto: false,
            name_pattern: "PC-###".into(),
            name_start: 1,
            name_restart: true,
        }
    }
}

impl Policy {
    pub fn path() -> std::path::PathBuf {
        crate::paths::data_dir().join("policy.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        crate::paths::ensure_data_dir()?;
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let path = Self::path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    }

    pub fn admin_ready(&self) -> bool {
        validate_account_name(&self.admin_user).is_ok() && !self.admin_password.is_empty()
    }

    /// Save, bumping the revision of every standard whose settings changed
    /// since the last save, so PCs on automatic get the new version.
    pub fn save_bumping(&mut self) -> std::io::Result<()> {
        let old = Self::load();
        if old.admin_user.trim() != self.admin_user.trim()
            || old.admin_password != self.admin_password
            || old.admin_demote_others != self.admin_demote_others
        {
            self.admin_rev = old.admin_rev.max(self.admin_rev) + 1;
        }
        self.wallpaper_file = file_revision(&self.wallpaper_path).unwrap_or_default();
        if old.wallpaper_path != self.wallpaper_path
            || old.wallpaper_style != self.wallpaper_style
            || old.wallpaper_lock != self.wallpaper_lock
            || old.wallpaper_file != self.wallpaper_file
        {
            self.wallpaper_rev = old.wallpaper_rev.max(self.wallpaper_rev) + 1;
        }
        self.save()
    }

    pub fn wallpaper_ready(&self) -> bool {
        std::fs::metadata(&self.wallpaper_path)
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_WALLPAPER_BYTES)
    }
}

// ---------------------------------------------------------------------------
// PC names
// ---------------------------------------------------------------------------

/// A valid Windows computer (NetBIOS) name: 1–15 chars of letters, digits
/// and hyphens, not only digits, not starting or ending with a hyphen.
pub fn validate_pc_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty name".into());
    }
    if name.len() > 15 {
        return Err(format!("“{name}” is longer than 15 characters"));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("“{name}”: use only letters, digits and hyphens"));
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("“{name}” can't be only digits"));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(format!("“{name}” can't start or end with a hyphen"));
    }
    Ok(())
}

/// Expand a naming pattern for the `n`th PC: every run of `#` becomes `n`
/// zero-padded to the run's length. A pattern without `#` gets the number
/// appended.
pub fn expand_pattern(pattern: &str, n: u32) -> String {
    let pattern = pattern.trim();
    if !pattern.contains('#') {
        return format!("{pattern}{n}").to_uppercase();
    }
    let mut out = String::new();
    let mut run = 0;
    for c in pattern.chars().chain(std::iter::once('\0')) {
        if c == '#' {
            run += 1;
            continue;
        }
        if run > 0 {
            out.push_str(&format!("{n:0run$}"));
            run = 0;
        }
        if c != '\0' {
            out.push(c);
        }
    }
    out.to_uppercase()
}

pub fn rename_script(new_name: &str, restart: bool) -> String {
    let mut s = format!(
        "# FleetView: rename this PC to {new_name}\n\
         $ErrorActionPreference = 'Stop'\n\
         $new = {q}\n\
         $pending = (Get-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\ComputerName\\ComputerName').ComputerName\n\
         if ($env:COMPUTERNAME -ieq $new) {{ \"Already named $new\"; exit 0 }}\n\
         if ($pending -ieq $new) {{ \"Already renamed to $new - waiting for a restart\" }} else {{\n\
         \x20 $cs = Get-CimInstance Win32_ComputerSystem\n\
         \x20 if ($cs.PartOfDomain) {{ Write-Warning \"Joined to the domain $($cs.Domain): the domain may refuse a rename without domain credentials.\" }}\n\
         \x20 Rename-Computer -NewName $new -Force -WarningAction SilentlyContinue\n\
         \x20 \"Renamed $env:COMPUTERNAME -> $new (takes effect after a restart)\"\n\
         }}\n",
        q = ps_quote(new_name)
    );
    if restart {
        s.push_str(
            "shutdown.exe /r /t 60 /c \"This PC will restart in 1 minute to apply its new name (requested by IT). Please save your work.\"\n\
             \"Restarting in 1 minute\"\n",
        );
    }
    s
}

// ---------------------------------------------------------------------------
// Local admin account
// ---------------------------------------------------------------------------

pub fn validate_account_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("enter a user name".into());
    }
    if name.chars().count() > 20 {
        return Err("local user names are at most 20 characters".into());
    }
    if name
        .chars()
        .any(|c| "\"/\\[]:;|=,+*?<>@".contains(c) || c.is_control())
    {
        return Err("the name contains a character Windows doesn't allow".into());
    }
    if name.chars().all(|c| c == '.' || c == ' ') {
        return Err("the name can't be only dots or spaces".into());
    }
    Ok(())
}

/// Create (or reset) the standard admin account and put it in the local
/// Administrators group (found by SID, so it also works on Romanian Windows
/// where the group is "Administratori").
///
/// The password travels base64-encoded inside the script and is never on its
/// first line, which is the only part the agent writes to its audit log.
pub fn admin_script(user: &str, password: &str, demote_others: bool) -> String {
    let b64 = base64_encode(password.as_bytes());
    let mut s = format!(
        r#"# FleetView: set the standard local admin account
$ErrorActionPreference = 'Stop'
$name = {name}
$pw = ConvertTo-SecureString ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{b64}'))) -AsPlainText -Force
$admins = Get-LocalGroup -SID 'S-1-5-32-544'
function Get-AdminSids {{
  $g = [ADSI]"WinNT://$env:COMPUTERNAME/$($admins.Name),group"
  foreach ($m in @($g.Invoke('Members'))) {{
    try {{
      $bytes = [byte[]]$m.GetType().InvokeMember('objectSid', 'GetProperty', $null, $m, $null)
      (New-Object Security.Principal.SecurityIdentifier($bytes, 0)).Value
    }} catch {{}}
  }}
}}
$u = Get-LocalUser -Name $name -ErrorAction SilentlyContinue
if ($u) {{
  Set-LocalUser -Name $name -Password $pw -PasswordNeverExpires $true -AccountNeverExpires
  if (-not $u.Enabled) {{ Enable-LocalUser -Name $name; "Enabled $name" }}
  "Password set for $name"
}} else {{
  New-LocalUser -Name $name -Password $pw -PasswordNeverExpires -AccountNeverExpires -Description 'Standard IT admin account (FleetView)' | Out-Null
  "Created $name"
}}
$u = Get-LocalUser -Name $name
if ((Get-AdminSids) -notcontains $u.SID.Value) {{
  Add-LocalGroupMember -Group $admins -Member $u
  "Added $name to $($admins.Name)"
}} else {{
  "$name is already in $($admins.Name)"
}}
"#,
        name = ps_quote(user.trim()),
    );
    if demote_others {
        s.push_str(
            r#"$sids = @(Get-AdminSids)
foreach ($o in Get-LocalUser | Where-Object { $sids -contains $_.SID.Value -and $_.SID.Value -ne $u.SID.Value }) {
  try { Remove-LocalGroupMember -Group $admins -Member $o; "Removed $($o.Name) from $($admins.Name)" }
  catch { Write-Warning "Could not remove $($o.Name): $_" }
}
"#,
        );
    }
    s.push_str(
        r#"$names = foreach ($sid in Get-AdminSids) { try { (New-Object Security.Principal.SecurityIdentifier($sid)).Translate([Security.Principal.NTAccount]).Value } catch { $sid } }
"Administrators now: $($names -join ', ')"
"#,
    );
    s
}

// ---------------------------------------------------------------------------
// Desktop background
// ---------------------------------------------------------------------------

/// Write the image to `%windir%\Web\Wallpaper\FleetView` and make it the
/// background of every user profile on the PC — loaded, not loaded, and the
/// Default profile that new users are created from. Users logged on now get it
/// immediately through a one-off scheduled task that runs in their session.
/// With `lock`, the per-user wallpaper policy is set too, so users can't
/// change it; without it, any earlier lock is removed.
pub fn wallpaper_script(image: &[u8], ext: &str, style: WallStyle, lock: bool) -> String {
    let (wstyle, tile, pstyle) = style.reg_values();
    let ext: String = ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(5)
        .collect::<String>()
        .to_lowercase();
    let ext = if ext.is_empty() { "jpg".into() } else { ext };
    format!(
        r#"# FleetView: set the standard desktop background
$ErrorActionPreference = 'Continue'
$dir = Join-Path $env:windir 'Web\Wallpaper\FleetView'
$file = Join-Path $dir 'wallpaper.{ext}'
try {{
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  Get-ChildItem $dir -Filter 'wallpaper.*' | Remove-Item -Force -ErrorAction SilentlyContinue
  [IO.File]::WriteAllBytes($file, [Convert]::FromBase64String('{b64}'))
}} catch {{ "Cannot write the image: $_"; exit 1 }}
$lock = ${lock}
function Set-Hive($root) {{
  $d = "$root\Control Panel\Desktop"
  & reg.exe add $d /v Wallpaper /t REG_SZ /d $file /f *> $null
  & reg.exe add $d /v WallpaperStyle /t REG_SZ /d '{wstyle}' /f *> $null
  & reg.exe add $d /v TileWallpaper /t REG_SZ /d '{tile}' /f *> $null
  $ok = $LASTEXITCODE -eq 0
  $p = "$root\Software\Microsoft\Windows\CurrentVersion\Policies\System"
  if ($lock) {{
    & reg.exe add $p /v Wallpaper /t REG_SZ /d $file /f *> $null
    & reg.exe add $p /v WallpaperStyle /t REG_SZ /d '{pstyle}' /f *> $null
  }} else {{
    & reg.exe delete $p /v Wallpaper /f *> $null
    & reg.exe delete $p /v WallpaperStyle /f *> $null
  }}
  $ok
}}
$done = 0
$loaded = @((Get-ChildItem Registry::HKEY_USERS).PSChildName)
foreach ($sid in $loaded | Where-Object {{ $_ -match '^S-1-5-21-[\d-]+$' }}) {{
  if (Set-Hive "HKU\$sid") {{ $done++ }}
}}
$profiles = Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList' |
  Where-Object {{ $_.PSChildName -match '^S-1-5-21-[\d-]+$' -and $loaded -notcontains $_.PSChildName }}
$hives = @($profiles | ForEach-Object {{ Join-Path $_.GetValue('ProfileImagePath') 'NTUSER.DAT' }})
$defProfile = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList').Default
if ($defProfile) {{ $hives += (Join-Path $defProfile 'NTUSER.DAT') }}
$i = 0
foreach ($h in $hives) {{
  if (-not (Test-Path -LiteralPath $h)) {{ continue }}
  $i++
  $mount = "HKU\FleetView_$i"
  & reg.exe load $mount $h *> $null
  if ($LASTEXITCODE -ne 0) {{ continue }}
  if (Set-Hive $mount) {{ $done++ }}
  [gc]::Collect()
  & reg.exe unload $mount *> $null
}}
"Background set for $done profile(s), including the default profile for new users"
# Apply it now for whoever is logged on (SystemParametersInfo must run in
# the user's own session).
$users = (Get-CimInstance Win32_Process -Filter "Name='explorer.exe'" -ErrorAction SilentlyContinue | Measure-Object).Count
if ($users -gt 0) {{
  $cmd = "`$t = Add-Type -MemberDefinition '[DllImport(""user32.dll"", CharSet=CharSet.Unicode)] public static extern int SystemParametersInfo(int a, int b, string c, int d);' -Name W -Namespace FleetView -PassThru; `$t::SystemParametersInfo(20, 0, '$file', 3) | Out-Null"
  $enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($cmd))
  try {{
    $group = (New-Object Security.Principal.SecurityIdentifier 'S-1-5-32-545').Translate([Security.Principal.NTAccount]).Value
    $a = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand $enc"
    $pr = New-ScheduledTaskPrincipal -GroupId $group -RunLevel Limited
    $st = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName 'FleetView-Wallpaper' -Action $a -Principal $pr -Settings $st -Force -ErrorAction Stop | Out-Null
    Start-ScheduledTask -TaskName 'FleetView-Wallpaper'
    Start-Sleep -Seconds 8
    Unregister-ScheduledTask -TaskName 'FleetView-Wallpaper' -Confirm:$false
    "Refreshed the desktop of the logged-on user(s)"
  }} catch {{ "Logged-on users see it after their next sign-in ($_)" }}
}}
exit 0
"#,
        b64 = base64_encode(image),
        lock = if lock { "true" } else { "false" },
    )
}

// ---------------------------------------------------------------------------

/// Quote a value as a PowerShell single-quoted string literal.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_expansion() {
        assert_eq!(expand_pattern("ansc-###", 7), "ANSC-007");
        assert_eq!(expand_pattern("PC-#", 12), "PC-12");
        assert_eq!(expand_pattern("LAB", 3), "LAB3");
        assert_eq!(expand_pattern("R##-PC##", 4), "R04-PC04");
    }

    #[test]
    fn pc_name_rules() {
        assert!(validate_pc_name("ANSC-001").is_ok());
        assert!(validate_pc_name("1234").is_err());
        assert!(validate_pc_name("TOO-LONG-NAME-123").is_err());
        assert!(validate_pc_name("bad_name").is_err());
        assert!(validate_pc_name("-x").is_err());
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode("parolă".as_bytes()), "cGFyb2zEgw==");
    }

    #[test]
    fn admin_script_keeps_password_off_the_first_line() {
        let s = admin_script("itadmin", "S3cret'pw", true);
        let first = s.lines().next().unwrap();
        assert!(!first.contains("S3cret"));
        assert!(
            !s.contains("S3cret"),
            "password must only appear base64-encoded"
        );
        assert!(s.contains("S-1-5-32-544"));
        assert!(s.contains("Remove-LocalGroupMember"));
        assert!(!admin_script("itadmin", "x", false).contains("Remove-LocalGroupMember"));
    }

    #[test]
    fn account_name_rules() {
        assert!(validate_account_name("itadmin").is_ok());
        assert!(validate_account_name("").is_err());
        assert!(validate_account_name("a/b").is_err());
        assert!(validate_account_name("this-name-is-way-too-long").is_err());
    }

    #[test]
    fn detect_name_guess() {
        assert_eq!(guess_detect_name("ChromeSetup"), "chrome");
        assert_eq!(guess_detect_name("7z2301-x64"), "");
        assert_eq!(guess_detect_name("Firefox Setup 120.0"), "firefox");
    }

    #[test]
    fn rename_script_quotes_and_restarts() {
        let s = rename_script("ANSC-001", true);
        assert!(s.contains("$new = 'ANSC-001'"));
        assert!(s.contains("shutdown.exe /r"));
        assert!(!rename_script("ANSC-001", false).contains("shutdown"));
    }

    /// Windows PowerShell 5.1 reads a script without BOM as ANSI, where the
    /// bytes of "—" or "“" decode to smart quotes that end strings. Keep the
    /// generated script text pure ASCII.
    #[test]
    fn generated_scripts_are_ascii() {
        for s in [
            admin_script("itadmin", "parolă", true),
            wallpaper_script(b"img", "jpg", WallStyle::Fill, true),
            wallpaper_script(b"img", "png", WallStyle::Span, false),
            rename_script("ANSC-001", true),
        ] {
            if let Some(c) = s.chars().find(|c| !c.is_ascii()) {
                panic!("non-ASCII {c:?} in generated script:\n{s}");
            }
        }
    }
}
