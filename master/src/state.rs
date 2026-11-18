//! Shared application state between the egui UI thread and the async backend.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use fleetview_shared::protocol::{MasterMsg, SoftwareEntry, Telemetry};

pub type SharedState = Arc<Mutex<Shared>>;

pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

pub struct AgentView {
    pub agent_id: String,
    pub hostname: String,
    pub online: bool,
    pub first_seen: DateTime<Local>,
    pub last_seen: DateTime<Local>,
    pub addr: Option<SocketAddr>,
    pub telemetry: Option<Telemetry>,
    pub software: Vec<SoftwareEntry>,
    pub console_output: String,
    pub last_frame: Option<ScreenFrame>,
    pub frame_seq: u64,
    pub streaming: bool,
    pub tx: Option<UnboundedSender<MasterMsg>>,
    /// Version the agent reported (Hello / telemetry); kept while offline.
    pub agent_version: String,
    /// Progress of a remote update, shown in the UI.
    pub update_note: Option<String>,
    /// Command id of the pending remote update (to catch its failure).
    pub update_command: Option<String>,
    /// Last file-transfer acknowledgement: (transfer id, bytes received).
    pub last_ack: Option<(String, u64)>,
}

impl AgentView {
    pub fn new(agent_id: String, hostname: String, now: DateTime<Local>) -> Self {
        Self {
            agent_id,
            hostname,
            online: false,
            first_seen: now,
            last_seen: now,
            addr: None,
            telemetry: None,
            software: Vec::new(),
            console_output: String::new(),
            last_frame: None,
            frame_seq: 0,
            streaming: false,
            tx: None,
            agent_version: String::new(),
            update_note: None,
            update_command: None,
            last_ack: None,
        }
    }

    pub fn offline_from(known: &KnownAgent) -> Self {
        let mut a = Self::new(
            known.agent_id.clone(),
            known.hostname.clone(),
            known.first_seen,
        );
        a.last_seen = known.last_seen;
        a.agent_version = known.agent_version.clone();
        a
    }

    pub fn send(&self, msg: MasterMsg) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(msg);
        }
    }
}

pub struct Shared {
    pub agents: BTreeMap<String, AgentView>,
    pub events: VecDeque<String>,
    pub master_name: String,
    pub listening: bool,
    pub listen_error: Option<String>,
}

impl Shared {
    pub fn new(master_name: String) -> Self {
        Self {
            agents: BTreeMap::new(),
            events: VecDeque::new(),
            master_name,
            listening: false,
            listen_error: None,
        }
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        let line = format!("{}  {}", Local::now().format("%H:%M:%S"), msg.into());
        tracing::info!("{line}");
        self.events.push_front(line);
        while self.events.len() > 500 {
            self.events.pop_back();
        }
    }

    pub fn online_count(&self) -> usize {
        self.agents.values().filter(|a| a.online).count()
    }

    pub fn total_ever_seen(&self) -> usize {
        self.agents.len()
    }
}

/// Persisted record of every agent the master has ever seen (for the
/// "online vs. ever seen" metric).
#[derive(Clone, Serialize, Deserialize)]
pub struct KnownAgent {
    pub agent_id: String,
    pub hostname: String,
    pub first_seen: DateTime<Local>,
    pub last_seen: DateTime<Local>,
    #[serde(default)]
    pub agent_version: String,
}

pub fn load_known() -> Vec<KnownAgent> {
    std::fs::read_to_string(crate::paths::known_agents_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_known(state: &Shared) {
    let list: Vec<KnownAgent> = state
        .agents
        .values()
        .map(|a| KnownAgent {
            agent_id: a.agent_id.clone(),
            hostname: a.hostname.clone(),
            first_seen: a.first_seen,
            last_seen: a.last_seen,
            agent_version: a.agent_version.clone(),
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&list) {
        let path = crate::paths::known_agents_path();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Append to a bounded text buffer, trimming from the front on a UTF-8 char
/// boundary (a byte-index `split_off` panics in the middle of a multi-byte
/// character such as Romanian diacritics).
pub fn push_bounded(buf: &mut String, text: &str, max: usize) {
    buf.push_str(text);
    if buf.len() > max {
        let mut cut = buf.len() - max;
        while !buf.is_char_boundary(cut) {
            cut += 1;
        }
        buf.drain(..cut);
    }
}

#[cfg(test)]
mod tests {
    use super::push_bounded;

    #[test]
    fn push_bounded_never_splits_chars() {
        let mut s = String::new();
        for _ in 0..1000 {
            push_bounded(&mut s, "ăîșțâ", 101);
            assert!(s.len() <= 101);
        }
    }
}
