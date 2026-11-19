//! Dashboard: KPIs, alerts, resources, updates, recent jobs and activity.

use std::collections::BTreeMap;

use eframe::egui::{self, vec2, RichText};
use fleetview_shared::protocol::MasterMsg;

use super::{Action, App, DeviceRow, Page, Severity, StatusFilter};
use crate::agentpkg::UpdateState;
use crate::ui::theme::semibold;
use crate::ui::widgets as w;

impl App {
    pub(super) fn page_dashboard(&mut self, ui: &mut egui::Ui, rows: &[DeviceRow]) {
        let pal = self.pal;
        let online = rows.iter().filter(|r| r.online).count();
        let total = rows.len();
        let greeting = match chrono::Local::now().format("%H").to_string().parse::<u32>() {
            Ok(h) if h < 12 => "Good morning",
            Ok(h) if h < 18 => "Good afternoon",
            _ => "Good evening",
        };
        self.page_header(
            ui,
            "Dashboard",
            &format!("{greeting} — {online} of {total} PCs are online right now."),
            |app, ui| {
                if ui
                    .button("↻  Refresh all")
                    .on_hover_text("Ask every online PC for fresh telemetry and inventory")
                    .clicked()
                {
                    let ids = app.online_ids();
                    for id in &ids {
                        app.send_to(id, MasterMsg::RequestTelemetry);
                        app.send_to(id, MasterMsg::RequestSoftware);
                    }
                    app.toast(format!("Refresh requested from {} PC(s)", ids.len()));
                }
            },
        );

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // ---- KPI tiles -------------------------------------------------
                let attention: Vec<&DeviceRow> = rows
                    .iter()
                    .filter(|r| r.worst().is_some_and(|s| s >= Severity::Warn))
                    .collect();
                let outdated = rows
                    .iter()
                    .filter(|r| r.online && r.update == UpdateState::Outdated)
                    .count();
                let (running, failed_today) = {
                    let s = self.state.lock().unwrap();
                    let today = chrono::Local::now().date_naive();
                    let failed = s
                        .jobs
                        .iter()
                        .filter(|j| j.created.date_naive() == today)
                        .map(|j| j.counts().2)
                        .sum::<usize>();
                    (s.running_jobs(), failed)
                };

                let avail = ui.available_width();
                let cols = if avail > 900.0 { 4 } else { 2 };
                let gap = 12.0;
                let tile_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                let mut clicked: Option<u8> = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(gap, gap);
                    let pct = if total > 0 {
                        100.0 * online as f32 / total as f32
                    } else {
                        0.0
                    };
                    if w::stat_tile(
                        ui,
                        &pal,
                        tile_w,
                        "🖥",
                        "Online now",
                        &format!("{online} / {total}"),
                        &format!("{pct:.0}% of known PCs reachable"),
                        pal.green,
                    )
                    .clicked()
                    {
                        clicked = Some(0);
                    }
                    if w::stat_tile(
                        ui,
                        &pal,
                        tile_w,
                        "⚠",
                        "Needs attention",
                        &attention.len().to_string(),
                        "disk, memory, CPU or update problems",
                        if attention.is_empty() {
                            pal.green
                        } else {
                            pal.amber
                        },
                    )
                    .clicked()
                    {
                        clicked = Some(1);
                    }
                    if w::stat_tile(
                        ui,
                        &pal,
                        tile_w,
                        "⬆",
                        "Outdated agents",
                        &outdated.to_string(),
                        match &self.package {
                            Some(_) => "online PCs behind the agent package",
                            None => "no agent package loaded",
                        },
                        if outdated == 0 { pal.blue } else { pal.violet },
                    )
                    .clicked()
                    {
                        clicked = Some(2);
                    }
                    if w::stat_tile(
                        ui,
                        &pal,
                        tile_w,
                        "⚡",
                        "Jobs",
                        &format!("{running} running"),
                        &if failed_today > 0 {
                            format!("{failed_today} failed result(s) today")
                        } else {
                            "no failures today".to_string()
                        },
                        if failed_today > 0 { pal.red } else { pal.blue },
                    )
                    .clicked()
                    {
                        clicked = Some(3);
                    }
                });
                match clicked {
                    Some(0) => {
                        self.status_filter = StatusFilter::Online;
                        self.go(Page::Devices);
                    }
                    Some(1) => {
                        self.status_filter = StatusFilter::Attention;
                        self.go(Page::Devices);
                    }
                    Some(2) => {
                        self.status_filter = StatusFilter::Outdated;
                        self.go(Page::Devices);
                    }
                    Some(3) => self.go(Page::Jobs),
                    _ => {}
                }
                ui.add_space(gap);

                // ---- Two columns -----------------------------------------------
                let wide = avail > 900.0;
                let left_w = if wide { (avail - gap) * 0.58 } else { avail };
                let right_w = if wide { avail - gap - left_w } else { avail };

                let left = |app: &mut App, ui: &mut egui::Ui| {
                    app.attention_card(ui, rows);
                    ui.add_space(gap);
                    app.resources_card(ui, rows);
                };
                let right = |app: &mut App, ui: &mut egui::Ui| {
                    app.updates_card(ui, rows);
                    ui.add_space(gap);
                    app.os_card(ui, rows);
                    ui.add_space(gap);
                    app.recent_jobs_card(ui);
                };

                if wide {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.allocate_ui_with_layout(
                            vec2(left_w, 0.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                ui.set_width(left_w);
                                left(self, ui)
                            },
                        );
                        ui.allocate_ui_with_layout(
                            vec2(right_w, 0.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                ui.set_width(right_w);
                                right(self, ui)
                            },
                        );
                    });
                } else {
                    left(self, ui);
                    ui.add_space(gap);
                    right(self, ui);
                }
                ui.add_space(gap);
                self.recent_activity_card(ui);
            });
    }

    fn attention_card(&mut self, ui: &mut egui::Ui, rows: &[DeviceRow]) {
        let pal = self.pal;
        let mut items: Vec<(&DeviceRow, &super::Alert)> = rows
            .iter()
            .flat_map(|r| r.alerts.iter().map(move |a| (r, a)))
            .collect();
        items.sort_by(|a, b| {
            b.1.severity
                .cmp(&a.1.severity)
                .then_with(|| a.0.host.cmp(&b.0.host))
        });
        let mut open: Option<String> = None;
        let mut more = false;
        w::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                w::card_title(ui, &pal, "Needs attention");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    w::faint(ui, &pal, format!("{} item(s)", items.len()));
                });
            });
            ui.add_space(6.0);
            if items.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✔").color(pal.green).size(16.0));
                    w::subtle(
                        ui,
                        &pal,
                        "All clear — no disk, memory, CPU or update issues.",
                    );
                });
                return;
            }
            const SHOWN: usize = 10;
            {
                {
                    for (r, a) in items.iter().take(SHOWN) {
                        let color = match a.severity {
                            Severity::Crit => pal.red,
                            Severity::Warn => pal.amber,
                            Severity::Info => pal.blue,
                        };
                        let resp = ui
                            .horizontal(|ui| {
                                w::dot(ui, color, false);
                                ui.label(
                                    RichText::new(&r.host).font(semibold(13.0)).color(pal.text),
                                );
                                ui.label(RichText::new(&a.text).color(pal.text_dim));
                            })
                            .response
                            .interact(egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            open = Some(r.id.clone());
                        }
                    }
                }
            }
            if items.len() > SHOWN
                && w::ghost(ui, &format!("+{} more →", items.len() - SHOWN)).clicked()
            {
                more = true;
            }
        });
        if more {
            self.status_filter = StatusFilter::Attention;
            self.go(Page::Devices);
        }
        if let Some(id) = open {
            self.open_device(&id);
        }
    }

    fn resources_card(&mut self, ui: &mut egui::Ui, rows: &[DeviceRow]) {
        let pal = self.pal;
        let live: Vec<&DeviceRow> = rows
            .iter()
            .filter(|r| r.online && r.cpu.is_some())
            .collect();
        let avg = |f: &dyn Fn(&DeviceRow) -> Option<f32>| -> Option<f32> {
            let v: Vec<f32> = live.iter().filter_map(|r| f(r)).collect();
            (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
        };
        let cpu = avg(&|r| r.cpu);
        let mem = avg(&|r| r.mem);
        let disk = avg(&|r| r.disk.as_ref().map(|d| d.1));
        w::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            w::card_title(ui, &pal, "Fleet resources (online average)");
            ui.add_space(8.0);
            if live.is_empty() {
                w::subtle(ui, &pal, "No telemetry yet.");
                return;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 26.0;
                for (label, v, warn) in [
                    ("CPU", cpu, self.prefs.cpu_warn_pct),
                    ("Memory", mem, self.prefs.mem_warn_pct),
                    ("Fullest disk", disk, self.prefs.disk_warn_pct),
                ] {
                    ui.vertical(|ui| {
                        let v = v.unwrap_or(0.0);
                        w::ring(
                            ui,
                            &pal,
                            v / 100.0,
                            pal.usage(v, warn),
                            86.0,
                            &format!("{v:.0}%"),
                        );
                        ui.add_space(2.0);
                        ui.label(RichText::new(label).color(pal.text_dim));
                    });
                }
            });
            ui.add_space(10.0);
            // Busiest PCs
            let mut busy: Vec<&&DeviceRow> = live.iter().collect();
            busy.sort_by(|a, b| b.cpu.unwrap_or(0.0).total_cmp(&a.cpu.unwrap_or(0.0)));
            w::card_title(ui, &pal, "Busiest right now");
            ui.add_space(4.0);
            let mut open = None;
            egui::Grid::new("busiest")
                .num_columns(3)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    for r in busy.iter().take(5) {
                        if ui.link(RichText::new(&r.host).color(pal.text)).clicked() {
                            open = Some(r.id.clone());
                        }
                        let c = r.cpu.unwrap_or(0.0);
                        w::meter(
                            ui,
                            &pal,
                            c / 100.0,
                            pal.usage(c, self.prefs.cpu_warn_pct),
                            140.0,
                            &format!("{c:>3.0}% CPU"),
                        );
                        w::sparkline(
                            ui,
                            &pal,
                            &tail(&r.cpu_hist, 40),
                            vec2(110.0, 22.0),
                            pal.accent,
                            100.0,
                        );
                        ui.end_row();
                    }
                });
            if let Some(id) = open {
                self.open_device(&id);
            }
        });
    }

    fn updates_card(&mut self, ui: &mut egui::Ui, rows: &[DeviceRow]) {
        let pal = self.pal;
        let outdated: Vec<String> = rows
            .iter()
            .filter(|r| r.online && !r.updating && r.update == UpdateState::Outdated)
            .map(|r| r.id.clone())
            .collect();
        let updatable: Vec<String> = rows
            .iter()
            .filter(|r| r.online && !r.updating && r.update != UpdateState::Newer)
            .map(|r| r.id.clone())
            .collect();
        let current = rows
            .iter()
            .filter(|r| r.update == UpdateState::Current)
            .count();
        let updating = rows.iter().filter(|r| r.updating).count();
        w::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            w::card_title(ui, &pal, "Agent updates");
            ui.add_space(6.0);
            match &self.package {
                Some(p) => {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Package {}", p.version))
                                .font(semibold(15.0))
                                .color(pal.text),
                        );
                        w::pill(ui, &format!("{current} current"), pal.green);
                        if !outdated.is_empty() {
                            w::pill(ui, &format!("{} outdated", outdated.len()), pal.amber);
                        }
                        if updating > 0 {
                            w::pill(ui, &format!("{updating} updating"), pal.blue);
                        }
                    });
                    w::faint(ui, &pal, p.path.display().to_string());
                }
                None => {
                    ui.label(RichText::new("No agent package loaded").color(pal.amber));
                    w::faint(ui, &pal, &self.package_error);
                }
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if w::primary_enabled(
                    ui,
                    &pal,
                    !outdated.is_empty(),
                    &format!("⬆  Update {} outdated", outdated.len()),
                )
                .clicked()
                {
                    self.confirm = Some(Action::Update {
                        ids: outdated.clone(),
                    });
                }
                if ui
                    .add_enabled(
                        self.package.is_some() && !updatable.is_empty(),
                        egui::Button::new(format!("⟳  Update/repair all {}", updatable.len())),
                    )
                    .on_hover_text("Push the package to every online PC (never downgrades)")
                    .clicked()
                {
                    self.confirm = Some(Action::Update {
                        ids: updatable.clone(),
                    });
                }
                if ui.button("Choose package…").clicked() {
                    self.choose_package();
                }
            });
        });
    }

    fn os_card(&mut self, ui: &mut egui::Ui, rows: &[DeviceRow]) {
        let pal = self.pal;
        let mut by_os: BTreeMap<String, usize> = BTreeMap::new();
        for r in rows {
            let os = if r.os.is_empty() {
                "Unknown".to_string()
            } else {
                r.os.clone()
            };
            *by_os.entry(os).or_default() += 1;
        }
        let mut list: Vec<(String, usize)> = by_os.into_iter().collect();
        list.sort_by_key(|x| std::cmp::Reverse(x.1));
        let max = list.first().map(|x| x.1).unwrap_or(1).max(1);
        let colors = [
            pal.accent, pal.violet, pal.blue, pal.green, pal.amber, pal.red,
        ];
        w::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            w::card_title(ui, &pal, "Operating systems");
            ui.add_space(6.0);
            if list.is_empty() {
                w::subtle(ui, &pal, "No PCs yet.");
            }
            let bar_w = (ui.available_width() - 190.0).max(60.0);
            egui::Grid::new("os_grid")
                .num_columns(3)
                .spacing([10.0, 6.0])
                .show(ui, |ui| {
                    for (i, (os, n)) in list.iter().take(6).enumerate() {
                        ui.label(RichText::new(super::truncate(os, 24)).color(pal.text_dim));
                        w::bar(
                            ui,
                            &pal,
                            *n as f32 / max as f32,
                            colors[i % colors.len()],
                            bar_w,
                        );
                        ui.label(
                            RichText::new(n.to_string())
                                .font(semibold(13.0))
                                .color(pal.text),
                        );
                        ui.end_row();
                    }
                });
        });
    }

    fn recent_jobs_card(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let jobs: Vec<crate::state::Job> = self
            .state
            .lock()
            .unwrap()
            .jobs
            .iter()
            .take(5)
            .cloned()
            .collect();
        let mut open = None;
        w::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                w::card_title(ui, &pal, "Recent jobs");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::ghost(ui, "View all →").clicked() {
                        open = Some(None);
                    }
                });
            });
            ui.add_space(4.0);
            if jobs.is_empty() {
                w::subtle(ui, &pal, "Nothing sent yet.");
            }
            for j in &jobs {
                let (pending, ok, failed, skipped) = j.counts();
                let resp = ui
                    .horizontal(|ui| {
                        let color = if pending > 0 {
                            pal.blue
                        } else if failed > 0 {
                            pal.red
                        } else {
                            pal.green
                        };
                        w::dot(ui, color, pending > 0);
                        ui.label(RichText::new(super::truncate(&j.title, 34)).color(pal.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            w::faint(ui, &pal, w::fmt_ago(j.created));
                            let mut s = format!("{ok}/{}", j.targets.len() - skipped);
                            if failed > 0 {
                                s.push_str(&format!(" · {failed} failed"));
                            }
                            ui.label(RichText::new(s).size(12.0).color(pal.text_dim));
                        });
                    })
                    .response
                    .interact(egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    open = Some(Some(j.id));
                }
            }
        });
        if let Some(j) = open {
            if j.is_some() {
                self.selected_job = j;
            }
            self.go(Page::Jobs);
        }
    }

    fn recent_activity_card(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let events: Vec<(chrono::DateTime<chrono::Local>, String)> = self
            .state
            .lock()
            .unwrap()
            .events
            .iter()
            .take(8)
            .cloned()
            .collect();
        let mut more = false;
        w::card(ui, &pal, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                w::card_title(ui, &pal, "Recent activity");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::ghost(ui, "Open log →").clicked() {
                        more = true;
                    }
                });
            });
            ui.add_space(4.0);
            for (at, msg) in &events {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(at.format("%H:%M:%S").to_string())
                            .monospace()
                            .color(pal.text_faint),
                    );
                    ui.label(RichText::new(msg).color(event_color(&pal, msg)));
                });
            }
        });
        if more {
            self.go(Page::Activity);
        }
    }
}

pub(super) fn event_color(pal: &crate::ui::theme::Palette, msg: &str) -> egui::Color32 {
    let m = msg.to_lowercase();
    if m.contains("fail") || m.contains("error") || m.contains("rejected") || m.contains("[error]")
    {
        pal.red
    } else if m.contains("went offline")
        || m.contains("warn")
        || m.contains("not delivered")
        || m.contains("disconnected")
    {
        pal.amber
    } else if m.contains("connected from") || m.contains("updated") {
        pal.green
    } else {
        pal.text_dim
    }
}

pub(super) fn tail(v: &[f32], n: usize) -> Vec<f32> {
    v[v.len().saturating_sub(n)..].to_vec()
}
