//! Command execution: PowerShell, file push + run, and install-from-URL.
//!
//! All of this runs inside the LocalSystem service, so commands execute with
//! full administrative rights and in session 0 (non-interactive). Installers
//! therefore need silent switches (`/S`, `/qn`, `/quiet`, ...) — see docs.
//!
//! Every executed command is appended to a local audit log on the machine so
//! there is an on-box record of what was run remotely.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use fleetview_shared::protocol::{CommandResult, RunSpec};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::paths;

/// Hard limit for a PowerShell script run. A script that waits for input or
/// hangs would otherwise block forever (session 0 has no UI to answer it).
const PS_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Hard limit for an installer / pushed file run.
const RUN_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// Max bytes of stdout / stderr sent back. Larger output would exceed the
/// protocol frame limit and kill the whole connection.
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
/// Upper bound for a pushed or downloaded file.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub fn audit(line: &str) {
    let path = paths::data_dir().join("audit.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(
            f,
            "{}  {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            line
        );
    }
}

/// Create a fresh, randomly named directory under the staging dir. Each job
/// gets its own so concurrent pushes of the same file name can't collide.
fn new_job_dir() -> std::io::Result<PathBuf> {
    use rand::RngCore;
    let mut b = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut b);
    let dir = paths::staging_dir().join(hex::encode(b));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Run a PowerShell script (written to a temp file and executed with -File to
/// avoid command-line quoting pitfalls).
pub async fn run_powershell(command_id: String, script: String) -> CommandResult {
    audit(&format!(
        "powershell [{command_id}] ({} lines): {}",
        script.lines().count(),
        first_line(&script)
    ));
    let job_dir = match new_job_dir() {
        Ok(d) => d,
        Err(e) => return fail(&command_id, &format!("staging dir: {e}")),
    };
    let script_path = job_dir.join("script.ps1");
    // UTF-8 BOM so Windows PowerShell 5.1 reads non-ASCII (e.g. Romanian
    // diacritics) correctly instead of as ANSI.
    let mut body = vec![0xEF, 0xBB, 0xBF];
    body.extend_from_slice(script.as_bytes());
    if let Err(e) = tokio::fs::write(&script_path, body).await {
        let _ = tokio::fs::remove_dir_all(&job_dir).await;
        return fail(&command_id, &format!("write script: {e}"));
    }

    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ])
    .arg(&script_path);
    let result = run_captured(&command_id, cmd, PS_TIMEOUT).await;

    let _ = tokio::fs::remove_dir_all(&job_dir).await;
    result
}

/// Download an installer and run it silently.
pub async fn install_from_url(command_id: String, url: String, run: RunSpec) -> CommandResult {
    audit(&format!("install_from_url [{command_id}]: {url}"));
    let parsed = match reqwest::Url::parse(url.trim()) {
        Ok(u) => u,
        Err(e) => return fail(&command_id, &format!("invalid URL: {e}")),
    };
    // The file is executed as SYSTEM: plain HTTP would let anyone on the path
    // swap the installer for their own binary.
    if parsed.scheme() != "https" {
        return fail(&command_id, "only https:// URLs are allowed");
    }
    let file_name = parsed
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download.exe")
        .to_string();

    let job_dir = match new_job_dir() {
        Ok(d) => d,
        Err(e) => return fail(&command_id, &format!("staging dir: {e}")),
    };
    let dest = job_dir.join(sanitize(&file_name));

    let result = match download(parsed, &dest).await {
        Ok(()) => run_file(&dest, &run).await,
        Err(e) => fail(&command_id, &format!("download failed: {e:#}")),
    };
    let _ = tokio::fs::remove_dir_all(&job_dir).await;
    result
}

async fn download(url: reqwest::Url, dest: &Path) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 60))
        .https_only(true)
        .build()?;
    let mut resp = client.get(url).send().await?.error_for_status()?;
    if resp.content_length().unwrap_or(0) > MAX_FILE_BYTES {
        anyhow::bail!("file too large");
    }
    // Stream to disk instead of buffering the whole installer in memory.
    let mut f = tokio::fs::File::create(dest).await?;
    let mut written: u64 = 0;
    while let Some(chunk) = resp.chunk().await? {
        written += chunk.len() as u64;
        if written > MAX_FILE_BYTES {
            anyhow::bail!("file too large");
        }
        f.write_all(&chunk).await?;
    }
    f.flush().await?;
    Ok(())
}

