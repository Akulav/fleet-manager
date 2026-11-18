//! egui desktop UI for the master. English only (per requirement).

use std::collections::HashMap;

use eframe::egui;

use fleetview_shared::protocol::{ConfigUpdate, MasterMsg, RunSpec};

use crate::agentpkg::{self, AgentPackage, UpdateState};
use crate::settings::MasterSettings;
use crate::state::SharedState;

const GREEN: egui::Color32 = egui::Color32::from_rgb(40, 180, 120);
const AMBER: egui::Color32 = egui::Color32::from_rgb(220, 160, 40);

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Overview,
    Software,
    PowerShell,
    Screen,
}

pub struct App {
    state: SharedState,
    settings: MasterSettings,
    selected: Option<String>, // None => fleet overview
    tab: Tab,

    // per-agent screen textures: agent_id -> (frame_seq, texture)
    textures: HashMap<String, (u64, egui::TextureHandle)>,

    // input fields
    ps_input: String,
    ps_run_all: bool,
    notify_title: String,
    notify_body: String,
    software_filter: String,
    install_args: String,
    install_is_msi: bool,
    url_input: String,
    stream_fps: u32,

    // "Run on ALL" awaiting confirmation: (script, target agent ids)
    pending_run_all: Option<(String, Vec<String>)>,

    // agent-config export window
    show_config: bool,
    config_text: String,
    config_master_addr: String,
    emanagement_input: String,
    config_status: String,

    // agent package used for setups and remote updates
    package: Option<AgentPackage>,
    package_error: String,
    // "Update agents" awaiting confirmation: target agent ids
    pending_update: Option<Vec<String>>,

    // Remote config editor
    show_remote_cfg: bool,
    rc_scope_all: bool,
    rc: [ConfigField; 5],
    rc_status: String,
    // "Install on ALL online" awaiting confirmation: (file path or URL, is_url)
    pending_install_all: Option<(String, bool)>,
}

/// One row in the remote-config editor: a key, whether to apply it, and its
/// value. The key names match the agent's `REMOTE_SETTABLE_KEYS`.
#[derive(Clone)]
struct ConfigField {
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    apply: bool,
    value: String,
}

fn default_config_fields() -> [ConfigField; 5] {
    let f = |key, label, hint| ConfigField {
        key,
        label,
        hint,
        apply: false,
        value: String::new(),
    };
    [
        f(
            "emanagement_url",
            "e-Management URL",
            "http://docs.ansc.local:3080 — turns on the MoldSign bridge; empty = off",
        ),
        f(
            "master_address",
            "Master address",
            "host:port to skip discovery; empty = auto-discover",
        ),
        f(
            "discovery_targets",
            "Discovery targets",
            "extra broadcast addresses, comma-separated, e.g. 192.168.20.255:48621",
        ),
        f(
            "telemetry_interval_secs",
            "Telemetry interval (s)",
            "seconds between telemetry reports, e.g. 30",
        ),
        f(
            "moldsign_path",
            "MoldSign path",
            "override SignDesktop.exe location; empty = auto",
        ),
    ]
}

