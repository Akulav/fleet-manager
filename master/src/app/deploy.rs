//! Deploy & settings: agent setup builder, agent package, alert thresholds,
//! appearance, and master identity.

use eframe::egui::{self, vec2, RichText};

use super::App;
use crate::ui::theme::semibold;
use crate::ui::widgets as w;

impl App {
    pub(super) fn page_deploy(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        self.page_header(
            ui,
            "Deploy & settings",
            "Build the agent setup, manage the agent package, and tune alerts.",
            |_, _| {},
        );
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let gap = 12.0;
                let avail = ui.available_width();
                let wide = avail > 940.0;
                let left_w = if wide { (avail - gap) * 0.56 } else { avail };
                let right_w = if wide { avail - gap - left_w } else { avail };

                if wide {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.allocate_ui_with_layout(
                            vec2(left_w, 0.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                self.setup_card(ui, left_w);
                            },
                        );
                        ui.allocate_ui_with_layout(
                            vec2(right_w, 0.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                self.package_card(ui, right_w);
                                ui.add_space(gap);
                                self.alerts_card(ui, right_w);
                                ui.add_space(gap);
                                self.master_card(ui, right_w);
                            },
                        );
                    });
                } else {
                    self.setup_card(ui, avail);
                    ui.add_space(gap);
                    self.package_card(ui, avail);
                    ui.add_space(gap);
                    self.alerts_card(ui, avail);
                    ui.add_space(gap);
                    self.master_card(ui, avail);
                }
                let _ = pal;
            });
    }

    fn setup_card(&mut self, ui: &mut egui::Ui, width: f32) {
        let pal = self.pal;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            w::card_title(ui, &pal, "Agent setup");
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "One .exe that installs (or updates) the agent on double-click — or silently with /S. \
                     It carries a config that pins this master's identity, so agents trust only this master.",
                )
                .color(pal.text_dim),
            );
            ui.add_space(10.0);
            let mut regenerate = false;
            egui::Grid::new("cfg_fields").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(RichText::new("Fixed master address").color(pal.text));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.config_master_addr)
                        .hint_text("optional, host:port — empty = auto-discover")
                        .desired_width(300.0),
                );
                regenerate |= r.lost_focus();
                ui.end_row();
                ui.label(RichText::new("e-Management URL").color(pal.text)).on_hover_text(
                    "e.g. http://docs.ansc.local:3080 — turns on document signing through the agent \
                     (MoldSign bridge). Also applied by remote updates.",
                );
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.emanagement_input)
                        .hint_text("optional — enables the MoldSign bridge")
                        .desired_width(300.0),
                );
                regenerate |= r.lost_focus();
                ui.end_row();
            });
            ui.add_space(8.0);
            if regenerate || ui.button("Apply").clicked() {
                self.apply_setup_fields();
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if w::primary_enabled(
                    ui,
                    &pal,
                    self.package.is_some(),
                    "💾  Save agent setup (.exe)…",
                )
                .clicked()
                {
                    self.save_agent_setup();
                }
                if ui.button("📋  Copy config").clicked() {
                    ui.ctx().copy_text(self.config_text.clone());
                    self.toast("Config copied — it contains the enrollment secret");
                }
                if ui.button("Save config as JSON…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name("agent.config.json")
                        .save_file()
                    {
                        match std::fs::write(&path, &self.config_text) {
                            Ok(()) => self.toast("Config saved"),
                            Err(e) => self.toast_err(format!("Cannot save: {e}")),
                        }
                    }
                }
            });
            if self.package.is_none() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Load an agent package (right) to build setups.")
                        .color(pal.amber),
                );
            }
            if !self.config_status.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&self.config_status).color(pal.text_dim));
            }
            ui.add_space(8.0);
            egui::CollapsingHeader::new(
                RichText::new("Show config (contains the enrollment secret)").color(pal.text_dim),
            )
            .id_salt("cfg_preview")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.config_text.as_str())
                                .desired_width(f32::INFINITY)
                                .code_editor(),
                        );
                    });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("🔒  Keep the setup and the config private — anyone with the enrollment secret can enroll an agent.")
                    .size(12.0)
                    .color(pal.text_faint),
            );
        });
    }

    fn apply_setup_fields(&mut self) {
        let url = self.emanagement_input.trim().to_string();
        let valid = url.is_empty() || url.starts_with("http://") || url.starts_with("https://");
        if !valid {
            self.config_status = "The e-Management URL must start with http:// or https://".into();
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

    fn save_agent_setup(&mut self) {
        self.apply_setup_fields();
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
        match std::fs::write(&path, bytes) {
            Ok(()) => {
                self.config_status = format!(
                    "Saved {}. Keep it private: it contains the enrollment secret.",
                    path.display()
                );
                self.toast("Agent setup saved");
            }
            Err(e) => self.config_status = format!("Cannot write {}: {e}", path.display()),
        }
    }

    fn package_card(&mut self, ui: &mut egui::Ui, width: f32) {
        let pal = self.pal;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            w::card_title(ui, &pal, "Agent package");
            ui.add_space(6.0);
            match &self.package {
                Some(p) => {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Agent {}", p.version))
                                .font(semibold(16.0))
                                .color(pal.text),
                        );
                        w::status_pill(ui, "Loaded", pal.green);
                    });
                    w::faint(ui, &pal, p.path.display().to_string());
                }
                None => {
                    ui.label(
                        RichText::new("No agent package")
                            .font(semibold(15.0))
                            .color(pal.amber),
                    );
                    w::faint(ui, &pal, &self.package_error);
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("📂  Choose agent exe…").clicked() {
                    self.choose_package();
                }
                if ui
                    .button("↻  Reload")
                    .on_hover_text("Re-read the package file")
                    .clicked()
                {
                    self.load_package();
                    if self.package.is_some() {
                        self.toast("Package reloaded");
                    }
                }
            });
            ui.add_space(4.0);
            w::faint(
                ui,
                &pal,
                "Used for agent setups and remote updates. Updates never downgrade an agent.",
            );
        });
    }

    fn alerts_card(&mut self, ui: &mut egui::Ui, width: f32) {
        let pal = self.pal;
        let mut changed = false;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            w::card_title(ui, &pal, "Alerts & appearance");
            ui.add_space(6.0);
            egui::Grid::new("alert_prefs")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Disk fuller than");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.prefs.disk_warn_pct, 50.0..=99.0)
                                .suffix(" %")
                                .integer(),
                        )
                        .changed();
                    ui.end_row();
                    ui.label("Memory above");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.prefs.mem_warn_pct, 50.0..=100.0)
                                .suffix(" %")
                                .integer(),
                        )
                        .changed();
                    ui.end_row();
                    ui.label("CPU sustained above");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.prefs.cpu_warn_pct, 50.0..=100.0)
                                .suffix(" %")
                                .integer(),
                        )
                        .changed();
                    ui.end_row();
                    ui.label("Offline longer than");
                    changed |= ui
                        .add(egui::Slider::new(&mut self.prefs.stale_days, 0..=90).suffix(" days"))
                        .on_hover_text("0 = never flag offline PCs")
                        .changed();
                    ui.end_row();
                    ui.label("Theme");
                    ui.horizontal(|ui| {
                        let mut dark = self.prefs.dark;
                        ui.radio_value(&mut dark, true, "Dark");
                        ui.radio_value(&mut dark, false, "Light");
                        if dark != self.prefs.dark {
                            self.set_dark(ui.ctx(), dark);
                        }
                    });
                    ui.end_row();
                });
        });
        if changed {
            self.prefs.save();
        }
    }

    fn master_card(&mut self, ui: &mut egui::Ui, width: f32) {
        let pal = self.pal;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            w::card_title(ui, &pal, "This master");
            ui.add_space(6.0);
            egui::Grid::new("master_info")
                .num_columns(2)
                .spacing([14.0, 7.0])
                .show(ui, |ui| {
                    w::kv_row(ui, &pal, "Name", &self.settings.master_name);
                    w::kv_row(
                        ui,
                        &pal,
                        "Control port",
                        &self.settings.control_port.to_string(),
                    );
                    w::kv_row(
                        ui,
                        &pal,
                        "Discovery port",
                        &fleetview_shared::DISCOVERY_PORT.to_string(),
                    );
                    w::kv_row(
                        ui,
                        &pal,
                        "Certificate",
                        &super::truncate(&self.settings.cert_fingerprint, 32),
                    );
                    w::kv_row(
                        ui,
                        &pal,
                        "Data folder",
                        &crate::paths::data_dir().display().to_string(),
                    );
                });
            ui.add_space(6.0);
            if ui.button("📂  Open data folder").clicked() {
                let _ = std::process::Command::new("explorer")
                    .arg(crate::paths::data_dir())
                    .spawn();
            }
            w::faint(
                ui,
                &pal,
                "Back up master.settings.json — losing it orphans every installed agent.",
            );
        });
    }
}
