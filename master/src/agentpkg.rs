//! The agent package: the agent exe the master hands out, both as a
//! self-provisioning setup file and as a remote update.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use fleetview_shared::package::{self, Version};
use fleetview_shared::protocol::{MasterMsg, RunSpec};

use crate::state::{SharedState, TargetStatus};

/// Agent exe loaded into memory (config trailer, if any, already stripped).
pub struct AgentPackage {
    pub path: PathBuf,
    pub version: Version,
    pub image: Arc<Vec<u8>>,
}

impl AgentPackage {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let (image, _) = package::split_config_trailer(&bytes);
        let version = package::find_agent_version(image).with_context(|| {
            format!(
                "{} is not a FleetView agent 0.2.0 or newer (no version marker)",
                path.display()
            )
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            version,
            image: Arc::new(image.to_vec()),
        })
    }

    /// `fleetview-agent.exe` next to the master (where the installer puts it).
    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("fleetview-agent.exe")))
            .unwrap_or_else(|| PathBuf::from("fleetview-agent.exe"))
    }

    /// A ready-to-run agent setup with `config_json` embedded.
    pub fn setup_with_config(&self, config_json: &str) -> anyhow::Result<Vec<u8>> {
        // Refuse to embed something that isn't JSON: the agent would reject it
        // only at install time, on the user's PC.
        serde_json::from_str::<serde_json::Value>(config_json)
            .context("the agent config is not valid JSON")?;
        package::with_config_trailer(&self.image, config_json.as_bytes())
    }
}

/// How an agent's reported version compares with the package.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UpdateState {
    /// Older than the package (or a pre-marker 0.1.x build).
    Outdated,
    Current,
    /// Newer than the package: never downgrade from the UI.
    Newer,
    Unknown,
}

pub fn update_state(agent_version: &str, pkg: &Version) -> UpdateState {
    match Version::parse(agent_version) {
        Some(v) if v < *pkg => UpdateState::Outdated,
        Some(v) if v == *pkg => UpdateState::Current,
        Some(_) => UpdateState::Newer,
        None => UpdateState::Unknown,
    }
}

/// Agents updated at the same time (each transfer is several MB).
const PARALLEL_UPDATES: usize = 4;
const CHUNK: usize = 256 * 1024;
/// How long to wait for an agent to acknowledge the whole file.
const ACK_TIMEOUT: Duration = Duration::from_secs(180);

/// Push the package to `ids` and have each agent install it. Agents of any
/// version handle this: it is an ordinary file push run with `update`.
pub fn start_update(
    state: SharedState,
    pkg: &AgentPackage,
    ids: Vec<String>,
    emanagement_url: &str,
    new_id: fn() -> String,
    job: Option<u64>,
) {
    let mut args = vec!["update".to_string()];
    let url = emanagement_url.trim();
    if !url.is_empty() {
        args.push("--set".into());
        args.push(format!("emanagement_url={url}"));
    }
    let count = ids.len();
    let queue = Arc::new(Mutex::new(VecDeque::from(ids)));
    for _ in 0..PARALLEL_UPDATES.min(count) {
        let queue = queue.clone();
        let state = state.clone();
        let image = pkg.image.clone();
        let version = pkg.version;
        let args = args.clone();
        std::thread::spawn(move || loop {
            let Some(id) = queue.lock().unwrap().pop_front() else {
                break;
            };
            push_one(&state, &id, &image, version, &args, new_id, job);
        });
    }
    if let Ok(mut s) = state.lock() {
        s.log(format!(
            "agent update to {} started for {count} PC(s)",
            pkg.version
        ));
    }
}

/// How a [`send_file_and_wait`] ended.
#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// The agent acknowledged every byte (a run, if requested, has started).
    Delivered,
    /// No connection, or it dropped before the file was confirmed.
    Offline,
    TimedOut,
}

