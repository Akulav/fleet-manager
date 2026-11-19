//! Shared application state between the egui UI thread and the async backend.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use fleetview_shared::protocol::{CommandResult, MasterMsg, SoftwareEntry, Telemetry};

pub type SharedState = Arc<Mutex<Shared>>;

/// Telemetry samples kept per agent for the CPU/RAM charts (30 s apart by
/// default, so about two hours).
pub const HISTORY_LEN: usize = 240;
/// Jobs kept in the Jobs view.
const MAX_JOBS: usize = 300;
/// Output kept per job target.
const JOB_OUTPUT_MAX: usize = 64_000;

/// Latest screen frame. The UI sizes textures from the decoded JPEG; the
/// reported size is kept for diagnostics.
#[allow(dead_code)]
pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct Sample {
    pub at: DateTime<Local>,
    pub cpu: f32,
    pub mem: f32,
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
    pub frame_at: Option<DateTime<Local>>,
    pub streaming: bool,
    pub tx: Option<UnboundedSender<MasterMsg>>,
    /// Version the agent reported (Hello / telemetry); kept while offline.
    pub agent_version: String,
    /// Progress of a remote update, shown in the UI.
    pub update_note: Option<String>,
    /// Command id of the pending remote update (to catch its failure).
    pub update_command: Option<String>,
    /// Bytes acknowledged per in-flight file transfer (removed by the sender).
    pub acks: HashMap<String, u64>,
    /// CPU / memory history for the charts.
    pub history: VecDeque<Sample>,
    /// Admin-assigned labels (e.g. "Accounting", "2nd floor").
    pub tags: Vec<String>,
    /// Free-form admin notes.
    pub notes: String,
    /// Last known network / OS facts, kept (and persisted) while offline so the
    /// list stays useful and Wake-on-LAN works after a master restart.
    pub last_ips: Vec<String>,
    pub last_macs: Vec<String>,
    pub last_os: String,
    pub last_user: Option<String>,
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
            frame_at: None,
            streaming: false,
            tx: None,
            agent_version: String::new(),
            update_note: None,
            update_command: None,
            acks: HashMap::new(),
            history: VecDeque::new(),
            tags: Vec::new(),
            notes: String::new(),
            last_ips: Vec::new(),
            last_macs: Vec::new(),
            last_os: String::new(),
            last_user: None,
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
        a.tags = known.tags.clone();
        a.notes = known.notes.clone();
        a.last_ips = known.ips.clone();
        a.last_macs = known.macs.clone();
        a.last_os = known.os.clone();
        a.last_user = known.user.clone();
        a
    }

    /// Queue a message for the agent. False when it has no live connection.
    pub fn send(&self, msg: MasterMsg) -> bool {
        match &self.tx {
            Some(tx) => tx.send(msg).is_ok(),
            None => false,
        }
    }

    /// Record a telemetry snapshot (history + cached facts).
    pub fn apply_telemetry(&mut self, t: Telemetry, now: DateTime<Local>) {
        self.hostname = t.hostname.clone();
        self.agent_version = t.agent_version.clone();
        self.last_ips = t.ip_addresses.clone();
        if !t.mac_addresses.is_empty() {
            self.last_macs = t.mac_addresses.clone();
        }
        self.last_os = t.os_version.clone();
        self.last_user = t.logged_in_user.clone();
        let mem = 100.0 * t.mem_used_mb as f32 / t.mem_total_mb.max(1) as f32;
        self.history.push_back(Sample {
            at: now,
            cpu: t.cpu_usage_pct,
            mem,
        });
        while self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        self.telemetry = Some(t);
    }

    pub fn mem_pct(&self) -> Option<f32> {
        self.telemetry
            .as_ref()
            .map(|t| 100.0 * t.mem_used_mb as f32 / t.mem_total_mb.max(1) as f32)
    }

    /// Used % of the fullest disk, with its mount.
    pub fn worst_disk(&self) -> Option<(String, f32)> {
        self.telemetry.as_ref().and_then(|t| {
            t.disks
                .iter()
                .filter(|d| d.total_gb > 0.0)
                .map(|d| {
                    (
                        d.mount.clone(),
                        (100.0 * (d.total_gb - d.available_gb) / d.total_gb) as f32,
                    )
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
        })
    }
}

// ---------------------------------------------------------------------------
// Jobs: every action sent to one or more PCs, with a per-PC result.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobKind {
    Script,
    Install,
    Uninstall,
    Update,
    Config,
    Notify,
    Power,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Script => "Script",
            JobKind::Install => "Install",
            JobKind::Uninstall => "Uninstall",
            JobKind::Update => "Agent update",
            JobKind::Config => "Config",
            JobKind::Notify => "Message",
            JobKind::Power => "Power",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TargetStatus {
    /// Sent, waiting for the agent's result.
    Pending,
    Succeeded,
    Failed,
    /// Never sent (offline, or nothing to do on that PC).
    Skipped,
}

#[derive(Clone)]
pub struct JobTarget {
    pub agent_id: String,
    pub hostname: String,
    pub command_id: Option<String>,
    pub status: TargetStatus,
    pub exit_code: Option<i32>,
    pub output: String,
    pub finished: Option<DateTime<Local>>,
}

#[derive(Clone)]
pub struct Job {
    pub id: u64,
    pub kind: JobKind,
    pub title: String,
    /// The script / URL / file behind the job, for "run again".
    pub detail: String,
    pub created: DateTime<Local>,
    pub targets: Vec<JobTarget>,
}

impl Job {
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut c = (0, 0, 0, 0);
        for t in &self.targets {
            match t.status {
                TargetStatus::Pending => c.0 += 1,
                TargetStatus::Succeeded => c.1 += 1,
                TargetStatus::Failed => c.2 += 1,
                TargetStatus::Skipped => c.3 += 1,
            }
        }
        c
    }

    pub fn is_running(&self) -> bool {
        self.targets
            .iter()
            .any(|t| t.status == TargetStatus::Pending)
    }
}