/// Only one installer runs at a time on a PC. Windows Installer refuses a
/// second concurrent MSI (error 1618) and many EXE installers wrap an MSI, so
/// a fleet push of several packages queues here instead of failing.
static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Execute a file per its [`RunSpec`].
pub async fn run_file(path: &Path, run: &RunSpec) -> CommandResult {
    audit(&format!(
        "run_file [{}]: {} {:?} (msi={})",
        run.command_id,
        path.display(),
        run.args,
        run.is_msi
    ));
    // The agent's own self-update keeps its captured output and never waits
    // behind a (possibly hung) installer.
    if !run.is_msi && run.args.first().is_some_and(|a| a == "update") {
        let mut c = Command::new(path);
        c.args(&run.args);
        return run_captured(&run.command_id, c, RUN_TIMEOUT).await;
    }

    let _queue = INSTALL_LOCK.lock().await;
    let log_path = path.with_extension("install.log");
    let cmd = if run.is_msi {
        let mut c = Command::new("msiexec.exe");
        c.arg("/i").arg(path);
        c.args(&run.args);
        if !run.args.iter().any(|a| a.to_lowercase().starts_with("/l")) {
            c.arg("/L*v").arg(&log_path);
        }
        c
    } else {
        let mut c = Command::new(path);
        c.args(&run.args);
        c
    };
    let mut result = run_installer(&run.command_id, cmd, RUN_TIMEOUT).await;
    if run.is_msi && !result.success {
        if let Some(tail) = read_log_tail(&log_path, 40) {
            result.stdout.push_str("\n--- end of the MSI log ---\n");
            result.stdout.push_str(&tail);
        }
    }
    result
}

/// Run an installer without capturing its output pipes.
///
/// Installers rarely print anything, but they often start a service, tray
/// app or updater that inherits stdout/stderr. Waiting for those pipes to
/// close (as `wait_with_output` does) then blocks until the 60-minute
/// timeout even though the installer itself finished long ago — so only the
/// process exit is awaited here.
async fn run_installer(command_id: &str, mut cmd: Command, timeout: Duration) -> CommandResult {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(command_id, &format!("could not start the installer: {e}")),
    };
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => {
            let code = status.code();
            let (success, meaning) = installer_exit_meaning(code);
            CommandResult {
                command_id: command_id.to_string(),
                success,
                exit_code: code,
                stdout: match code {
                    Some(c) => format!("installer exited with code {c}: {meaning}"),
                    None => "installer ended without an exit code".into(),
                },
                stderr: String::new(),
            }
        }
        Ok(Err(e)) => fail(command_id, &format!("wait: {e}")),
        Err(_) => fail(
            command_id,
            &format!(
                "timed out after {} min and was killed — the installer is probably waiting \
                 for input on a hidden window: check its silent switches",
                timeout.as_secs() / 60
            ),
        ),
    }
}

/// Whether an installer exit code means success, plus a short explanation.
/// Covers Windows Installer codes, which most EXE wrappers pass through.
pub fn installer_exit_meaning(code: Option<i32>) -> (bool, &'static str) {
    match code {
        Some(0) => (true, "success"),
        Some(3010) => (true, "success — a restart is needed to finish"),
        Some(1641) => (true, "success — the installer started a restart"),
        Some(1602) => (
            false,
            "cancelled (the installer showed UI that was dismissed)",
        ),
        Some(1603) => (false, "fatal error during installation (see the MSI log)"),
        Some(1618) => (false, "another installation is already in progress"),
        Some(1619) => (false, "the package could not be opened"),
        Some(1620) => (false, "the package is not a valid MSI"),
        Some(1625) => (false, "blocked by system policy"),
        Some(1633) => (
            false,
            "this package does not support this platform (32/64-bit)",
        ),
        Some(1638) => (
            false,
            "another version of this product is already installed",
        ),
        Some(1639) => (false, "invalid command-line arguments"),
        Some(1) | Some(2) => (false, "generic failure — often wrong silent arguments"),
        Some(_) => (false, "failed"),
        None => (false, "killed"),
    }
}

/// Last `lines` lines of an installer log (MSI logs are UTF-16 LE).
fn read_log_tail(path: &Path, lines: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    Some(all[start..].join("\n"))
}

/// Spawn `cmd`, wait (bounded by `timeout`) and capture its output.
async fn run_captured(command_id: &str, mut cmd: Command, timeout: Duration) -> CommandResult {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(command_id, &format!("spawn: {e}")),
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => CommandResult {
            command_id: command_id.to_string(),
            success: o.status.success(),
            exit_code: o.status.code(),
            stdout: truncate_output(&o.stdout),
            stderr: truncate_output(&o.stderr),
        },
        Ok(Err(e)) => fail(command_id, &format!("wait: {e}")),
        // Dropping the future drops the child, which kills it (kill_on_drop).
        Err(_) => fail(
            command_id,
            &format!(
                "timed out after {} min and was killed",
                timeout.as_secs() / 60
            ),
        ),
    }
}

