//! Modal dialogs: confirmation of risky actions, and the bulk-action
//! composers (script, install, message, config, tags).

use eframe::egui::{self, vec2, RichText};

use fleetview_shared::protocol::ConfigUpdate;

use super::{truncate, Action, App, ComposerKind, DeviceRow, PowerKind};
use crate::ui::theme::{semibold, tint};
use crate::ui::widgets as w;

impl App {
    /// Dim the app behind a modal and swallow clicks on it.
    fn modal_backdrop(&self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("modal_backdrop"))
            .order(egui::Order::PanelResizeLine)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_black_alpha(if self.pal.dark { 150 } else { 90 }),
                );
            });
    }

    fn hosts_of(&self, ids: &[String]) -> Vec<String> {
        let s = self.state.lock().unwrap();
        let mut h: Vec<String> = ids.iter().map(|i| s.hostname_of(i)).collect();
        h.sort_by_key(|x| x.to_lowercase());
        h
    }

    fn host_list(&self, ui: &mut egui::Ui, ids: &[String]) {
        let pal = self.pal;
        let hosts = self.hosts_of(ids);
        egui::Frame::none()
            .fill(pal.surface2)
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("modal_hosts")
                    .max_height(110.0)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
                            for h in hosts {
                                ui.label(RichText::new(h).color(pal.text_dim));
                            }
                        });
                    });
            });
    }

    pub(super) fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm.clone() else {
            return;
        };
        let pal = self.pal;
        self.modal_backdrop(ctx);
        let ids = action.ids();
        let n = ids.len();
        let (title, body, label, danger): (String, String, String, bool) = match &action {
            Action::RunScript { .. } => (
                "Run script as SYSTEM?".into(),
                format!("The script below runs elevated on {n} PC(s)."),
                format!("Run on {n} PC(s)"),
                n > 1,
            ),
            Action::InstallApps { apps, .. } => (
                "Install software?".into(),
                format!(
                    "Push {} installer(s) to {n} PC(s) and run them silently as SYSTEM, one after another on each PC. \
                     PCs that already have an app (by its “skip if installed” name) are skipped.",
                    apps.len()
                ),
                format!("Install on {n} PC(s)"),
                false,
            ),
            Action::Rename { restart, .. } => (
                "Rename PCs?".into(),
                format!(
                    "Rename {n} PC(s) as listed below. The new name takes effect after a restart{}.",
                    if *restart { " — they restart in 1 minute, with a warning to the user" } else { "" }
                ),
                format!("Rename {n} PC(s)"),
                *restart,
            ),
            Action::SetAdmin { .. } => (
                "Set the admin account?".into(),
                format!(
                    "Create (or reset the password of) the local admin “{}” on {n} PC(s) and add it to Administrators.{}",
                    self.policy.admin_user.trim(),
                    if self.policy.admin_demote_others {
                        "\nEvery OTHER local account is removed from Administrators (it keeps working as a standard user)."
                    } else {
                        ""
                    }
                ),
                format!("Apply to {n} PC(s)"),
                self.policy.admin_demote_others,
            ),
            Action::Wallpaper { .. } => (
                "Set the desktop background?".into(),
                format!(
                    "Set {} as the background of every user on {n} PC(s) ({}){}.",
                    std::path::Path::new(&self.policy.wallpaper_path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                    self.policy.wallpaper_style.label(),
                    if self.policy.wallpaper_lock { ", locked so users can't change it" } else { "" }
                ),
                format!("Apply to {n} PC(s)"),
                false,
            ),
            Action::InstallUrl { url, .. } => (
                "Install from URL?".into(),
                format!(
                    "Each PC downloads {} and runs it silently as SYSTEM{} with arguments “{}”.",
                    truncate(url, 80),
                    if self.install_is_msi { " (MSI)" } else { "" },
                    self.install_args
                ),
                format!("Install on {n} PC(s)"),
                false,
            ),
            Action::Update { .. } => {
                let v = self.package.as_ref().map(|p| p.version.to_string()).unwrap_or_default();
                let mut b = format!(
                    "Install agent {v} on {n} PC(s). Each agent restarts (a few seconds offline) and rolls back by itself if the new version doesn't start."
                );
                if !self.settings.emanagement_url.trim().is_empty() {
                    b.push_str(&format!("\nMoldSign bridge: e-Management URL set to {}.", self.settings.emanagement_url.trim()));
                }
                ("Update agents?".into(), b, format!("Update {n} PC(s)"), false)
            }
            Action::Power { kind, .. } => {
                let b = match kind {
                    PowerKind::Restart => format!("{n} PC(s) restart in 1 minute. Users see a warning and can save their work."),
                    PowerKind::Shutdown => format!("{n} PC(s) shut down in 1 minute. Users see a warning and can save their work."),
                    PowerKind::Abort => format!("Cancel a pending restart/shutdown on {n} PC(s)."),
                };
                (format!("{}?", kind.label()), b, format!("{} {n} PC(s)", kind.label()), *kind != PowerKind::Abort)
            }
            Action::Uninstall { name, .. } => (
                "Uninstall software?".into(),
                format!("Silently uninstall “{name}” from {n} PC(s). Some uninstallers may still show a window to the user."),
                format!("Uninstall from {n} PC(s)"),
                true,
            ),
            Action::SetConfig { updates, .. } => (
                "Change agent config?".into(),
                format!(
                    "Set {} on {n} PC(s). Agents restart to apply the change.",
                    updates.iter().map(|u| u.key.as_str()).collect::<Vec<_>>().join(", ")
                ),
                format!("Apply to {n} PC(s)"),
                false,
            ),
            Action::Notify { .. } => (
                "Send message?".into(),
                format!("Show a notification to the user on {n} PC(s)."),
                format!("Send to {n} PC(s)"),
                false,
            ),
            Action::Forget { .. } => (
                "Remove from list?".into(),
                format!("Forget {n} offline PC(s), with their tags and notes. They re-appear if their agent connects again."),
                format!("Remove {n} PC(s)"),
                true,
            ),
        };

        let mut decision: Option<bool> = None;
        egui::Window::new(RichText::new(&title).font(semibold(17.0)))
            .id(egui::Id::new("confirm"))
            .collapsible(false)
            .resizable(false)
            .title_bar(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.label(RichText::new(&body).color(pal.text));
                ui.add_space(8.0);
                if let Action::RunScript { script, .. } = &action {
                    egui::Frame::none()
                        .fill(ui.visuals().extreme_bg_color)
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::ScrollArea::vertical()
                                .id_salt("confirm_script")
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    ui.label(RichText::new(script).monospace().color(pal.text));
                                });
                        });
                    ui.add_space(8.0);
                }
                if let Action::Notify { title, body, .. } = &action {
                    egui::Frame::none()
                        .fill(pal.surface2)
                        .rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(title).font(semibold(13.5)).color(pal.text));
                            ui.label(RichText::new(body).color(pal.text_dim));
                        });
                    ui.add_space(8.0);
                }
                if let Action::InstallApps { apps, .. } = &action {
                    self.app_summary(ui, apps);
                    ui.add_space(8.0);
                }
                if let Action::Rename { names, .. } = &action {
                    let s = self.state.lock().unwrap();
                    let list: Vec<(String, String)> = names
                        .iter()
                        .map(|(id, n)| (s.hostname_of(id), n.clone()))
                        .collect();
                    drop(s);
                    self.rename_preview(ui, &list);
                    ui.add_space(8.0);
                }
                w::card_title(ui, &pal, &format!("Target PCs ({n})"));
                self.host_list(ui, &ids);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let go = if danger {
                            w::danger(ui, &pal, n > 0, &label)
                        } else {
                            w::primary_enabled(ui, &pal, n > 0, &label)
                        };
                        if go.clicked() {
                            decision = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            decision = Some(false);
                        }
                    });
                });
            });
        match decision {
            Some(true) => {
                self.confirm = None;
                self.execute(action);
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    pub(super) fn composer_window(&mut self, ctx: &egui::Context, rows: &[DeviceRow]) {
        let Some(c) = &self.composer else { return };
        let kind = c.kind;
        let ids = c.ids.clone();
        let pal = self.pal;
        self.modal_backdrop(ctx);
        let title = match kind {
            ComposerKind::Script => "Run PowerShell",
            ComposerKind::Install => "Install software",
            ComposerKind::Notify => "Message users",
            ComposerKind::Config => "Remote config",
            ComposerKind::Tags => "Edit tags",
            ComposerKind::Rename => "Rename PCs",
        };
        let mut open = true;
        let mut next: Option<Action> = None;
        let mut close = false;
        egui::Window::new(RichText::new(title).font(semibold(17.0)))
            .id(egui::Id::new("composer"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(560.0);
                // Target picker: selected PCs or all online.
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Targets: {} PC(s)", ids.len())).font(semibold(13.5)).color(pal.text));
                    if kind != ComposerKind::Tags {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let all = self.online_ids();
                            if ui.add_enabled(all.len() != ids.len(), egui::Button::new(format!("Use all {} online", all.len()))).clicked() {
                                if let Some(c) = &mut self.composer {
                                    c.ids = all;
                                }
                            }
                        });
                    }
                });
                self.host_list(ui, &ids);
                ui.add_space(10.0);

                match kind {
                    ComposerKind::Script => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Script (runs as SYSTEM)").color(pal.text_dim));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                self.snippet_menus(ui);
                            });
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut self.ps_input)
                                .desired_rows(9)
                                .desired_width(f32::INFINITY)
                                .code_editor()
                                .hint_text("Get-Service Spooler | Restart-Service"),
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let ok = !ids.is_empty() && !self.ps_input.trim().is_empty();
                                if w::primary_enabled(ui, &pal, ok, "Review & run…").clicked() {
                                    next = Some(Action::RunScript { script: self.ps_input.clone(), ids: ids.clone() });
                                }
                                if ui.button("Cancel").clicked() {
                                    close = true;
                                }
                            });
                        });
                    }
                    ComposerKind::Install => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Installers").font(semibold(13.5)).color(pal.text));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add_enabled(!self.policy.apps.is_empty(), egui::Button::new(format!("+ App package ({})", self.policy.apps.len())))
                                    .on_hover_text("Add every app of the fleet's standard package (Standards page)")
                                    .clicked()
                                {
                                    for a in self.policy.apps.clone() {
                                        if !self.install_queue.iter().any(|q| q.path == a.path) {
                                            self.install_queue.push(a);
                                        }
                                    }
                                }
                                if ui.button("📂  Add files…").clicked() {
                                    self.add_installer_files_to_queue();
                                }
                            });
                        });
                        ui.add_space(4.0);
                        let mut queue = std::mem::take(&mut self.install_queue);
                        self.app_list_editor(ui, &mut queue, "install_queue");
                        self.install_queue = queue;
                        ui.add_space(10.0);
                        let files_ok = !self.install_queue.is_empty()
                            && self.install_queue.iter().all(|a| std::path::Path::new(&a.path).is_file());
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let label = format!("Install {} app(s)…", self.install_queue.len());
                                if w::primary_enabled(ui, &pal, !ids.is_empty() && files_ok, &label).clicked() {
                                    next = Some(Action::InstallApps {
                                        apps: self.install_queue.clone(),
                                        ids: ids.clone(),
                                        package: false,
                                    });
                                }
                                if ui.button("Cancel").clicked() {
                                    close = true;
                                }
                            });
                        });
                        ui.add_space(8.0);
                        egui::CollapsingHeader::new(RichText::new("Install from an https URL instead").color(pal.text_dim))
                            .id_salt("inst_url")
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(&mut self.url_input).hint_text("https://example.com/setup.msi").desired_width(300.0));
                                    ui.add(egui::TextEdit::singleline(&mut self.install_args).desired_width(130.0))
                                        .on_hover_text("Silent arguments");
                                    ui.checkbox(&mut self.install_is_msi, "MSI");
                                });
                                let url = self.url_input.trim().to_string();
                                let url_ok = url.to_lowercase().starts_with("https://");
                                if !url.is_empty() && !url_ok {
                                    ui.label(RichText::new("URLs must start with https://").color(pal.red));
                                }
                                if ui.add_enabled(!ids.is_empty() && url_ok, egui::Button::new("Install from URL…")).clicked() {
                                    next = Some(Action::InstallUrl { url: url.clone(), ids: ids.clone() });
                                }
                            });
                    }
                    ComposerKind::Rename => {
                        let mut names: Vec<(String, String)> = Vec::new();
                        let mut problems: Vec<String> = Vec::new();
                        if ids.len() == 1 {
                            ui.horizontal(|ui| {
                                ui.label("New name");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.rename_input)
                                        .hint_text("e.g. ANSC-012")
                                        .char_limit(15)
                                        .desired_width(220.0),
                                );
                            });
                            let n = self.rename_input.trim().to_uppercase();
                            match crate::policy::validate_pc_name(&n) {
                                Ok(()) => names.push((ids[0].clone(), n)),
                                Err(e) if !n.is_empty() => problems.push(e),
                                Err(_) => {}
                            }
                        } else {
                            egui::Grid::new("rename_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                                ui.label("Pattern");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.policy.name_pattern)
                                        .hint_text("ANSC-###")
                                        .desired_width(220.0),
                                );
                                ui.end_row();
                                ui.label("First number");
                                ui.add(egui::DragValue::new(&mut self.policy.name_start).range(0..=99999));
                                ui.end_row();
                            });
                            w::faint(ui, &pal, "Each run of # becomes a zero-padded number; PCs are numbered in their current name order.");
                            let s = self.state.lock().unwrap();
                            let mut sorted: Vec<(String, String)> = ids.iter().map(|i| (s.hostname_of(i).to_lowercase(), i.clone())).collect();
                            drop(s);
                            sorted.sort();
                            for (k, (_, id)) in sorted.into_iter().enumerate() {
                                let n = crate::policy::expand_pattern(&self.policy.name_pattern, self.policy.name_start + k as u32);
                                if let Err(e) = crate::policy::validate_pc_name(&n) {
                                    problems.push(e);
                                }
                                names.push((id, n));
                            }
                        }
                        ui.checkbox(&mut self.policy.name_restart, "Restart in 1 minute to apply (users get a warning)");
                        problems.dedup();
                        for p in problems.iter().take(3) {
                            ui.label(RichText::new(p).color(pal.red));
                        }
                        if !names.is_empty() {
                            let s = self.state.lock().unwrap();
                            let list: Vec<(String, String)> =
                                names.iter().map(|(id, n)| (s.hostname_of(id), n.clone())).collect();
                            drop(s);
                            ui.add_space(6.0);
                            self.rename_preview(ui, &list);
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let ok = !names.is_empty() && problems.is_empty();
                                if w::primary_enabled(ui, &pal, ok, "Review & rename…").clicked() {
                                    let _ = self.policy.save();
                                    next = Some(Action::Rename { names: names.clone(), restart: self.policy.name_restart });
                                }
                                if ui.button("Cancel").clicked() {
                                    close = true;
                                }
                            });
                        });
                    }
                    ComposerKind::Notify => {
                        egui::Grid::new("notify").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label("Title");
                            ui.add(egui::TextEdit::singleline(&mut self.notify_title).desired_width(420.0));
                            ui.end_row();
                            ui.label("Message");
                            ui.add(
                                egui::TextEdit::multiline(&mut self.notify_body)
                                    .hint_text("e.g. Please restart your PC before 17:00 today for updates.")
                                    .desired_rows(4)
                                    .desired_width(420.0),
                            );
                            ui.end_row();
                        });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let ok = !ids.is_empty() && !self.notify_body.trim().is_empty();
                                if w::primary_enabled(ui, &pal, ok, "Send").clicked() {
                                    let (t, b) = (self.notify_title.clone(), self.notify_body.trim().to_string());
                                    if ids.len() == 1 {
                                        self.notify(t, b, &ids);
                                        close = true;
                                    } else {
                                        next = Some(Action::Notify { title: t, body: b, ids: ids.clone() });
                                    }
                                }
                                if ui.button("Cancel").clicked() {
                                    close = true;
                                }
                            });
                        });
                    }
                    ComposerKind::Config => {
                        ui.label(
                            RichText::new(
                                "The agent applies only these keys (never its master identity or enrollment secret) \
                                 and restarts to pick them up. An empty value clears the key.",
                            )
                            .color(pal.text_dim),
                        );
                        ui.add_space(6.0);
                        egui::Grid::new("rc_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                            for f in self.rc.iter_mut() {
                                ui.checkbox(&mut f.apply, f.label);
                                ui.add_enabled(
                                    f.apply,
                                    egui::TextEdit::singleline(&mut f.value).desired_width(340.0).hint_text(f.hint),
                                );
                                ui.end_row();
                            }
                        });
                        let updates: Vec<ConfigUpdate> = self
                            .rc
                            .iter()
                            .filter(|f| f.apply)
                            .map(|f| ConfigUpdate {
                                key: f.key.to_string(),
                                value: {
                                    let v = f.value.trim();
                                    (!v.is_empty()).then(|| v.to_string())
                                },
                            })
                            .collect();
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let ok = !ids.is_empty() && !updates.is_empty();
                                if w::primary_enabled(ui, &pal, ok, "Review & apply…").clicked() {
                                    next = Some(Action::SetConfig { ids: ids.clone(), updates: updates.clone() });
                                }
                                if ui.button("Cancel").clicked() {
                                    close = true;
                                }
                            });
                        });
                    }
                    ComposerKind::Tags => {
                        let existing = App::all_tags(rows);
                        let on: Vec<(String, usize)> = existing
                            .iter()
                            .map(|t| {
                                (
                                    t.clone(),
                                    rows.iter().filter(|r| ids.contains(&r.id) && r.tags.contains(t)).count(),
                                )
                            })
                            .collect();
                        ui.label(RichText::new("Click a tag to add it to (or remove it from) every target.").color(pal.text_dim));
                        ui.add_space(6.0);
                        let mut add: Option<String> = None;
                        let mut remove: Option<String> = None;
                        ui.horizontal_wrapped(|ui| {
                            for (t, n) in &on {
                                let all = *n == ids.len() && *n > 0;
                                let label = if all {
                                    format!("✔ {t}")
                                } else if *n > 0 {
                                    format!("◐ {t} ({n})")
                                } else {
                                    t.clone()
                                };
                                if w::chip(ui, &pal, all, &label).clicked() {
                                    if all {
                                        remove = Some(t.clone());
                                    } else {
                                        add = Some(t.clone());
                                    }
                                }
                            }
                            if on.is_empty() {
                                w::faint(ui, &pal, "No tags yet — create one below.");
                            }
                        });
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut self.tag_input)
                                    .hint_text("New tag, e.g. Accounting")
                                    .desired_width(300.0),
                            );
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if (ui.button("Add to all").clicked() || enter) && !self.tag_input.trim().is_empty() {
                                add = Some(self.tag_input.trim().to_string());
                                self.tag_input.clear();
                            }
                        });
                        if let Some(t) = add {
                            self.set_tags(&ids, &[t], &[]);
                        }
                        if let Some(t) = remove {
                            self.set_tags(&ids, &[], &[t]);
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if w::primary(ui, &pal, "Done").clicked() {
                                    close = true;
                                }
                            });
                        });
                    }
                }
            });
        if !open || close {
            self.composer = None;
        }
        if let Some(a) = next {
            self.composer = None;
            self.confirm = Some(a);
        }
        let _ = tint(pal.accent, 0);
    }
}
