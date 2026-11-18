//! The agent package: the agent exe the master hands out, both as a
//! self-provisioning setup file and as a remote update.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use fleetview_shared::package::{self, Version};
use fleetview_shared::protocol::{MasterMsg, RunSpec};

use crate::state::SharedState;

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
            push_one(&state, &id, &image, version, &args, new_id);
        });
    }
    if let Ok(mut s) = state.lock() {
        s.log(format!(
            "agent update to {} started for {count} PC(s)",
            pkg.version
        ));
    }
}

fn set_note(state: &SharedState, id: &str, note: String) {
    if let Some(a) = state.lock().unwrap().agents.get_mut(id) {
        a.update_note = Some(note);
    }
}

fn push_one(
    state: &SharedState,
    id: &str,
    image: &[u8],
    version: Version,
    args: &[String],
    new_id: fn() -> String,
) {
    let command_id = new_id();
    let transfer_id = new_id();
    let tx = {
        let mut s = state.lock().unwrap();
        let Some(a) = s.agents.get_mut(id) else {
            return;
        };
        let Some(tx) = a.tx.clone() else {
            a.update_note = Some("offline — not updated".into());
            return;
        };
        a.update_note = Some(format!("sending {version}…"));
        a.update_command = Some(command_id.clone());
        tx
    };
    let total = image.len() as u64;
    let mut ok = tx
        .send(MasterMsg::FileBegin {
            transfer_id: transfer_id.clone(),
            file_name: format!("fleetview-agent-{version}.exe"),
            total_len: total,
            run_after: Some(RunSpec {
                command_id,
                args: args.to_vec(),
                is_msi: false,
            }),
        })
        .is_ok();
    for chunk in image.chunks(CHUNK) {
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
    if !ok {
        set_note(state, id, "connection lost while sending".into());
        return;
    }

    // Wait until the agent has the whole file before starting the next one,
    // so a fleet-wide update doesn't queue a copy per PC in memory.
    let deadline = Instant::now() + ACK_TIMEOUT;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let s = state.lock().unwrap();
        let Some(a) = s.agents.get(id) else { return };
        if a.last_ack.as_ref() == Some(&(transfer_id.clone(), total)) {
            break;
        }
        // Result already in (e.g. a fast failure), or the PC went away.
        if a.update_command.is_none() || !a.online {
            return;
        }
        if Instant::now() >= deadline {
            drop(s);
            set_note(
                state,
                id,
                "no confirmation from the agent (timed out)".into(),
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