fn truncate_output(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() <= MAX_OUTPUT_BYTES {
        return s.into_owned();
    }
    let mut cut = MAX_OUTPUT_BYTES;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… [output truncated]", &s[..cut])
}

/// Stateful receiver for a chunked file push from the master.
pub struct FileTransfer {
    job_dir: PathBuf,
    path: PathBuf,
    file: Option<std::fs::File>,
    received: u64,
    total: u64,
    run_after: Option<RunSpec>,
}

impl FileTransfer {
    pub fn begin(
        _transfer_id: &str,
        file_name: &str,
        total_len: u64,
        run_after: Option<RunSpec>,
    ) -> anyhow::Result<Self> {
        if total_len > MAX_FILE_BYTES {
            anyhow::bail!("file too large ({total_len} bytes)");
        }
        let job_dir = new_job_dir()?;
        let path = job_dir.join(sanitize(file_name));
        let file = std::fs::File::create(&path)?;
        audit(&format!(
            "file push begin: {} ({total_len} bytes)",
            path.display()
        ));
        Ok(Self {
            job_dir,
            path,
            file: Some(file),
            received: 0,
            total: total_len,
            run_after,
        })
    }

    pub fn write(&mut self, data: &[u8]) -> anyhow::Result<()> {
        if self.received + data.len() as u64 > self.total {
            anyhow::bail!("more data than announced ({} bytes)", self.total);
        }
        let f = self
            .file
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("closed"))?;
        f.write_all(data)?;
        self.received += data.len() as u64;
        Ok(())
    }

    pub fn received(&self) -> u64 {
        self.received
    }

    pub fn command_id(&self) -> Option<&str> {
        self.run_after.as_ref().map(|r| r.command_id.as_str())
    }

    /// Flush and, if requested, run the file. Returns a result only when a run
    /// was requested (or when the transfer turned out to be incomplete).
    pub async fn finish(mut self) -> Option<CommandResult> {
        let flushed = match self.file.take() {
            Some(mut f) => f.flush().and_then(|_| f.sync_all()),
            None => Ok(()),
        };
        audit(&format!(
            "file push end: {} ({}/{} bytes)",
            self.path.display(),
            self.received,
            self.total
        ));
        let run = self.run_after.take();
        if self.received != self.total || flushed.is_err() {
            let msg = format!(
                "transfer incomplete ({}/{} bytes); not running it",
                self.received, self.total
            );
            let _ = tokio::fs::remove_dir_all(&self.job_dir).await;
            return Some(fail(
                run.as_ref().map(|r| r.command_id.as_str()).unwrap_or(""),
                &msg,
            ));
        }
        // Pushed files without a run spec are left in staging for later use.
        let run = run?;
        let result = run_file(&self.path, &run).await;
        let _ = tokio::fs::remove_dir_all(&self.job_dir).await;
        Some(result)
    }
}

impl Drop for FileTransfer {
    fn drop(&mut self) {
        // An unfinished transfer (connection dropped mid-way) leaves a partial
        // file behind; clean it up. Completed runs already removed the dir.
        if self.file.is_some() || self.run_after.is_some() {
            self.file = None;
            let _ = std::fs::remove_dir_all(&self.job_dir);
        }
    }
}

fn sanitize(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file.bin".into());
    let cleaned: String = base
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ').to_string();
    if cleaned.is_empty() {
        "file.bin".into()
    } else {
        cleaned
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(120).collect()
}

fn fail(command_id: &str, msg: &str) -> CommandResult {
    CommandResult {
        command_id: command_id.to_string(),
        success: false,
        exit_code: None,
        stdout: String::new(),
        stderr: msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_paths_and_bad_chars() {
        assert_eq!(sanitize(r"..\..\Windows\evil.exe"), "evil.exe");
        assert_eq!(sanitize("a/b/setup?.msi"), "setup_.msi");
        assert_eq!(sanitize(".."), "file.bin");
        assert_eq!(sanitize(""), "file.bin");
    }

    #[test]
    fn reboot_required_counts_as_success() {
        assert!(installer_exit_meaning(Some(0)).0);
        assert!(installer_exit_meaning(Some(3010)).0);
        assert!(installer_exit_meaning(Some(1641)).0);
        assert!(!installer_exit_meaning(Some(1603)).0);
        assert!(!installer_exit_meaning(None).0);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "ă".repeat(MAX_OUTPUT_BYTES); // 2 bytes each
        let out = truncate_output(s.as_bytes());
        assert!(out.ends_with("[output truncated]"));
    }
}
