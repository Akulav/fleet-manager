//! egui desktop UI for the master. English only (per requirement).

use std::collections::HashMap;

use eframe::egui;

use fleetview_shared::protocol::{MasterMsg, RunSpec};

use crate::settings::MasterSettings;
use crate::state::SharedState;

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

    // agent-config export window
    show_config: bool,
    config_text: String,
    config_master_addr: String,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, state: SharedState, settings: MasterSettings) -> Self {
        Self {
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
            show_config: false,
            config_text: String::new(),
            config_master_addr: String::new(),
        }
    }

    fn new_command_id() -> String {
        format!("{}", chrono::Local::now().timestamp_millis())
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
    }
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("FleetView");
                ui.label(egui::RichText::new("by Akulav").weak());
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
                ui.label(format!("{name}"));
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
                    if ui.button("Agent installer config").clicked() {
                        self.config_text = self
                            .settings
                            .agent_config_json(None)
                            .unwrap_or_else(|e| format!("error: {e}"));
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
                    let agents: Vec<(String, String, bool, f32)> = {
                        let s = self.state.lock().unwrap();
                        s.agents
                            .values()
                            .map(|a| {
                                let cpu = a.telemetry.as_ref().map(|t| t.cpu_usage_pct).unwrap_or(0.0);
                                (a.agent_id.clone(), a.hostname.clone(), a.online, cpu)
                            })
                            .collect()
                    };
                    for (id, host, online, cpu) in agents {
                        let dot = if online { "🟢" } else { "⚪" };
                        let label = if online {
                            format!("{dot} {host}  ({cpu:.0}%)")
                        } else {
                            format!("{dot} {host}")
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
        let rows: Vec<[String; 5]> = {
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
                    [
                        format!("{} {}", if a.online { "🟢" } else { "⚪" }, a.hostname),
                        user,
                        ips,
                        a.last_seen.format("%Y-%m-%d %H:%M").to_string(),
                        a.telemetry
                            .as_ref()
                            .map(|t| format!("{:.0}% CPU", t.cpu_usage_pct))
                            .unwrap_or_default(),
                    ]
                })
                .collect()
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("fleet_grid")
                .striped(true)
                .num_columns(5)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Host");
                    ui.strong("User");
                    ui.strong("IP addresses");
                    ui.strong("Last seen");
                    ui.strong("CPU");
                    ui.end_row();
                    for r in &rows {
                        for cell in r {
                            ui.label(cell);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(12.0);
            ui.collapsing("Events", |ui| {
                let events: Vec<String> =
                    self.state.lock().unwrap().events.iter().cloned().collect();
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for e in events {
                        ui.monospace(e);
                    }
                });
            });
        });
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
        let t = {
            let s = self.state.lock().unwrap();
            s.agents.get(id).and_then(|a| a.telemetry.clone())
        };
        let Some(t) = t else {
            ui.label("No telemetry yet…");
            return;
        };
        egui::Grid::new("tele").num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
            ui.strong("OS");
            ui.label(&t.os_version);
            ui.end_row();
            ui.strong("Logged-in user");
            ui.label(t.logged_in_user.clone().unwrap_or_else(|| "—".into()));
            ui.end_row();
            ui.strong("CPU");
            ui.label(format!("{} ({} cores) — {:.0}%", t.cpu_name, t.cpu_cores, t.cpu_usage_pct));
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
            let frac = if d.total_gb > 0.0 { (used / d.total_gb) as f32 } else { 0.0 };
            ui.horizontal(|ui| {
                ui.label(format!("{}", d.mount));
                ui.add(egui::ProgressBar::new(frac).desired_width(220.0).text(format!(
                    "{:.0}/{:.0} GB",
                    used, d.total_gb
                )));
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
            if ui.button("Download & run").clicked() && !self.url_input.is_empty() {
                let run = RunSpec {
                    command_id: Self::new_command_id(),
                    args: split_args(&self.install_args),
                    is_msi: self.install_is_msi,
                };
                self.send_to(
                    id,
                    MasterMsg::InstallFromUrl {
                        command_id: run.command_id.clone(),
                        url: self.url_input.clone(),
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
            egui::Grid::new("sw").striped(true).num_columns(3).spacing([12.0, 4.0]).show(
                ui,
                |ui| {
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
                },
            );
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
                    let ids: Vec<String> = {
                        let s = self.state.lock().unwrap();
                        s.agents.values().filter(|a| a.online).map(|a| a.agent_id.clone()).collect()
                    };
                    for aid in ids {
                        self.send_to(
                            &aid,
                            MasterMsg::RunPowerShell {
                                command_id: Self::new_command_id(),
                                script: script.clone(),
                            },
                        );
                    }
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
            s.agents.get(id).map(|a| a.console_output.clone()).unwrap_or_default()
        };
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
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
                self.send_to(id, MasterMsg::RequestScreenshot { command_id: Self::new_command_id() });
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
                        MasterMsg::StartScreenStream { interval_ms: interval, jpeg_quality: 60 },
                    );
                    // Transparency: tell the user their screen is being viewed.
                    self.send_to(
                        id,
                        MasterMsg::NotifyUser {
                            title: "Remote support".into(),
                            body: "An administrator has started viewing this screen.".into(),
                        },
                    );
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
                a.last_frame.as_ref().map(|f| (a.frame_seq, f.width, f.height, f.jpeg.clone()))
            })
        };

        if let Some((seq, w, h, jpeg)) = frame {
            let need_update = self.textures.get(id).map(|(s, _)| *s != seq).unwrap_or(true);
            if need_update {
                if let Ok(img) = image::load_from_memory(&jpeg) {
                    let rgba = img.to_rgba8();
                    let color = egui::ColorImage::from_rgba_unmultiplied(
                        [w as usize, h as usize],
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
                let avail = ui.available_width().min(w as f32);
                let scale = avail / w as f32;
                let size = egui::vec2(w as f32 * scale, h as f32 * scale);
                ui.image(egui::load::SizedTexture::new(tex.id(), size));
            }
        } else {
            ui.label("No screenshot yet. Click “Capture once” or “Start live view”.");
        }
    }

    fn push_installer(&mut self, id: &str) {
        let Some(path) = rfd::FileDialog::new().pick_file() else { return };
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "installer.exe".into());
        let is_msi = self.install_is_msi || file_name.to_lowercase().ends_with(".msi");
        let run = RunSpec {
            command_id: Self::new_command_id(),
            args: split_args(&self.install_args),
            is_msi,
        };

        // Clone the sender so we can stream the file off the UI thread.
        let tx = {
            let s = self.state.lock().unwrap();
            s.agents.get(id).and_then(|a| a.tx.clone())
        };
        let Some(tx) = tx else { return };
        let transfer_id = Self::new_command_id();

        std::thread::spawn(move || {
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => return,
            };
            let total = data.len() as u64;
            let _ = tx.send(MasterMsg::FileBegin {
                transfer_id: transfer_id.clone(),
                file_name,
                total_len: total,
                run_after: Some(run),
            });
            for chunk in data.chunks(256 * 1024) {
                if tx
                    .send(MasterMsg::FileChunk {
                        transfer_id: transfer_id.clone(),
                        data: chunk.to_vec(),
                    })
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.send(MasterMsg::FileEnd { transfer_id });
        });
    }

    fn config_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_config;
        egui::Window::new("Agent installer configuration")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(
                    "Bundle this as agent.config.json with the agent installer. It pins this \
                     master's identity so agents trust only this master.",
                );
                ui.horizontal(|ui| {
                    ui.label("Fixed master address (optional, host:port):");
                    ui.text_edit_singleline(&mut self.config_master_addr);
                    if ui.button("Apply").clicked() {
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
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.config_text.as_str())
                            .desired_width(f32::INFINITY)
                            .code_editor(),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Copy to clipboard").clicked() {
                        ui.ctx().copy_text(self.config_text.clone());
                    }
                    if ui.button("Save to file…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("agent.config.json")
                            .save_file()
                        {
                            let _ = std::fs::write(path, &self.config_text);
                        }
                    }
                });
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
}

fn split_args(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| x.to_string()).collect()
}

/// Best-effort silent uninstall from an uninstall string.
fn uninstall_script(uninstall: &str) -> String {
    let lower = uninstall.to_lowercase();
    if lower.contains("msiexec") {
        // Convert an install string to a quiet uninstall.
        let s = uninstall.replace("/I", "/X").replace("/i", "/X");
        if s.to_lowercase().contains("/qn") {
            format!("cmd /c {s}")
        } else {
            format!("cmd /c {s} /qn /norestart")
        }
    } else {
        // Try passing common silent switches; may still show UI for some apps.
        format!("Start-Process -Wait -FilePath cmd -ArgumentList '/c',@'\n{uninstall} /S\n'@")
    }
}