pub struct Shared {
    pub agents: BTreeMap<String, AgentView>,
    pub events: VecDeque<(DateTime<Local>, String)>,
    pub master_name: String,
    pub listening: bool,
    pub listen_error: Option<String>,
    pub jobs: VecDeque<Job>,
    next_job_id: u64,
    /// command_id -> (job id, agent id)
    command_jobs: HashMap<String, (u64, String)>,
}

impl Shared {
    pub fn new(master_name: String) -> Self {
        Self {
            agents: BTreeMap::new(),
            events: VecDeque::new(),
            master_name,
            listening: false,
            listen_error: None,
            jobs: VecDeque::new(),
            next_job_id: 1,
            command_jobs: HashMap::new(),
        }
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::info!("{msg}");
        self.events.push_front((Local::now(), msg));
        while self.events.len() > 1000 {
            self.events.pop_back();
        }
    }

    pub fn hostname_of(&self, id: &str) -> String {
        self.agents
            .get(id)
            .map(|a| a.hostname.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// Create a job with every target pending. Returns its id.
    pub fn create_job(
        &mut self,
        kind: JobKind,
        title: impl Into<String>,
        detail: impl Into<String>,
        agent_ids: &[String],
    ) -> u64 {
        let id = self.next_job_id;
        self.next_job_id += 1;
        let targets = agent_ids
            .iter()
            .map(|a| JobTarget {
                agent_id: a.clone(),
                hostname: self.hostname_of(a),
                command_id: None,
                status: TargetStatus::Pending,
                exit_code: None,
                output: String::new(),
                finished: None,
            })
            .collect();
        self.jobs.push_front(Job {
            id,
            kind,
            title: title.into(),
            detail: detail.into(),
            created: Local::now(),
            targets,
        });
        while self.jobs.len() > MAX_JOBS {
            if let Some(old) = self.jobs.pop_back() {
                for t in old.targets {
                    if let Some(c) = t.command_id {
                        self.command_jobs.remove(&c);
                    }
                }
            }
        }
        id
    }

    fn target_mut(&mut self, job: u64, agent: &str) -> Option<&mut JobTarget> {
        self.jobs
            .iter_mut()
            .find(|j| j.id == job)
            .and_then(|j| j.targets.iter_mut().find(|t| t.agent_id == agent))
    }

    /// Tie a command id to a job target so its result lands there.
    pub fn attach_command(&mut self, job: u64, agent: &str, command_id: &str) {
        if let Some(t) = self.target_mut(job, agent) {
            t.command_id = Some(command_id.to_string());
            self.command_jobs
                .insert(command_id.to_string(), (job, agent.to_string()));
        }
    }

    /// Finish a job target without a command result (offline, delivered, ...).
    pub fn finish_target(&mut self, job: u64, agent: &str, status: TargetStatus, note: &str) {
        if let Some(t) = self.target_mut(job, agent) {
            if t.status == TargetStatus::Pending {
                t.status = status;
                t.output = note.to_string();
                t.finished = Some(Local::now());
            }
        }
    }

    /// Route a command result to its job, if it belongs to one.
    pub fn complete_command(&mut self, r: &CommandResult) {
        let Some((job, agent)) = self.command_jobs.remove(&r.command_id) else {
            return;
        };
        if let Some(t) = self.target_mut(job, &agent) {
            t.status = if r.success {
                TargetStatus::Succeeded
            } else {
                TargetStatus::Failed
            };
            t.exit_code = r.exit_code;
            let mut out = r.stdout.clone();
            if !r.stderr.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("[stderr]\n");
                out.push_str(&r.stderr);
            }
            if out.len() > JOB_OUTPUT_MAX {
                let mut cut = JOB_OUTPUT_MAX;
                while !out.is_char_boundary(cut) {
                    cut -= 1;
                }
                out.truncate(cut);
                out.push_str("\n… (truncated)");
            }
            t.output = out;
            t.finished = Some(Local::now());
        }
    }

    /// The agent disconnected: its pending results will never come.
    pub fn fail_pending_for(&mut self, agent: &str) {
        let mut dropped = Vec::new();
        for j in self.jobs.iter_mut() {
            // An agent update restarts the agent on purpose.
            if j.kind == JobKind::Update {
                continue;
            }
            for t in j.targets.iter_mut() {
                if t.agent_id == agent && t.status == TargetStatus::Pending {
                    t.status = TargetStatus::Failed;
                    t.output = "connection lost before a result arrived".into();
                    t.finished = Some(Local::now());
                    if let Some(c) = &t.command_id {
                        dropped.push(c.clone());
                    }
                }
            }
        }
        for c in dropped {
            self.command_jobs.remove(&c);
        }
    }

    pub fn running_jobs(&self) -> usize {
        self.jobs.iter().filter(|j| j.is_running()).count()
    }
}

/// Persisted record of every agent the master has ever seen (for the
/// "online vs. ever seen" metric, tags/notes and Wake-on-LAN).
#[derive(Clone, Serialize, Deserialize)]
pub struct KnownAgent {
    pub agent_id: String,
    pub hostname: String,
    pub first_seen: DateTime<Local>,
    pub last_seen: DateTime<Local>,
    #[serde(default)]
    pub agent_version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub ips: Vec<String>,
    #[serde(default)]
    pub macs: Vec<String>,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub user: Option<String>,
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
            tags: a.tags.clone(),
            notes: a.notes.clone(),
            ips: a.last_ips.clone(),
            macs: a.last_macs.clone(),
            os: a.last_os.clone(),
            user: a.last_user.clone(),
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&list) {
        let _ = crate::paths::ensure_data_dir();
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
    use super::*;

    #[test]
    fn push_bounded_never_splits_chars() {
        let mut s = String::new();
        for _ in 0..1000 {
            push_bounded(&mut s, "ăîșțâ", 101);
            assert!(s.len() <= 101);
        }
    }

    #[test]
    fn job_results_route_by_command_id() {
        let mut s = Shared::new("m".into());
        let ids = vec!["a".to_string(), "b".to_string()];
        let job = s.create_job(JobKind::Script, "t", "d", &ids);
        s.attach_command(job, "a", "c1");
        s.finish_target(job, "b", TargetStatus::Skipped, "offline");
        assert!(s.jobs[0].is_running());
        s.complete_command(&CommandResult {
            command_id: "c1".into(),
            success: true,
            exit_code: Some(0),
            stdout: "hi".into(),
            stderr: String::new(),
        });
        let j = &s.jobs[0];
        assert!(!j.is_running());
        assert_eq!(j.counts(), (0, 1, 0, 1));
        assert_eq!(j.targets[0].output, "hi");
    }

    #[test]
    fn disconnect_fails_pending_targets() {
        let mut s = Shared::new("m".into());
        let ids = vec!["a".to_string()];
        let job = s.create_job(JobKind::Script, "t", "d", &ids);
        s.attach_command(job, "a", "c1");
        s.fail_pending_for("a");
        assert_eq!(s.jobs[0].targets[0].status, TargetStatus::Failed);
    }
}
