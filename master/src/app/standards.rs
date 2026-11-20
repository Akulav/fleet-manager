//! Standards page: the settings every PC should share — one local admin
//! account, one desktop background, one app package — plus PC naming.
//! Each standard can be applied by hand or kept applied automatically.

use eframe::egui::{self, vec2, RichText};

use super::{Action, App, Composer, ComposerKind, DeviceRow};
use crate::installer::InstallerKind;
use crate::policy::{self, AppItem, WallStyle};
use crate::ui::theme::semibold;
use crate::ui::widgets as w;

/// Where an "Apply to…" menu sends a standard.
enum Target {
    Selected,
    AllOnline,
}

impl App {
    pub(super) fn page_standards(&mut self, ui: &mut egui::Ui, rows: &[DeviceRow]) {
        self.page_header(
            ui,
            "Standards",
            "One admin account, one background and one app package on every PC. \
             Switch a standard to automatic and new PCs get it as soon as they connect.",
            |_, _| {},
        );
        let selected: Vec<String> = rows
            .iter()
            .filter(|r| r.online && self.checked.contains(&r.id))
            .map(|r| r.id.clone())
            .collect();
        let online: Vec<String> = rows
            .iter()
            .filter(|r| r.online)
            .map(|r| r.id.clone())
            .collect();
        let total = rows.len();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ui.available_width().min(980.0);
                ui.set_max_width(width);
                if !self.standards_status.is_empty() {
                    ui.label(RichText::new(&self.standards_status).color(self.pal.amber));
                    ui.add_space(6.0);
                }
                self.admin_card(ui, width, &selected, &online, total);
                ui.add_space(12.0);
                self.wallpaper_card(ui, width, &selected, &online, total);
                ui.add_space(12.0);
                self.package_card_std(ui, width, &selected, &online, total);
                ui.add_space(12.0);
                self.names_card(ui, width, &selected, &online);
                ui.add_space(12.0);
                w::faint(
                    ui,
                    &self.pal,
                    format!(
                        "Saved in {} (with the admin password) — keep the data folder private and backed up.",
                        policy::Policy::path().display()
                    ),
                );
            });
    }

    /// Save the standards; false (with a message) when they're inconsistent.
    pub(super) fn save_policy(&mut self) -> bool {
        if self.policy.admin_password != self.admin_pw_confirm {
            self.standards_status = "The two admin passwords don't match — nothing saved.".into();
            return false;
        }
        match self.policy.save_bumping() {
            Ok(()) => {
                self.standards_status.clear();
                true
            }
            Err(e) => {
                self.standards_status = format!("Cannot save the standards: {e}");
                false
            }
        }
    }

    /// How many known PCs have `key` at `rev`.
    fn applied_count(&self, key: &str, rev: &str) -> usize {
        let s = self.state.lock().unwrap();
        s.agents
            .values()
            .filter(|a| a.applied.get(key).map(String::as_str) == Some(rev))
            .count()
    }

    /// "Apply to…" menu; returns the chosen target list.
    fn apply_menu(
        &self,
        ui: &mut egui::Ui,
        enabled: bool,
        label: &str,
        selected: &[String],
        online: &[String],
    ) -> Option<Vec<String>> {
        let mut pick = None;
        ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button(label, |ui| {
                if ui
                    .add_enabled(
                        !selected.is_empty(),
                        egui::Button::new(format!("Selected online PCs ({})", selected.len())),
                    )
                    .on_disabled_hover_text("Tick PCs on the Devices page first")
                    .clicked()
                {
                    pick = Some(Target::Selected);
                    ui.close_menu();
                }
                if ui
                    .add_enabled(
                        !online.is_empty(),
                        egui::Button::new(format!("All online PCs ({})", online.len())),
                    )
                    .clicked()
                {
                    pick = Some(Target::AllOnline);
                    ui.close_menu();
                }
            });
        });
        pick.map(|t| match t {
            Target::Selected => selected.to_vec(),
            Target::AllOnline => online.to_vec(),
        })
    }

    fn compliance(&self, ui: &mut egui::Ui, done: usize, total: usize, auto: bool) {
        let pal = self.pal;
        let color = if total > 0 && done == total {
            pal.green
        } else if done > 0 {
            pal.amber
        } else {
            pal.text_faint
        };
        w::status_pill(ui, &format!("{done} / {total} PCs"), color);
        if auto {
            w::status_pill(ui, "Automatic", pal.blue);
        }
    }

    fn admin_card(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        selected: &[String],
        online: &[String],
        total: usize,
    ) {
        let pal = self.pal;
        let done = self.applied_count(policy::KEY_ADMIN, &self.policy.admin_rev.to_string());
        let mut apply_to = None;
        let mut changed = false;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            ui.horizontal(|ui| {
                w::card_title(ui, &pal, "Local admin account");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.compliance(ui, done, total, self.policy.admin_auto);
                });
            });
            w::faint(
                ui,
                &pal,
                "Creates the account (or resets its password) and adds it to the local Administrators \
                 group — found by SID, so it works on Romanian Windows too. Password never expires.",
            );
            ui.add_space(8.0);
            egui::Grid::new("std_admin")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("User name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.policy.admin_user)
                            .hint_text("e.g. itadmin")
                            .desired_width(240.0),
                    );
                    ui.end_row();
                    ui.label("Password");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.policy.admin_password)
                                .password(!self.show_pw)
                                .desired_width(240.0),
                        );
                        ui.checkbox(&mut self.show_pw, "Show");
                    });
                    ui.end_row();
                    ui.label("Repeat password");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.admin_pw_confirm)
                            .password(!self.show_pw)
                            .desired_width(240.0),
                    );
                    ui.end_row();
                });
            if let Err(e) = policy::validate_account_name(&self.policy.admin_user) {
                if !self.policy.admin_user.is_empty() {
                    ui.label(RichText::new(e).color(pal.red));
                }
            }
            if !self.admin_pw_confirm.is_empty()
                && self.policy.admin_password != self.admin_pw_confirm
            {
                ui.label(RichText::new("The passwords don't match").color(pal.red));
            }
            ui.add_space(4.0);
            changed |= ui
                .checkbox(
                    &mut self.policy.admin_demote_others,
                    "Remove every other local account from Administrators",
                )
                .on_hover_text(
                    "They keep working as standard users. Domain accounts and groups are not touched. \
                     Make sure the users don't need admin rights for their daily work.",
                )
                .changed();
            if self.policy.admin_demote_others {
                ui.label(
                    RichText::new("⚠  Users who are local admins today lose admin rights.")
                        .color(pal.amber),
                );
            }
            changed |= ui
                .checkbox(
                    &mut self.policy.admin_auto,
                    "Keep applied automatically (also on new PCs)",
                )
                .changed();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("💾  Save").clicked() && self.save_policy() {
                    self.toast("Admin account standard saved");
                }
                let ready = self.policy.admin_ready()
                    && self.policy.admin_password == self.admin_pw_confirm;
                apply_to = self.apply_menu(ui, ready, "▶  Apply to…", selected, online);
            });
        });
        if changed {
            self.save_policy();
        }
        if let Some(ids) = apply_to {
            if self.save_policy() {
                self.confirm = Some(Action::SetAdmin { ids });
            }
        }
    }

    fn wallpaper_card(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        selected: &[String],
        online: &[String],
        total: usize,
    ) {
        let pal = self.pal;
        let done = self.applied_count(
            policy::KEY_WALLPAPER,
            &self.policy.wallpaper_rev.to_string(),
        );
        let mut apply_to = None;
        let mut changed = false;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            ui.horizontal(|ui| {
                w::card_title(ui, &pal, "Desktop background");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.compliance(ui, done, total, self.policy.wallpaper_auto);
                });
            });
            w::faint(
                ui,
                &pal,
                "Set for every user profile on the PC and for new users; whoever is logged on sees it \
                 right away. The image is shrunk to 4K and sent as JPEG.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.policy.wallpaper_path)
                        .hint_text("C:\\Wallpapers\\company.jpg")
                        .desired_width(360.0),
                );
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Images", &["jpg", "jpeg", "png"])
                        .pick_file()
                    {
                        self.policy.wallpaper_path = p.display().to_string();
                        changed = true;
                    }
                }
            });
            if !self.policy.wallpaper_path.is_empty()
                && !std::path::Path::new(&self.policy.wallpaper_path).is_file()
            {
                ui.label(RichText::new("File not found").color(pal.red));
            }
            ui.horizontal(|ui| {
                ui.label("Style");
                egui::ComboBox::from_id_salt("wall_style")
                    .selected_text(self.policy.wallpaper_style.label())
                    .show_ui(ui, |ui| {
                        for s in WallStyle::ALL {
                            changed |= ui
                                .selectable_value(&mut self.policy.wallpaper_style, s, s.label())
                                .changed();
                        }
                    });
                changed |= ui
                    .checkbox(
                        &mut self.policy.wallpaper_lock,
                        "Lock (users can't change it)",
                    )
                    .changed();
            });
            changed |= ui
                .checkbox(
                    &mut self.policy.wallpaper_auto,
                    "Keep applied automatically (also on new PCs)",
                )
                .changed();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("💾  Save").clicked() && self.save_policy() {
                    self.toast("Background standard saved");
                }
                let ready = std::path::Path::new(&self.policy.wallpaper_path).is_file();
                apply_to = self.apply_menu(ui, ready, "▶  Apply to…", selected, online);
            });
        });
        if changed {
            self.save_policy();
        }
        if let Some(ids) = apply_to {
            if self.save_policy() {
                self.confirm = Some(Action::Wallpaper { ids });
            }
        }
    }

    fn package_card_std(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        selected: &[String],
        online: &[String],
        total: usize,
    ) {
        let pal = self.pal;
        let revs: Vec<(String, Option<String>)> = self
            .policy
            .apps
            .iter()
            .map(|a| (a.key(), a.revision()))
            .collect();
        let done = if revs.is_empty() {
            0
        } else {
            let s = self.state.lock().unwrap();
            s.agents
                .values()
                .filter(|a| {
                    revs.iter().all(|(k, r)| {
                        r.as_deref()
                            .is_some_and(|r| a.applied.get(k).map(String::as_str) == Some(r))
                    })
                })
                .count()
        };
        let mut install_to = None;
        let mut changed = false;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            ui.horizontal(|ui| {
                w::card_title(ui, &pal, "App package");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.compliance(ui, done, total, self.policy.apps_auto);
                });
            });
            w::faint(
                ui,
                &pal,
                "Installers every PC should have. They install silently one after another on each PC; \
                 a PC that already lists a program matching “skip if installed” is skipped. \
                 Replacing a file with a newer one installs it again everywhere.",
            );
            ui.add_space(8.0);
            let mut apps = std::mem::take(&mut self.policy.apps);
            changed |= self.app_list_editor(ui, &mut apps, "std_apps");
            self.policy.apps = apps;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("📂  Add installers…").clicked() {
                    for p in pick_installers() {
                        if !self
                            .policy
                            .apps
                            .iter()
                            .any(|a| a.path == p.display().to_string())
                        {
                            self.policy.apps.push(AppItem::from_path(&p));
                            changed = true;
                        }
                    }
                }
                changed |= ui
                    .checkbox(
                        &mut self.policy.apps_auto,
                        "Keep installed automatically (also on new PCs)",
                    )
                    .changed();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("💾  Save").clicked() && self.save_policy() {
                    self.toast("App package saved");
                }
                let ready = !self.policy.apps.is_empty()
                    && self
                        .policy
                        .apps
                        .iter()
                        .all(|a| std::path::Path::new(&a.path).is_file());
                install_to = self.apply_menu(ui, ready, "▶  Install package on…", selected, online);
            });
        });
        if changed {
            self.save_policy();
        }
        if let Some(ids) = install_to {
            if self.save_policy() {
                self.confirm = Some(Action::InstallApps {
                    apps: self.policy.apps.clone(),
                    ids,
                    package: true,
                });
            }
        }
    }

    fn names_card(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        selected: &[String],
        online: &[String],
    ) {
        let pal = self.pal;
        let mut open = None;
        w::card(ui, &pal, |ui| {
            ui.set_width(width - 30.0);
            w::card_title(ui, &pal, "PC names");
            w::faint(
                ui,
                &pal,
                "Rename PCs from a pattern such as ANSC-### (→ ANSC-001, ANSC-002, …). A single PC can \
                 also be renamed from its device page. Domain-joined PCs may need a domain admin instead.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Pattern");
                ui.add(
                    egui::TextEdit::singleline(&mut self.policy.name_pattern)
                        .hint_text("ANSC-###")
                        .desired_width(160.0),
                );
                ui.label("from");
                ui.add(egui::DragValue::new(&mut self.policy.name_start).range(0..=99999));
                let example =
                    policy::expand_pattern(&self.policy.name_pattern, self.policy.name_start);
                match policy::validate_pc_name(&example) {
                    Ok(()) => w::faint(ui, &pal, format!("e.g. {example}")),
                    Err(e) => ui.label(RichText::new(e).color(pal.red)),
                };
            });
            ui.add_space(8.0);
            open = self.apply_menu(ui, true, "✏  Rename…", selected, online);
        });
        if let Some(ids) = open {
            self.composer = Some(Composer {
                kind: ComposerKind::Rename,
                ids,
            });
        }
    }

    // -----------------------------------------------------------------------
    // Shared pieces (also used by the Install composer and confirmations)
    // -----------------------------------------------------------------------

    /// Editable list of installers. Returns true when something changed.
    pub(super) fn app_list_editor(
        &mut self,
        ui: &mut egui::Ui,
        apps: &mut Vec<AppItem>,
        salt: &str,
    ) -> bool {
        let pal = self.pal;
        let mut changed = false;
        if apps.is_empty() {
            w::faint(ui, &pal, "No installers yet — add .exe / .msi files.");
            return false;
        }
        let mut remove = None;
        let mut up = None;
        let n = apps.len();
        egui::Frame::none()
            .fill(pal.surface2)
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt(("apps", salt))
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (i, a) in apps.iter_mut().enumerate() {
                            ui.push_id((salt, i), |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("{}.", i + 1)).color(pal.text_faint));
                                    ui.label(
                                        RichText::new(super::truncate(&a.file_name(), 34))
                                            .font(semibold(13.5))
                                            .color(pal.text),
                                    )
                                    .on_hover_text(&a.path);
                                    w::pill(ui, &a.kind, pal.blue);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if w::ghost(ui, "✖").on_hover_text("Remove").clicked() {
                                            remove = Some(i);
                                        }
                                        if i > 0 && w::ghost(ui, "⬆").on_hover_text("Install earlier").clicked() {
                                            up = Some(i);
                                        }
                                    });
                                });
                                ui.horizontal(|ui| {
                                    ui.add_space(18.0);
                                    ui.label(RichText::new("args").color(pal.text_dim));
                                    changed |= ui
                                        .add(egui::TextEdit::singleline(&mut a.args).desired_width(210.0))
                                        .on_hover_text("Silent switches (auto-detected)")
                                        .changed();
                                    changed |= ui.checkbox(&mut a.is_msi, "MSI").changed();
                                    ui.label(RichText::new("skip if installed").color(pal.text_dim));
                                    changed |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut a.detect)
                                                .hint_text("program name")
                                                .desired_width(140.0),
                                        )
                                        .on_hover_text(
                                            "Skip PCs whose installed programs include a name containing this \
                                             (case-insensitive). Empty = always install.",
                                        )
                                        .changed();
                                });
                                let warn = if !std::path::Path::new(&a.path).is_file() {
                                    Some("File not found")
                                } else if a.kind == InstallerKind::Unknown.label() {
                                    InstallerKind::Unknown.warning()
                                } else if a.kind == InstallerKind::Squirrel.label() {
                                    InstallerKind::Squirrel.warning()
                                } else {
                                    None
                                };
                                if let Some(wn) = warn {
                                    ui.horizontal(|ui| {
                                        ui.add_space(18.0);
                                        ui.label(RichText::new(format!("⚠  {wn}")).size(12.0).color(pal.amber));
                                    });
                                }
                                if i + 1 < n {
                                    ui.separator();
                                }
                            });
                        }
                    });
            });
        if let Some(i) = remove {
            apps.remove(i);
            changed = true;
        }
        if let Some(i) = up {
            apps.swap(i, i - 1);
            changed = true;
        }
        changed
    }

    pub(super) fn add_installer_files_to_queue(&mut self) {
        for p in pick_installers() {
            if !self
                .install_queue
                .iter()
                .any(|a| a.path == p.display().to_string())
            {
                self.install_queue.push(AppItem::from_path(&p));
            }
        }
    }

    /// Read-only list of installers with their arguments.
    pub(super) fn app_summary(&self, ui: &mut egui::Ui, apps: &[AppItem]) {
        let pal = self.pal;
        egui::Frame::none()
            .fill(pal.surface2)
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("app_summary")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for (i, a) in apps.iter().enumerate() {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
                                ui.label(
                                    RichText::new(format!("{}. {}", i + 1, a.file_name()))
                                        .color(pal.text),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{}{}",
                                        if a.is_msi { "msiexec /i … " } else { "" },
                                        a.args
                                    ))
                                    .monospace()
                                    .color(pal.text_dim),
                                );
                            });
                        }
                    });
            });
    }

    /// "old → new" list of renames.
    pub(super) fn rename_preview(&self, ui: &mut egui::Ui, list: &[(String, String)]) {
        let pal = self.pal;
        egui::Frame::none()
            .fill(pal.surface2)
            .rounding(egui::Rounding::same(8.0))
            .inner_margin(egui::Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("rename_preview")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        egui::Grid::new("rename_preview_grid")
                            .num_columns(3)
                            .spacing([10.0, 3.0])
                            .show(ui, |ui| {
                                for (old, new) in list {
                                    ui.label(RichText::new(old).color(pal.text_dim));
                                    ui.label(RichText::new("→").color(pal.text_faint));
                                    ui.label(
                                        RichText::new(new).font(semibold(13.0)).color(pal.text),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            });
    }
}

/// Let the admin pick several installers at once.
fn pick_installers() -> Vec<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Installers", &["exe", "msi"])
        .pick_files()
        .unwrap_or_default()
}