impl App {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        state: SharedState,
        settings: MasterSettings,
    ) -> Self {
        let emanagement_input = settings.emanagement_url.clone();
        let mut app = Self {
            state,
            settings,
            selected: None,
            tab: Tab::Overview,
            textures: HashMap::new(),
            ps_input: String::new(),
            ps_run_all: false,
            notify_title: "Message from IT".into(),
            notify_body: String::new(),
            software_filter: String::new(),
            install_args: "/S".into(),
            install_is_msi: false,
            url_input: String::new(),
            stream_fps: 2,
            pending_run_all: None,
            show_config: false,
            config_text: String::new(),
            config_master_addr: String::new(),
            emanagement_input,
            config_status: String::new(),
            package: None,
            package_error: String::new(),
            pending_update: None,
            show_remote_cfg: false,
            rc_scope_all: true,
            rc: default_config_fields(),
            rc_status: String::new(),
            pending_install_all: None,
        };
        app.load_package();
        app
    }

    /// (Re)load the agent package from the configured path or the default.
    fn load_package(&mut self) {
        let path = if self.settings.agent_package_path.trim().is_empty() {
            AgentPackage::default_path()
        } else {
            std::path::PathBuf::from(self.settings.agent_package_path.trim())
        };
        match AgentPackage::load(&path) {
            Ok(p) => {
                self.package = Some(p);
                self.package_error.clear();
            }
            Err(e) => {
                self.package = None;
                self.package_error = format!("{e:#}");
            }
        }
    }

    fn choose_package(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("FleetView agent", &["exe"])
            .pick_file()
        else {
            return;
        };
        self.settings.agent_package_path = path.display().to_string();
        let _ = self.settings.save();
        self.load_package();
    }

    fn update_state_of(&self, agent_version: &str) -> UpdateState {
        match &self.package {
            Some(p) => agentpkg::update_state(agent_version, &p.version),
            None => UpdateState::Unknown,
        }
    }

    /// Online agents older than the package.
    fn outdated_online(&self) -> Vec<String> {
        let Some(p) = &self.package else {
            return Vec::new();
        };
        let s = self.state.lock().unwrap();
        s.agents
            .values()
            .filter(|a| a.online && a.update_command.is_none())
            .filter(|a| {
                // 0.1.x agents report a version too, so "Unknown" here means
                // no Hello yet: skip until they say who they are.
                agentpkg::update_state(&a.agent_version, &p.version) == UpdateState::Outdated
            })
            .map(|a| a.agent_id.clone())
            .collect()
    }

    /// Online agents whose version is not newer than the package (so an update
    /// or repair would not be a downgrade), and not already updating.
    fn updatable_online(&self) -> Vec<String> {
        let Some(p) = &self.package else {
            return Vec::new();
        };
        let s = self.state.lock().unwrap();
        s.agents
            .values()
            .filter(|a| a.online && a.update_command.is_none())
            .filter(|a| agentpkg::update_state(&a.agent_version, &p.version) != UpdateState::Newer)
            .map(|a| a.agent_id.clone())
            .collect()
    }

    fn update_agents(&mut self, ids: Vec<String>) {
        if let Some(p) = &self.package {
            agentpkg::start_update(
                self.state.clone(),
                p,
                ids,
                &self.settings.emanagement_url,
                Self::new_command_id,
            );
        }
    }

    /// Agent ids of every online agent.
    fn online_ids(&self) -> Vec<String> {
        let s = self.state.lock().unwrap();
        s.agents
            .values()
            .filter(|a| a.online)
            .map(|a| a.agent_id.clone())
            .collect()
    }

    /// Send a remote config change to each listed agent.
    fn set_config_on(&self, ids: &[String], updates: Vec<ConfigUpdate>) {
        if updates.is_empty() || ids.is_empty() {
            return;
        }
        let keys: Vec<&str> = updates.iter().map(|u| u.key.as_str()).collect();
        for id in ids {
            self.send_to(
                id,
                MasterMsg::SetConfig {
                    command_id: Self::new_command_id(),
                    updates: updates.clone(),
                },
            );
        }
        if let Ok(mut s) = self.state.lock() {
            s.log(format!(
                "remote config ({}) sent to {} agent(s)",
                keys.join(", "),
                ids.len()
            ));
        }
    }

    /// Stream a local installer/file to each listed agent and run it. The file
    /// is read once; each agent gets its own transfer, a few at a time.
    fn push_file_to(&self, path: std::path::PathBuf, ids: Vec<String>) {
        let args = split_args(&self.install_args);
        let is_msi = self.install_is_msi
            || path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("msi"));
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "installer.exe".into());
        let state = self.state.clone();
        std::thread::spawn(move || {
            let data = match std::fs::read(&path) {
                Ok(d) => std::sync::Arc::new(d),
                Err(e) => {
                    if let Ok(mut s) = state.lock() {
                        s.log(format!("cannot read {}: {e}", path.display()));
                    }
                    return;
                }
            };
            let queue =
                std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from(ids)));
            let mut workers = Vec::new();
            for _ in 0..4 {
                let (queue, state, data, file_name, args) = (
                    queue.clone(),
                    state.clone(),
                    data.clone(),
                    file_name.clone(),
                    args.clone(),
                );
                workers.push(std::thread::spawn(move || loop {
                    let Some(id) = queue.lock().unwrap().pop_front() else {
                        break;
                    };
                    let run = RunSpec {
                        command_id: App::new_command_id(),
                        args: args.clone(),
                        is_msi,
                    };
                    // Waits for the agent's acks before taking the next PC, so
                    // memory stays bounded however big the fleet is.
                    let outcome = agentpkg::send_file_and_wait(
                        &state,
                        &id,
                        App::new_command_id(),
                        file_name.clone(),
                        &data,
                        Some(run),
                    );
                    if outcome != agentpkg::SendOutcome::Delivered {
                        if let Ok(mut s) = state.lock() {
                            let host = s
                                .agents
                                .get(&id)
                                .map(|a| a.hostname.clone())
                                .unwrap_or(id.clone());
                            s.log(format!("{host}: {file_name} not delivered ({outcome:?})"));
                        }
                    }
                }));
            }
            for w in workers {
                let _ = w.join();
            }
            if let Ok(mut s) = state.lock() {
                s.log(format!("installer {file_name} pushed to the fleet"));
            }
        });
    }

    fn install_url_on(&self, url: String, ids: &[String]) {
        let is_msi = self.install_is_msi;
        let args = split_args(&self.install_args);
        for id in ids {
            let run = RunSpec {
                command_id: Self::new_command_id(),
                args: args.clone(),
                is_msi,
            };
            self.send_to(
                id,
                MasterMsg::InstallFromUrl {
                    command_id: run.command_id.clone(),
                    url: url.clone(),
                    run,
                },
            );
        }
        if let Ok(mut s) = self.state.lock() {
            s.log(format!("install-from-URL sent to {} agent(s)", ids.len()));
        }
    }

    /// Unique id per command. A bare millisecond timestamp collided when one
    /// click sent several messages (e.g. "Run on ALL", or a file push).
    fn new_command_id() -> String {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{}-{n}", chrono::Local::now().timestamp_millis())
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keep the UI live for telemetry/screen updates.
        ctx.request_repaint_after(std::time::Duration::from_millis(300));

        self.top_bar(ctx);
        self.left_panel(ctx);
        self.central(ctx);
        if self.show_config {
            self.config_window(ctx);
        }
        if self.pending_run_all.is_some() {
            self.confirm_run_all_window(ctx);
        }
        if self.pending_update.is_some() {
            self.confirm_update_window(ctx);
        }
        if self.pending_install_all.is_some() {
            self.confirm_install_all_window(ctx);
        }
        if self.show_remote_cfg {
            self.remote_config_window(ctx);
        }
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("FleetView");
                ui.label(
                    egui::RichText::new(format!("v{} · by Akulav", env!("CARGO_PKG_VERSION")))
                        .weak(),
                );
                ui.separator();
                let (online, total, listening, err, name) = {
                    let s = self.state.lock().unwrap();
                    (
                        s.online_count(),
                        s.total_ever_seen(),
                        s.listening,
                        s.listen_error.clone(),
                        s.master_name.clone(),
                    )
                };
                ui.label(name.to_string());
                ui.separator();
                if listening {
                    ui.colored_label(egui::Color32::from_rgb(40, 180, 120), "● listening");
                } else {
                    ui.colored_label(egui::Color32::from_rgb(200, 80, 80), "● not listening");
                }
                ui.separator();
                ui.strong(format!("Online: {online}"));
                ui.label(format!("/ ever seen: {total}"));
                if let Some(e) = err {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(200, 80, 80), e);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Agent installer").clicked() {
                        self.config_text = self
                            .settings
                            .agent_config_json(None)
                            .unwrap_or_else(|e| format!("error: {e}"));
                        self.config_status.clear();
                        self.show_config = true;
                    }
                });
            });
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("agents")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                if ui
                    .selectable_label(self.selected.is_none(), "🖥  Fleet overview")
                    .clicked()
                {
                    self.selected = None;
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let agents: Vec<(String, String, bool, f32, String)> = {
                        let s = self.state.lock().unwrap();
                        s.agents
                            .values()
                            .map(|a| {
                                let cpu =
                                    a.telemetry.as_ref().map(|t| t.cpu_usage_pct).unwrap_or(0.0);
                                (
                                    a.agent_id.clone(),
                                    a.hostname.clone(),
                                    a.online,
                                    cpu,
                                    a.agent_version.clone(),
                                )
                            })
                            .collect()
                    };
                    for (id, host, online, cpu, version) in agents {
                        let dot = if online { "🟢" } else { "⚪" };
                        let outdated = self.update_state_of(&version) == UpdateState::Outdated;
                        let mark = if outdated { "  ⬆" } else { "" };
                        let label = if online {
                            format!("{dot} {host}  ({cpu:.0}%){mark}")
                        } else {
                            format!("{dot} {host}{mark}")
                        };
                        if ui
                            .selectable_label(self.selected.as_deref() == Some(&id), label)
                            .clicked()
                        {
                            self.selected = Some(id.clone());
                        }
                    }
                });
            });
    }

    fn central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| match self.selected.clone() {
            None => self.fleet_overview(ui),
            Some(id) => self.agent_view(ui, &id),
        });
    }

    fn fleet_overview(&mut self, ui: &mut egui::Ui) {
        ui.heading("Fleet overview");
        ui.add_space(6.0);
        self.updates_panel(ui);
        self.fleet_actions_panel(ui);
        ui.add_space(6.0);
        let rows: Vec<([String; 5], String)> = {
            let s = self.state.lock().unwrap();
            s.agents
                .values()
                .map(|a| {
                    let ips = a
                        .telemetry
                        .as_ref()
                        .map(|t| t.ip_addresses.join(", "))
                        .unwrap_or_default();
                    let user = a
                        .telemetry
                        .as_ref()
                        .and_then(|t| t.logged_in_user.clone())
                        .unwrap_or_default();
                    (
                        [
                            format!("{} {}", if a.online { "🟢" } else { "⚪" }, a.hostname),
                            user,
                            ips,
                            a.last_seen.format("%Y-%m-%d %H:%M").to_string(),
                            a.telemetry
                                .as_ref()
                                .map(|t| format!("{:.0}% CPU", t.cpu_usage_pct))
                                .unwrap_or_default(),
                        ],
                        a.agent_version.clone(),
                    )
                })
                .collect()
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("fleet_grid")
                .striped(true)
                .num_columns(6)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Host");
                    ui.strong("User");
                    ui.strong("IP addresses");
                    ui.strong("Last seen");
                    ui.strong("CPU");
                    ui.strong("Agent");
                    ui.end_row();
                    for (r, version) in &rows {
                        for cell in r {
                            ui.label(cell);
                        }
                        self.version_label(ui, version);
                        ui.end_row();
                    }
                });
            ui.add_space(12.0);
            ui.collapsing("Events", |ui| {
                let events: Vec<String> =
                    self.state.lock().unwrap().events.iter().cloned().collect();
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for e in events {
                            ui.monospace(e);
                        }
                    });
            });
        });
    }

    fn version_label(&self, ui: &mut egui::Ui, version: &str) {
        let text = if version.is_empty() { "—" } else { version };
        match self.update_state_of(version) {
            UpdateState::Outdated => {
                ui.colored_label(AMBER, format!("{text} ⬆"))
                    .on_hover_text("Older than the agent package: can be updated");
            }
            UpdateState::Current => {
                ui.colored_label(GREEN, text);
            }
            _ => {
                ui.label(text);
            }
        }
    }

    /// Agent package + fleet-wide update controls.
    fn updates_panel(&mut self, ui: &mut egui::Ui) {
        let outdated = self.outdated_online();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Agent updates");
                ui.separator();
                match &self.package {
                    Some(p) => {
                        ui.label(format!("Package: agent {}", p.version))
                            .on_hover_text(p.path.display().to_string());
                    }
                    None => {
                        ui.colored_label(AMBER, "No agent package")
                            .on_hover_text(&self.package_error);
                    }
                }
                if ui.button("Choose agent exe…").clicked() {
                    self.choose_package();
                }
                if ui
                    .button("↻")
                    .on_hover_text("Reload the package file")
                    .clicked()
                {
                    self.load_package();
                }
                ui.separator();
                let n = outdated.len();
                let btn = ui.add_enabled(
                    n > 0,
                    egui::Button::new(format!("⬆ Update {n} outdated online PC(s)")),
                );
                if btn.clicked() {
                    self.pending_update = Some(outdated.clone());
                }
                // Update / reinstall every online agent that isn't newer than
                // the package (never a downgrade).
                let all = self.updatable_online();
                let m = all.len();
                if ui
                    .add_enabled(
                        self.package.is_some() && m > 0,
                        egui::Button::new(format!("⟳ Update/repair all {m} online")),
                    )
                    .on_hover_text("Push the package to every online PC (skips newer ones)")
                    .clicked()
                {
                    self.pending_update = Some(all);
                }
            });
        });
    }

    /// Fleet-wide actions: push an installer / URL to every online PC, and open
    /// the remote-config editor. (Updates are in `updates_panel` above.)
    fn fleet_actions_panel(&mut self, ui: &mut egui::Ui) {
        let online = self.online_ids();
        let n = online.len();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Fleet actions");
                ui.separator();
                ui.label(format!("{n} online"));
                ui.separator();
                if ui
                    .add_enabled(n > 0, egui::Button::new("⬇ Install file on all…"))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        self.pending_install_all = Some((path.display().to_string(), false));
                    }
                }
                ui.checkbox(&mut self.install_is_msi, "MSI");
                ui.label("args:");
                ui.add(egui::TextEdit::singleline(&mut self.install_args).desired_width(90.0));
                ui.separator();
                ui.label("URL:");
                ui.add(egui::TextEdit::singleline(&mut self.url_input).desired_width(200.0));
                let url_ok = self.url_input.trim().to_lowercase().starts_with("https://");
                if ui
                    .add_enabled(n > 0 && url_ok, egui::Button::new("⬇ URL on all…"))
                    .on_disabled_hover_text("Enter an https:// URL")
                    .clicked()
                {
                    self.pending_install_all = Some((self.url_input.trim().to_string(), true));
                }
                ui.separator();
                if ui.button("⚙ Remote config…").clicked() {
                    self.rc_status.clear();
                    self.show_remote_cfg = true;
                }
            });
        });
    }

    fn confirm_install_all_window(&mut self, ctx: &egui::Context) {
        let Some((target, is_url)) = self.pending_install_all.clone() else {
            return;
        };
        let ids = self.online_ids();
        let mut decision: Option<bool> = None;
        egui::Window::new("Install on all online PCs?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Run this on {} online PC(s) as SYSTEM{}:",
                    ids.len(),
                    if self.install_is_msi { " (MSI)" } else { "" }
                ));
                ui.add_space(4.0);
                ui.monospace(&target);
                ui.label(format!("Silent args: {}", self.install_args));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(format!("Install on {} PC(s)", ids.len()))
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.pending_install_all = None;
                if is_url {
                    self.install_url_on(target, &ids);
                } else {
                    self.push_file_to(std::path::PathBuf::from(target), ids);
                }
            }
            Some(false) => self.pending_install_all = None,
            None => {}
        }
    }

    fn remote_config_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_remote_cfg;
        let selected = self.selected.clone();
        egui::Window::new("Remote config")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    "Change agent config on running PCs. The agent applies only these keys \
                     (never its master identity or enrollment secret) and restarts to pick them \
                     up. An empty value clears the key.",
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.rc_scope_all, true, "All online PCs");
                    let one = selected.is_some();
                    ui.add_enabled_ui(one, |ui| {
                        ui.radio_value(&mut self.rc_scope_all, false, "Selected PC only");
                    });
                    if !one {
                        self.rc_scope_all = true;
                    }
                });
                ui.separator();
                egui::Grid::new("rc_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        for f in self.rc.iter_mut() {
                            ui.checkbox(&mut f.apply, f.label);
                            ui.add(
                                egui::TextEdit::singleline(&mut f.value)
                                    .desired_width(320.0)
                                    .hint_text(f.hint),
                            );
                            ui.end_row();
                        }
                    });
                ui.add_space(6.0);
                let updates: Vec<ConfigUpdate> = self
                    .rc
                    .iter()
                    .filter(|f| f.apply)
                    .map(|f| ConfigUpdate {
                        key: f.key.to_string(),
                        value: {
                            let v = f.value.trim();
                            if v.is_empty() {
                                None
                            } else {
                                Some(v.to_string())
                            }
                        },
                    })
                    .collect();
                let ids = if self.rc_scope_all {
                    self.online_ids()
                } else {
                    selected.clone().into_iter().collect()
                };
                ui.horizontal(|ui| {
                    let can = !updates.is_empty() && !ids.is_empty();
                    if ui
                        .add_enabled(
                            can,
                            egui::Button::new(format!("Apply to {} PC(s)", ids.len())),
                        )
                        .clicked()
                    {
                        let n = updates.len();
                        self.set_config_on(&ids, updates);
                        self.rc_status = format!(
                            "Sent {n} key(s) to {} PC(s). Watch the Events log.",
                            ids.len()
                        );
                    }
                    if !self.rc_status.is_empty() {
                        ui.label(&self.rc_status);
                    }
                });
            });
        self.show_remote_cfg = open;
    }

    fn confirm_update_window(&mut self, ctx: &egui::Context) {
        let Some(ids) = self.pending_update.clone() else {
            return;
        };
        let version = self
            .package
            .as_ref()
            .map(|p| p.version.to_string())
            .unwrap_or_default();
        let hosts: Vec<String> = {
            let s = self.state.lock().unwrap();
            ids.iter()
                .filter_map(|id| s.agents.get(id))
                .map(|a| format!("{} ({})", a.hostname, a.agent_version))
                .collect()
        };
        let mut decision: Option<bool> = None;
        egui::Window::new("Update agents?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Install agent {version} on {} PC(s). Each agent restarts (a few seconds \
                     offline) and rolls back by itself if the new version doesn't start.",
                    ids.len()
                ));
                if !self.settings.emanagement_url.trim().is_empty() {
                    ui.label(format!(
                        "MoldSign bridge: e-Management URL set to {}",
                        self.settings.emanagement_url.trim()
                    ));
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for h in &hosts {
                            ui.label(h);
                        }
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(format!("Update {} PC(s)", ids.len())).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.pending_update = None;
                self.update_agents(ids);
            }
            Some(false) => self.pending_update = None,
            None => {}
        }
    }

    fn agent_view(&mut self, ui: &mut egui::Ui, id: &str) {
        let (hostname, online) = {
            let s = self.state.lock().unwrap();
            match s.agents.get(id) {
                Some(a) => (a.hostname.clone(), a.online),
                None => (id.to_string(), false),
            }
        };
        ui.horizontal(|ui| {
            ui.heading(&hostname);
            if online {
                ui.colored_label(egui::Color32::from_rgb(40, 180, 120), "online");
            } else {
                ui.colored_label(egui::Color32::GRAY, "offline");
            }
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Overview, "Overview");
            ui.selectable_value(&mut self.tab, Tab::Software, "Software");
            ui.selectable_value(&mut self.tab, Tab::PowerShell, "PowerShell");
            ui.selectable_value(&mut self.tab, Tab::Screen, "Screen");
        });
        ui.separator();
        match self.tab {
            Tab::Overview => self.tab_overview(ui, id),
            Tab::Software => self.tab_software(ui, id),
            Tab::PowerShell => self.tab_powershell(ui, id),
            Tab::Screen => self.tab_screen(ui, id),
        }
    }

    fn send_to(&self, id: &str, msg: MasterMsg) {
        let s = self.state.lock().unwrap();
        if let Some(a) = s.agents.get(id) {
            a.send(msg);
        }
    }

    fn tab_overview(&mut self, ui: &mut egui::Ui, id: &str) {
        if ui.button("↻ Refresh telemetry").clicked() {
            self.send_to(id, MasterMsg::RequestTelemetry);
        }
        ui.add_space(6.0);
        self.agent_update_row(ui, id);
        ui.add_space(6.0);
        let t = {
            let s = self.state.lock().unwrap();
            s.agents.get(id).and_then(|a| a.telemetry.clone())
        };
        let Some(t) = t else {
            ui.label("No telemetry yet…");
            return;
        };
        egui::Grid::new("tele")
            .num_columns(2)
            .spacing([14.0, 4.0])
            .show(ui, |ui| {
                ui.strong("OS");
                ui.label(&t.os_version);
                ui.end_row();
                ui.strong("Logged-in user");
                ui.label(t.logged_in_user.clone().unwrap_or_else(|| "—".into()));
                ui.end_row();
                ui.strong("CPU");
                ui.label(format!(
                    "{} ({} cores) — {:.0}%",
                    t.cpu_name, t.cpu_cores, t.cpu_usage_pct
                ));
                ui.end_row();
                ui.strong("Memory");
                ui.label(format!(
                    "{} / {} MB ({:.0}%)",
                    t.mem_used_mb,
                    t.mem_total_mb,
                    100.0 * t.mem_used_mb as f32 / t.mem_total_mb.max(1) as f32
                ));
                ui.end_row();
                ui.strong("Uptime");
                ui.label(format!("{} h", t.uptime_secs / 3600));
                ui.end_row();
                ui.strong("IP addresses");
                ui.label(t.ip_addresses.join(", "));
                ui.end_row();
                ui.strong("MAC addresses");
                ui.label(t.mac_addresses.join(", "));
                ui.end_row();
                ui.strong("Agent version");
                ui.label(&t.agent_version);
                ui.end_row();
            });
        ui.add_space(10.0);
        ui.strong("Disks");
        for d in &t.disks {
            let used = d.total_gb - d.available_gb;
            let frac = if d.total_gb > 0.0 {
                (used / d.total_gb) as f32
            } else {
                0.0
            };
            ui.horizontal(|ui| {
                ui.label(d.mount.to_string());
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_width(220.0)
                        .text(format!("{:.0}/{:.0} GB", used, d.total_gb)),
                );
            });
        }

        ui.add_space(10.0);
        ui.separator();
        ui.strong("Notify this user");
        ui.horizontal(|ui| {
            ui.label("Title");
            ui.text_edit_singleline(&mut self.notify_title);
        });
        ui.horizontal(|ui| {
            ui.label("Body");
            ui.text_edit_singleline(&mut self.notify_body);
            if ui.button("Send").clicked() && !self.notify_body.is_empty() {
                self.send_to(
                    id,
                    MasterMsg::NotifyUser {
                        title: self.notify_title.clone(),
                        body: self.notify_body.clone(),
                    },
                );
            }
        });
    }

    /// Version of one agent + its update button and progress.
    fn agent_update_row(&mut self, ui: &mut egui::Ui, id: &str) {
        let (version, online, note, busy) = {
            let s = self.state.lock().unwrap();
            match s.agents.get(id) {
                Some(a) => (
                    a.agent_version.clone(),
                    a.online,
                    a.update_note.clone(),
                    a.update_command.is_some(),
                ),
                None => return,
            }
        };
        let state = self.update_state_of(&version);
        let pkg_version = self.package.as_ref().map(|p| p.version);
        ui.horizontal(|ui| {
            ui.strong("Agent");
            self.version_label(ui, &version);
            if let Some(pv) = pkg_version {
                let label = match state {
                    UpdateState::Outdated => format!("⬆ Update to {pv}"),
                    _ => format!("Reinstall {pv}"),
                };
                // Never downgrade from the UI.
                let enabled = online && !busy && state != UpdateState::Newer;
                if ui
                    .add_enabled(enabled, egui::Button::new(label))
                    .on_disabled_hover_text(if state == UpdateState::Newer {
                        "The agent is newer than the package"
                    } else {
                        "The agent must be online"
                    })
                    .clicked()
                {
                    self.update_agents(vec![id.to_string()]);
                }
            }
            if let Some(n) = note {
                ui.label(egui::RichText::new(n).weak());
            }
        });
    }

    fn tab_software(&mut self, ui: &mut egui::Ui, id: &str) {
        ui.horizontal(|ui| {
            if ui.button("↻ Refresh inventory").clicked() {
                self.send_to(id, MasterMsg::RequestSoftware);
            }
            ui.separator();
            if ui.button("⬆ Install file…").clicked() {
                self.push_installer(id);
            }
            ui.checkbox(&mut self.install_is_msi, "MSI");
            ui.label("Silent args:");
            ui.add(egui::TextEdit::singleline(&mut self.install_args).desired_width(120.0));
        });
        ui.horizontal(|ui| {
            ui.label("Install from URL:");
            ui.add(egui::TextEdit::singleline(&mut self.url_input).desired_width(260.0));
            let url_ok = self.url_input.trim().to_lowercase().starts_with("https://");
            if !self.url_input.trim().is_empty() && !url_ok {
                ui.colored_label(egui::Color32::from_rgb(200, 80, 80), "https:// only");
            }
            if ui
                .add_enabled(url_ok, egui::Button::new("Download & run"))
                .clicked()
            {
                let run = RunSpec {
                    command_id: Self::new_command_id(),
                    args: split_args(&self.install_args),
                    is_msi: self.install_is_msi,
                };
                self.send_to(
                    id,
                    MasterMsg::InstallFromUrl {
                        command_id: run.command_id.clone(),
                        url: self.url_input.trim().to_string(),
                        run,
                    },
                );
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut self.software_filter);
        });

        let filter = self.software_filter.to_lowercase();
        let list: Vec<(String, String, String)> = {
            let s = self.state.lock().unwrap();
            s.agents
                .get(id)
                .map(|a| {
                    a.software
                        .iter()
                        .filter(|e| filter.is_empty() || e.name.to_lowercase().contains(&filter))
                        .map(|e| {
                            (
                                e.name.clone(),
                                e.version.clone().unwrap_or_default(),
                                e.uninstall_string.clone().unwrap_or_default(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

        ui.label(format!("{} program(s)", list.len()));
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("sw")
                .striped(true)
                .num_columns(3)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Version");
                    ui.strong("");
                    ui.end_row();
                    for (name, ver, uninstall) in &list {
                        ui.label(name);
                        ui.label(ver);
                        if !uninstall.is_empty() {
                            if ui.small_button("Uninstall").clicked() {
                                let script = uninstall_script(uninstall);
                                self.send_to(
                                    id,
                                    MasterMsg::RunPowerShell {
                                        command_id: Self::new_command_id(),
                                        script,
                                    },
                                );
                            }
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
        });
    }

    fn tab_powershell(&mut self, ui: &mut egui::Ui, id: &str) {
        ui.label("Runs as SYSTEM (elevated) on the agent.");
        ui.add(
            egui::TextEdit::multiline(&mut self.ps_input)
                .desired_rows(5)
                .desired_width(f32::INFINITY)
                .code_editor()
                .hint_text("Get-Service | Where-Object Status -eq 'Running'"),
        );
        ui.horizontal(|ui| {
            if ui.button("▶ Run").clicked() && !self.ps_input.trim().is_empty() {
                let script = self.ps_input.clone();
                if self.ps_run_all {
                    // Running SYSTEM-level code on the whole fleet is never a
                    // single click: ask first (see `confirm_run_all_window`).
                    let ids: Vec<String> = {
                        let s = self.state.lock().unwrap();
                        s.agents
                            .values()
                            .filter(|a| a.online)
                            .map(|a| a.agent_id.clone())
                            .collect()
                    };
                    self.pending_run_all = Some((script, ids));
                } else {
                    self.send_to(
                        id,
                        MasterMsg::RunPowerShell {
                            command_id: Self::new_command_id(),
                            script,
                        },
                    );
                }
            }
            ui.checkbox(&mut self.ps_run_all, "Run on ALL online agents");
        });
        ui.separator();
        let output = {
            let s = self.state.lock().unwrap();
            s.agents
                .get(id)
                .map(|a| a.console_output.clone())
                .unwrap_or_default()
        };
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut output.as_str())
                        .desired_rows(16)
                        .desired_width(f32::INFINITY)
                        .code_editor(),
                );
            });
    }

    fn tab_screen(&mut self, ui: &mut egui::Ui, id: &str) {
        ui.horizontal(|ui| {
            if ui.button("📷 Capture once").clicked() {
                self.send_to(
                    id,
                    MasterMsg::RequestScreenshot {
                        command_id: Self::new_command_id(),
                    },
                );
            }
            let streaming = {
                let s = self.state.lock().unwrap();
                s.agents.get(id).map(|a| a.streaming).unwrap_or(false)
            };
            if !streaming {
                ui.add(egui::Slider::new(&mut self.stream_fps, 1..=10).text("fps"));
                if ui.button("▶ Start live view").clicked() {
                    let interval = (1000 / self.stream_fps.max(1)) as u64;
                    self.send_to(
                        id,
                        MasterMsg::StartScreenStream {
                            interval_ms: interval,
                            jpeg_quality: 60,
                        },
                    );
                    // The agent itself notifies the user (localized) when a
                    // live view starts or a screenshot is taken.
                    if let Ok(mut s) = self.state.lock() {
                        if let Some(a) = s.agents.get_mut(id) {
                            a.streaming = true;
                        }
                    }
                }
            } else if ui.button("■ Stop live view").clicked() {
                self.send_to(id, MasterMsg::StopScreenStream);
                if let Ok(mut s) = self.state.lock() {
                    if let Some(a) = s.agents.get_mut(id) {
                        a.streaming = false;
                    }
                }
            }
        });
        ui.separator();

        // Update texture if a new frame arrived.
        let frame = {
            let s = self.state.lock().unwrap();
            s.agents.get(id).and_then(|a| {
                a.last_frame
                    .as_ref()
                    .map(|f| (a.frame_seq, f.width, f.height, f.jpeg.clone()))
            })
        };

        if let Some((seq, _, _, jpeg)) = frame {
            let need_update = self
                .textures
                .get(id)
                .map(|(s, _)| *s != seq)
                .unwrap_or(true);
            if need_update {
                if let Ok(img) = image::load_from_memory(&jpeg) {
                    let rgba = img.to_rgba8();
                    // Use the decoded size, not the agent-reported one: a
                    // mismatch makes `from_rgba_unmultiplied` panic, which
                    // aborts the whole master (panic = "abort" in release).
                    let color = egui::ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    );
                    let tex = ui.ctx().load_texture(
                        format!("screen_{id}"),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(id.to_string(), (seq, tex));
                }
            }
            if let Some((_, tex)) = self.textures.get(id) {
                let [w, h] = tex.size().map(|v| v.max(1) as f32);
                let avail = ui.available_width().min(w);
                let scale = avail / w;
                let size = egui::vec2(w * scale, h * scale);
                ui.image(egui::load::SizedTexture::new(tex.id(), size));
            }
        } else {
            ui.label("No screenshot yet. Click “Capture once” or “Start live view”.");
        }
    }

    fn push_installer(&mut self, id: &str) {
        let Some(path) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        // Same path as the fleet-wide push (ack-tracked, logged on failure).
        self.push_file_to(path, vec![id.to_string()]);
    }

    fn confirm_run_all_window(&mut self, ctx: &egui::Context) {
        let Some((script, ids)) = self.pending_run_all.clone() else {
            return;
        };
        let mut decision: Option<bool> = None;
        egui::Window::new("Run on ALL online agents?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "This runs the script below as SYSTEM on {} online PC(s).",
                    ids.len()
                ));
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut script.as_str())
                                .desired_width(420.0)
                                .code_editor(),
                        );
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let run = ui.add_enabled(
                        !ids.is_empty(),
                        egui::Button::new(format!("Run on {} PC(s)", ids.len())),
                    );
                    if run.clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                for aid in &ids {
                    self.send_to(
                        aid,
                        MasterMsg::RunPowerShell {
                            command_id: Self::new_command_id(),
                            script: script.clone(),
                        },
                    );
                }
                if let Ok(mut s) = self.state.lock() {
                    s.log(format!("PowerShell sent to {} agent(s)", ids.len()));
                }
                self.pending_run_all = None;
            }
            Some(false) => self.pending_run_all = None,
            None => {}
        }
    }

    fn config_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_config;
        egui::Window::new("Agent installer")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    "“Save agent setup” writes one .exe that installs the agent (or updates it) \
                     on double-click. It carries the config below, which pins this master's \
                     identity so agents trust only this master.",
                );
                ui.add_space(4.0);
                let mut regenerate = false;
                egui::Grid::new("cfg_fields")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Fixed master address (optional, host:port):");
                        if ui
                            .text_edit_singleline(&mut self.config_master_addr)
                            .lost_focus()
                        {
                            regenerate = true;
                        }
                        ui.end_row();
                        ui.label("e-Management URL (MoldSign bridge, optional):")
                            .on_hover_text(
                                "e.g. http://docs.ansc.local:3080 — turns on document signing \
                                 through the agent. Also applied by remote updates.",
                            );
                        if ui
                            .text_edit_singleline(&mut self.emanagement_input)
                            .lost_focus()
                        {
                            regenerate = true;
                        }
                        ui.end_row();
                    });
                if regenerate || ui.button("Apply").clicked() {
                    let url = self.emanagement_input.trim().to_string();
                    let valid =
                        url.is_empty() || url.starts_with("http://") || url.starts_with("https://");
                    if !valid {
                        self.config_status = "The e-Management URL must start with http:// or \
                                              https://"
                            .into();
                    } else if url != self.settings.emanagement_url {
                        self.settings.emanagement_url = url;
                        self.config_status = match self.settings.save() {
                            Ok(()) => "Saved.".into(),
                            Err(e) => format!("Cannot save settings: {e:#}"),
                        };
                    }
                    let addr = if self.config_master_addr.trim().is_empty() {
                        None
                    } else {
                        Some(self.config_master_addr.trim().to_string())
                    };
                    self.config_text = self
                        .settings
                        .agent_config_json(addr)
                        .unwrap_or_else(|e| format!("error: {e}"));
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.config_text.as_str())
                                .desired_width(f32::INFINITY)
                                .code_editor(),
                        );
                    });
                ui.horizontal(|ui| {
                    let save_setup = ui.add_enabled(
                        self.package.is_some(),
                        egui::Button::new("💾 Save agent setup (.exe)…"),
                    );
                    if save_setup.clicked() {
                        self.save_agent_setup();
                    }
                    if ui.button("Copy config").clicked() {
                        ui.ctx().copy_text(self.config_text.clone());
                    }
                    if ui.button("Save config as JSON…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("agent.config.json")
                            .save_file()
                        {
                            let _ = std::fs::write(path, &self.config_text);
                        }
                    }
                });
                match &self.package {
                    Some(p) => ui.label(
                        egui::RichText::new(format!(
                            "Agent package {} — {}",
                            p.version,
                            p.path.display()
                        ))
                        .weak(),
                    ),
                    None => ui.colored_label(
                        AMBER,
                        format!(
                            "No agent package ({}). Use “Choose agent exe…” in Fleet overview.",
                            self.package_error
                        ),
                    ),
                };
                if !self.config_status.is_empty() {
                    ui.label(&self.config_status);
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Keep the enrollment secret private — anyone with it can enroll an agent.",
                    )
                    .weak(),
                );
            });
        self.show_config = open;
    }

    fn save_agent_setup(&mut self) {
        let Some(p) = &self.package else { return };
        let bytes = match p.setup_with_config(&self.config_text) {
            Ok(b) => b,
            Err(e) => {
                self.config_status = format!("{e:#}");
                return;
            }
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("FleetViewAgent-Setup-{}.exe", p.version))
            .add_filter("Program", &["exe"])
            .save_file()
        else {
            return;
        };
        self.config_status = match std::fs::write(&path, bytes) {
            Ok(()) => format!(
                "Saved {}. Keep it private: it contains the enrollment secret.",
                path.display()
            ),
            Err(e) => format!("Cannot write {}: {e}", path.display()),
        };
    }
}

