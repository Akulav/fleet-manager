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

use fleetview_shared::protocol::{CommandResult, RunSpec};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::paths;

fn audit(line: &str) {
    let path = paths::data_dir().join("audit.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}  {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), line);
    }
}

/// Run a PowerShell script (written to a temp file and executed with -File to
/// avoid command-line quoting pitfalls).
pub async fn run_powershell(command_id: String, script: String) -> CommandResult {
    audit(&format!("powershell [{command_id}]: {}", first_line(&script)));
    let script_path = paths::staging_dir().join(format!("cmd_{command_id}.ps1"));
    if let Err(e) = tokio::fs::create_dir_all(paths::staging_dir()).await {
        return fail(&command_id, &format!("staging dir: {e}"));
    }
    if let Err(e) = tokio::fs::write(&script_path, script.as_bytes()).await {
        return fail(&command_id, &format!("write script: {e}"));
    }

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .stdin(Stdio::null())
        .output()
        .await;

    let _ = tokio::fs::remove_file(&script_path).await;

    match output {
        Ok(o) => CommandResult {
            command_id,
            success: o.status.success(),
            exit_code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => fail(&command_id, &format!("spawn powershell: {e}")),
    }
}

/// Download an installer and run it silently.
pub async fn install_from_url(command_id: String, url: String, run: RunSpec) -> CommandResult {
    audit(&format!("install_from_url [{command_id}]: {url}"));
    let file_name = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("download.bin");
    let dest = paths::staging_dir().join(sanitize(file_name));
    if let Err(e) = tokio::fs::create_dir_all(paths::staging_dir()).await {
        return fail(&command_id, &format!("staging dir: {e}"));
    }

    match download(&url, &dest).await {
        Ok(()) => {}
        Err(e) => return fail(&command_id, &format!("download failed: {e:#}")),
    }

    run_file(&dest, &run).await
}

async fn download(url: &str, dest: &Path) -> anyhow::Result<()> {
    let resp = reqwest::get(url).await?.error_for_status()?;
    let bytes = resp.bytes().await?;
    let mut f = tokio::fs::File::create(dest).await?;
    f.write_all(&bytes).await?;
    f.flush().await?;
    Ok(())
}

/// Execute a file per its [`RunSpec`] and capture output.
pub async fn run_file(path: &Path, run: &RunSpec) -> CommandResult {
    audit(&format!(
        "run_file [{}]: {} {:?} (msi={})",
        run.command_id,
        path.display(),
        run.args,
        run.is_msi
    ));
    let mut cmd = if run.is_msi {
        let mut c = Command::new("msiexec.exe");
        c.arg("/i").arg(path);
        c.args(&run.args);
        c
    } else {
        let mut c = Command::new(path);
        c.args(&run.args);
        c
    };
    match cmd.stdin(Stdio::null()).output().await {
        Ok(o) => CommandResult {
            command_id: run.command_id.clone(),
            success: o.status.success(),
            exit_code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => fail(&run.command_id, &format!("spawn: {e}")),
    }
}

/// Stateful receiver for a chunked file push from the master.
pub struct FileTransfer {
    path: PathBuf,
    file: std::fs::File,
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
        std::fs::create_dir_all(paths::staging_dir())?;
        let path = paths::staging_dir().join(sanitize(file_name));
        let file = std::fs::File::create(&path)?;
        audit(&format!("file push begin: {} ({total_len} bytes)", path.display()));
        Ok(Self { path, file, received: 0, total: total_len, run_after })
    }

    pub fn write(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.file.write_all(data)?;
        self.received += data.len() as u64;
        Ok(())
    }

    pub fn received(&self) -> u64 {
        self.received
    }

    /// Flush and, if requested, run the file. Returns a result only when a run
    /// was requested.
    pub async fn finish(mut self) -> Option<CommandResult> {
        let _ = self.file.flush();
        drop(self.file);
        audit(&format!(
            "file push end: {} ({}/{} bytes)",
            self.path.display(),
            self.received,
            self.total
        ));
        match self.run_after {
            Some(run) => Some(run_file(&self.path, &run).await),
            None => None,
        }
    }
}

fn sanitize(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file.bin".into());
    base.chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect()
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