/// Stream `data` to one agent as a file push (run afterwards if `run` is set)
/// and wait until the agent has acknowledged every byte. Waiting means a
/// fleet-wide push holds at most one queued copy per worker thread instead of
/// one per PC, so a big installer can't exhaust the master's memory.
pub fn send_file_and_wait(
    state: &SharedState,
    id: &str,
    transfer_id: String,
    file_name: String,
    data: &[u8],
    run: Option<RunSpec>,
) -> SendOutcome {
    // Register the transfer so the backend records its acks (it ignores acks
    // for transfers nobody is waiting on).
    let tx = {
        let mut s = state.lock().unwrap();
        let Some(a) = s.agents.get_mut(id) else {
            return SendOutcome::Offline;
        };
        let Some(tx) = a.tx.clone() else {
            return SendOutcome::Offline;
        };
        a.acks.insert(transfer_id.clone(), 0);
        tx
    };
    let total = data.len() as u64;
    let mut ok = tx
        .send(MasterMsg::FileBegin {
            transfer_id: transfer_id.clone(),
            file_name,
            total_len: total,
            run_after: run,
        })
        .is_ok();
    for chunk in data.chunks(CHUNK) {
        if !ok {
            break;
        }
        ok = tx
            .send(MasterMsg::FileChunk {
                transfer_id: transfer_id.clone(),
                data: chunk.to_vec(),
            })
            .is_ok();
    }
    ok = ok
        && tx
            .send(MasterMsg::FileEnd {
                transfer_id: transfer_id.clone(),
            })
            .is_ok();
    drop(tx);
    if !ok || total == 0 {
        if let Some(a) = state.lock().unwrap().agents.get_mut(id) {
            a.acks.remove(&transfer_id);
        }
        // An empty file has no chunks, so no acks to wait for.
        return if ok {
            SendOutcome::Delivered
        } else {
            SendOutcome::Offline
        };
    }

    let deadline = Instant::now() + ACK_TIMEOUT;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let mut s = state.lock().unwrap();
        let Some(a) = s.agents.get_mut(id) else {
            return SendOutcome::Offline;
        };
        if a.acks.get(&transfer_id) == Some(&total) {
            a.acks.remove(&transfer_id);
            return SendOutcome::Delivered;
        }
        if !a.online {
            a.acks.remove(&transfer_id);
            return SendOutcome::Offline;
        }
        if Instant::now() >= deadline {
            a.acks.remove(&transfer_id);
            return SendOutcome::TimedOut;
        }
    }
}

fn set_note(state: &SharedState, id: &str, note: String) {
    if let Some(a) = state.lock().unwrap().agents.get_mut(id) {
        a.update_note = Some(note);
    }
}

/// Note a failed update on the agent and its job target.
fn fail(state: &SharedState, id: &str, job: Option<u64>, status: TargetStatus, note: &str) {
    let mut s = state.lock().unwrap();
    if let Some(a) = s.agents.get_mut(id) {
        a.update_note = Some(note.to_string());
        if status == TargetStatus::Failed {
            a.update_command = None;
        }
    }
    if let Some(j) = job {
        s.finish_target(j, id, status, note);
    }
}

fn push_one(
    state: &SharedState,
    id: &str,
    image: &[u8],
    version: Version,
    args: &[String],
    new_id: fn() -> String,
    job: Option<u64>,
) {
    let command_id = new_id();
    let transfer_id = new_id();
    let online = {
        let mut s = state.lock().unwrap();
        let online = match s.agents.get_mut(id) {
            None => return,
            Some(a) if a.tx.is_none() => false,
            Some(a) => {
                a.update_note = Some(format!("sending {version}…"));
                a.update_command = Some(command_id.clone());
                true
            }
        };
        if online {
            if let Some(j) = job {
                s.attach_command(j, id, &command_id);
            }
        }
        online
    };
    if !online {
        fail(
            state,
            id,
            job,
            TargetStatus::Skipped,
            "offline — not updated",
        );
        return;
    }
    let sent = send_file_and_wait(
        state,
        id,
        transfer_id,
        format!("fleetview-agent-{version}.exe"),
        image,
        Some(RunSpec {
            command_id,
            args: args.to_vec(),
            is_msi: false,
        }),
    );
    match sent {
        SendOutcome::Delivered => {}
        SendOutcome::Offline => {
            fail(
                state,
                id,
                job,
                TargetStatus::Failed,
                "connection lost while sending",
            );
            return;
        }
        SendOutcome::TimedOut => {
            fail(
                state,
                id,
                job,
                TargetStatus::Failed,
                "no confirmation from the agent (timed out)",
            );
            return;
        }
    }
    let s = state.lock().unwrap();
    let still_pending = s.agents.get(id).is_some_and(|a| {
        a.update_command.is_some()
            && a.update_note
                .as_deref()
                .is_some_and(|n| n.starts_with("sending"))
    });
    drop(s);
    if still_pending {
        set_note(
            state,
            id,
            format!("{version} received, starting the update…"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_state_compares_semver() {
        let pkg = Version::parse("0.2.0").unwrap();
        assert!(update_state("0.1.0", &pkg) == UpdateState::Outdated);
        assert!(update_state("0.2.0", &pkg) == UpdateState::Current);
        assert!(update_state("0.10.0", &pkg) == UpdateState::Newer);
        assert!(update_state("", &pkg) == UpdateState::Unknown);
    }
}