fn split_args(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| x.to_string()).collect()
}

/// Quote a value as a PowerShell single-quoted string literal.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Split an UninstallString into (executable, remaining arguments).
fn split_command(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
        return (rest.to_string(), String::new());
    }
    // Unquoted: the path may contain spaces, so cut after ".exe" if present.
    let lower = cmd.to_lowercase();
    if let Some(i) = lower.find(".exe") {
        let end = i + 4;
        return (cmd[..end].to_string(), cmd[end..].trim().to_string());
    }
    match cmd.split_once(' ') {
        Some((exe, args)) => (exe.to_string(), args.trim().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

/// Best-effort silent uninstall from an uninstall string. The generated
/// script waits for the uninstaller and exits with its exit code, so the
/// console shows real success/failure.
fn uninstall_script(uninstall: &str) -> String {
    let (exe, mut args) = split_command(uninstall);
    let exe_name = std::path::Path::new(&exe)
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let (file, args) = if exe_name.starts_with("msiexec") {
        // Use the product code if there is one; otherwise just switch /I → /X.
        match (uninstall.find('{'), uninstall.find('}')) {
            (Some(a), Some(b)) if b > a => (
                "msiexec.exe".to_string(),
                format!("/X {} /qn /norestart", &uninstall[a..=b]),
            ),
            _ => {
                let mut a = args.replacen("/I", "/X", 1).replacen("/i", "/X", 1);
                if !a.to_lowercase().contains("/q") {
                    a.push_str(" /qn /norestart");
                }
                ("msiexec.exe".to_string(), a)
            }
        }
    } else {
        let lower = args.to_lowercase();
        if exe_name.starts_with("unins") {
            // Inno Setup uninstaller.
            if !lower.contains("/verysilent") {
                args.push_str(" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART");
            }
        } else if !lower.contains("/s") && !lower.contains("/quiet") {
            // NSIS and many others; may still show UI for some apps.
            args.push_str(" /S");
        }
        (exe, args.trim().to_string())
    };

    let arg_list = if args.trim().is_empty() {
        String::new()
    } else {
        format!(" -ArgumentList {}", ps_quote(args.trim()))
    };
    format!(
        "$p = Start-Process -FilePath {}{arg_list} -Wait -PassThru -WindowStyle Hidden\n\
         exit $p.ExitCode",
        ps_quote(&file)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msi_uses_product_code() {
        let s = uninstall_script("MsiExec.exe /I{12345678-ABCD-1234-ABCD-1234567890AB}");
        assert!(s.contains("'msiexec.exe'"));
        assert!(s.contains("/X {12345678-ABCD-1234-ABCD-1234567890AB} /qn /norestart"));
        assert!(s.contains("exit $p.ExitCode"));
    }

    #[test]
    fn inno_gets_verysilent_and_quotes_are_escaped() {
        let s = uninstall_script(r#""C:\Program Files\O'Brien App\unins000.exe""#);
        assert!(s.contains(r"'C:\Program Files\O''Brien App\unins000.exe'"));
        assert!(s.contains("/VERYSILENT"));
    }

    #[test]
    fn unquoted_path_with_spaces() {
        let (exe, args) = split_command(r"C:\Program Files\App\uninstall.exe /x y");
        assert_eq!(exe, r"C:\Program Files\App\uninstall.exe");
        assert_eq!(args, "/x y");
    }
}
