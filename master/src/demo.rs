//! Demo mode (`fleetview-master --demo`): a simulated fleet that answers
//! commands, for screenshots, UI work and trying the app without agents.
//! It uses a separate data folder and never opens network ports.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Local;
use tokio::sync::mpsc;

use fleetview_shared::protocol::{
    AgentMsg, CommandResult, DiskInfo, MasterMsg, SoftwareEntry, Telemetry,
};

use crate::server::apply_agent_msg;
use crate::state::{AgentView, Sample, SharedState};

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--demo") || std::env::var_os("FLEETVIEW_DEMO").is_some()
}

/// Point the data folder at a throwaway location (call before anything reads it).
pub fn use_demo_data_dir() {
    if std::env::var_os("FLEETVIEW_DATA_DIR").is_none() {
        let dir = std::env::temp_dir().join("FleetView-demo");
        std::env::set_var("FLEETVIEW_DATA_DIR", dir);
    }
}

static SEED: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

fn rnd() -> f32 {
    // xorshift64*
    let mut x = SEED.load(Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    SEED.store(x, Ordering::Relaxed);
    ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
}

struct Profile {
    host: &'static str,
    user: &'static str,
    online: bool,
    version: &'static str,
    os: &'static str,
    cpu_base: f32,
    mem_total: u64,
    mem_base: f32,
    disk_used: f64,
    tags: &'static [&'static str],
}

const PROFILES: &[Profile] = &[
    Profile {
        host: "ACC-PC01",
        user: "ana.popescu",
        online: true,
        version: "0.3.1",
        os: "Windows 11 Pro",
        cpu_base: 12.0,
        mem_total: 16384,
        mem_base: 0.55,
        disk_used: 0.62,
        tags: &["Accounting"],
    },
    Profile {
        host: "ACC-PC02",
        user: "ion.rusu",
        online: true,
        version: "0.3.1",
        os: "Windows 11 Pro",
        cpu_base: 35.0,
        mem_total: 8192,
        mem_base: 0.82,
        disk_used: 0.955,
        tags: &["Accounting"],
    },
    Profile {
        host: "ACC-PC03",
        user: "",
        online: false,
        version: "0.3.0",
        os: "Windows 10 Pro",
        cpu_base: 8.0,
        mem_total: 8192,
        mem_base: 0.4,
        disk_used: 0.5,
        tags: &["Accounting"],
    },
    Profile {
        host: "HR-LAPTOP01",
        user: "maria.ciobanu",
        online: true,
        version: "0.3.0",
        os: "Windows 11 Pro",
        cpu_base: 18.0,
        mem_total: 16384,
        mem_base: 0.61,
        disk_used: 0.71,
        tags: &["HR", "Laptop"],
    },
    Profile {
        host: "HR-PC02",
        user: "elena.lungu",
        online: true,
        version: "0.3.1",
        os: "Windows 10 Pro",
        cpu_base: 6.0,
        mem_total: 8192,
        mem_base: 0.47,
        disk_used: 0.44,
        tags: &["HR"],
    },
    Profile {
        host: "RECEPTION",
        user: "front.desk",
        online: true,
        version: "0.3.1",
        os: "Windows 10 Pro",
        cpu_base: 97.0,
        mem_total: 4096,
        mem_base: 0.93,
        disk_used: 0.8,
        tags: &["Ground floor"],
    },
    Profile {
        host: "DIR-PC",
        user: "director",
        online: true,
        version: "0.3.1",
        os: "Windows 11 Pro",
        cpu_base: 9.0,
        mem_total: 32768,
        mem_base: 0.33,
        disk_used: 0.38,
        tags: &["Management"],
    },
    Profile {
        host: "IT-WS01",
        user: "admin",
        online: true,
        version: "0.3.1",
        os: "Windows 11 Enterprise",
        cpu_base: 22.0,
        mem_total: 32768,
        mem_base: 0.48,
        disk_used: 0.66,
        tags: &["IT"],
    },
    Profile {
        host: "LEGAL-PC01",
        user: "victor.bostan",
        online: true,
        version: "0.3.0",
        os: "Windows 10 Pro",
        cpu_base: 14.0,
        mem_total: 8192,
        mem_base: 0.58,
        disk_used: 0.92,
        tags: &["Legal"],
    },
    Profile {
        host: "LEGAL-PC02",
        user: "",
        online: false,
        version: "0.3.1",
        os: "Windows 11 Pro",
        cpu_base: 5.0,
        mem_total: 8192,
        mem_base: 0.4,
        disk_used: 0.52,
        tags: &["Legal"],
    },
    Profile {
        host: "ARCHIVE-PC",
        user: "",
        online: false,
        version: "0.2.4",
        os: "Windows 10 Pro",
        cpu_base: 3.0,
        mem_total: 4096,
        mem_base: 0.4,
        disk_used: 0.88,
        tags: &[],
    },
    Profile {
        host: "MEETING-ROOM",
        user: "",
        online: true,
        version: "0.3.1",
        os: "Windows 11 Pro",
        cpu_base: 4.0,
        mem_total: 8192,
        mem_base: 0.31,
        disk_used: 0.29,
        tags: &["Ground floor"],
    },
    Profile {
        host: "SALES-PC01",
        user: "dan.moraru",
        online: true,
        version: "0.3.1",
        os: "Windows 11 Pro",
        cpu_base: 27.0,
        mem_total: 16384,
        mem_base: 0.69,
        disk_used: 0.57,
        tags: &["Sales"],
    },
    Profile {
        host: "SALES-PC02",
        user: "irina.sandu",
        online: true,
        version: "0.3.1",
        os: "Windows 11 Pro",
        cpu_base: 16.0,
        mem_total: 16384,
        mem_base: 0.52,
        disk_used: 0.63,
        tags: &["Sales"],
    },
];

fn software_for(i: usize) -> Vec<SoftwareEntry> {
    let e = |n: &str, v: &str, p: &str, u: &str| SoftwareEntry {
        name: n.into(),
        version: Some(v.into()),
        publisher: Some(p.into()),
        uninstall_string: Some(u.into()),
    };
    let mut list = vec![
        e("Google Chrome", if i.is_multiple_of(3) { "130.0.6723.70" } else { "131.0.6778.86" }, "Google LLC", "\"C:\\Program Files\\Google\\Chrome\\Application\\setup.exe\" --uninstall"),
        e("Microsoft Edge", "131.0.2903.51", "Microsoft Corporation", ""),
        e("7-Zip 24.08 (x64)", "24.08", "Igor Pavlov", "\"C:\\Program Files\\7-Zip\\Uninstall.exe\""),
        e("Microsoft 365 Apps for business - ro-ro", "16.0.18129.20116", "Microsoft Corporation", "\"C:\\Program Files\\Common Files\\Microsoft Shared\\ClickToRun\\OfficeClickToRun.exe\" scenario=install"),
        e("Adobe Acrobat Reader (64-bit)", if i.is_multiple_of(4) { "24.003.20180" } else { "24.004.20220" }, "Adobe", "MsiExec.exe /I{AC76BA86-1033-FF00-7760-BC15014EA700}"),
        e("FleetView Agent", "0.3.1", "Akulav", ""),
        e("Microsoft Visual C++ 2015-2022 Redistributable (x64)", "14.40.33810.0", "Microsoft Corporation", "MsiExec.exe /X{3A1F7E9C-2D1B-4A17-9A2B-0D1F5E1C7A11}"),
        e("Notepad++ (64-bit x64)", "8.7.1", "Notepad++ Team", "\"C:\\Program Files\\Notepad++\\uninstall.exe\""),
    ];
    if i.is_multiple_of(2) {
        list.push(e(
            "MoldSign Desktop",
            "3.2.1",
            "STISC",
            "MsiExec.exe /X{5B6E2F3A-1C4D-4E5F-8A9B-0C1D2E3F4A5B}",
        ));
    }
    if i % 3 == 1 {
        list.push(e(
            "Zoom Workplace",
            "6.2.10",
            "Zoom Video Communications",
            "\"C:\\Users\\Public\\Zoom\\uninstall\\Installer.exe\" /uninstall",
        ));
    }
    if i % 5 == 2 {
        list.push(e(
            "1C:Enterprise 8 (x86-64)",
            "8.3.24.1548",
            "1C-Soft",
            "MsiExec.exe /X{1C8E1C8E-0000-4000-8000-000000000001}",
        ));
    }
    if i == 7 {
        list.push(e(
            "Wireshark 4.4.1 x64",
            "4.4.1",
            "The Wireshark developer community",
            "\"C:\\Program Files\\Wireshark\\uninstall-wireshark.exe\"",
        ));
        list.push(e(
            "PuTTY release 0.81 (64-bit)",
            "0.81.0.0",
            "Simon Tatham",
            "MsiExec.exe /X{7B1E2C3D-4E5F-4A6B-8C7D-9E0F1A2B3C4D}",
        ));
    }
    list
}

fn telemetry_for(p: &Profile, i: usize, jitter: bool) -> Telemetry {
    let j = |base: f32, spread: f32| -> f32 {
        if jitter {
            (base + (rnd() - 0.5) * spread).clamp(0.5, 100.0)
        } else {
            base
        }
    };
    let total_disk = if i.is_multiple_of(2) { 476.0 } else { 237.0 };
    let mut disks = vec![DiskInfo {
        mount: "C:\\".into(),
        total_gb: total_disk,
        available_gb: total_disk * (1.0 - p.disk_used),
    }];
    if i % 4 == 1 {
        disks.push(DiskInfo {
            mount: "D:\\".into(),
            total_gb: 931.0,
            available_gb: 512.0,
        });
    }
    Telemetry {
        hostname: p.host.into(),
        os_version: format!(
            "{} 10.0.{}",
            p.os,
            if p.os.contains("11") { 22631 } else { 19045 }
        ),
        logged_in_user: (!p.user.is_empty()).then(|| p.user.to_string()),
        uptime_secs: 3600 * (2 + i as u64 * 7) + 1234,
        cpu_usage_pct: j(p.cpu_base, if p.cpu_base > 90.0 { 5.0 } else { 18.0 }),
        cpu_name: if i.is_multiple_of(3) {
            "Intel(R) Core(TM) i5-12400".into()
        } else {
            "AMD Ryzen 5 5600G with Radeon Graphics".into()
        },
        cpu_cores: 12,
        mem_total_mb: p.mem_total,
        mem_used_mb: (p.mem_total as f32 * j(p.mem_base * 100.0, 6.0) / 100.0) as u64,
        disks,
        ip_addresses: vec![
            format!("192.168.10.{}", 20 + i),
            format!("fe80::1c2b:{:x}:9a1f:{:x}", i * 17, i * 31),
        ],
        mac_addresses: vec![format!(
            "3C:52:82:1A:{:02X}:{:02X}",
            i * 7 % 256,
            i * 13 % 256
        )],
        agent_version: p.version.into(),
    }
}

/// Seed the state with the demo fleet and start the simulators.
pub fn start(state: SharedState) {
    let now = Local::now();
    {
        let mut s = state.lock().unwrap();
        s.listening = true;
        for (i, p) in PROFILES.iter().enumerate() {
            let id = format!("demo-{i:02}");
            let mut a = AgentView::new(
                id.clone(),
                p.host.into(),
                now - chrono::Duration::days(40 + i as i64),
            );
            a.agent_version = p.version.into();
            a.tags = p.tags.iter().map(|t| t.to_string()).collect();
            let t = telemetry_for(p, i, false);
            // History: two hours of samples.
            for k in 0..120 {
                let at = now - chrono::Duration::seconds(30 * (120 - k));
                let wave = ((k as f32 + i as f32 * 9.0) / 9.0).sin() * 10.0;
                a.history.push_back(Sample {
                    at,
                    cpu: (p.cpu_base + wave + (rnd() - 0.5) * 14.0).clamp(0.5, 100.0),
                    mem: (p.mem_base * 100.0 + wave / 3.0 + (rnd() - 0.5) * 4.0).clamp(1.0, 100.0),
                });
            }
            a.last_ips = t.ip_addresses.clone();
            a.last_macs = t.mac_addresses.clone();
            a.last_os = t.os_version.clone();
            a.last_user = t.logged_in_user.clone();
            a.software = software_for(i);
            if p.online {
                a.telemetry = Some(t);
                a.online = true;
                let (tx, rx) = mpsc::unbounded_channel::<MasterMsg>();
                a.tx = Some(tx);
                let state = state.clone();
                let id2 = id.clone();
                std::thread::spawn(move || responder(state, id2, i, rx));
            } else {
                a.last_seen = now - chrono::Duration::days([3, 21, 64][i % 3]);
                a.last_user = None;
            }
            s.agents.insert(id, a);
        }
        s.log("demo mode: simulated fleet loaded (nothing is sent over the network)");
        s.log("RECEPTION (demo-05) connected from 192.168.10.25:51022");
        s.log("discovery query from SALES-PC02 (192.168.10.33:48621)");
    }
    // A couple of finished jobs so the Jobs page has something to show.
    {
        let st = state.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            for (script, title) in [
                ("gpupdate /force", "PowerShell: gpupdate /force"),
                (
                    "Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 5",
                    "PowerShell: Get-Process | Sort-Object WorkingSet64…",
                ),
            ] {
                let mut s = st.lock().unwrap();
                let ids: Vec<String> = s.agents.keys().cloned().collect();
                let job = s.create_job(crate::state::JobKind::Script, title, script, &ids);
                for (n, id) in ids.iter().enumerate() {
                    let cid = format!("demo-cmd-{job}-{n}");
                    let sent = s.agents.get(id).is_some_and(|a| {
                        a.send(MasterMsg::RunPowerShell {
                            command_id: cid.clone(),
                            script: script.into(),
                        })
                    });
                    if sent {
                        s.attach_command(job, id, &cid);
                    } else {
                        s.finish_target(
                            job,
                            id,
                            crate::state::TargetStatus::Skipped,
                            "offline — not sent",
                        );
                    }
                }
            }
        });
    }
    // Telemetry ticker.
    let st = state.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        for (i, p) in PROFILES.iter().enumerate() {
            if !p.online {
                continue;
            }
            let t = telemetry_for(p, i, true);
            apply_agent_msg(AgentMsg::Telemetry(t), &format!("demo-{i:02}"), &st);
        }
    });
}

fn result(id: &str, ok: bool, code: i32, out: &str, err: &str) -> AgentMsg {
    AgentMsg::CommandResult(CommandResult {
        command_id: id.into(),
        success: ok,
        exit_code: Some(code),
        stdout: out.into(),
        stderr: err.into(),
    })
}

fn responder(state: SharedState, id: String, i: usize, mut rx: mpsc::UnboundedReceiver<MasterMsg>) {
    let p = &PROFILES[i];
    let mut streaming: Option<std::sync::Arc<std::sync::atomic::AtomicBool>> = None;
    let mut transfers: std::collections::HashMap<
        String,
        (u64, Option<fleetview_shared::protocol::RunSpec>, String),
    > = Default::default();
    while let Some(msg) = rx.blocking_recv() {
        let send = |m: AgentMsg| apply_agent_msg(m, &id, &state);
        match msg {
            MasterMsg::RequestTelemetry => send(AgentMsg::Telemetry(telemetry_for(p, i, true))),
            MasterMsg::RequestSoftware => send(AgentMsg::Software(software_for(i))),
            MasterMsg::RunPowerShell { command_id, script } => {
                let st = state.clone();
                let id2 = id.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(300 + (rnd() * 2200.0) as u64));
                    let lower = script.to_lowercase();
                    let msg = if lower.contains("throw") || (i == 5 && !lower.contains("shutdown"))
                    {
                        result(
                            &command_id,
                            false,
                            1,
                            "",
                            "Access to the path 'C:\\ProgramData\\Vendor\\lock' is denied.",
                        )
                    } else if lower.contains("shutdown") {
                        result(&command_id, true, 0, "", "")
                    } else if lower.contains("get-process") {
                        result(&command_id, true, 0, "Name          Id   MB\n----          --   --\nchrome      4312  812\nOUTLOOK     6120  455\nTeams       2216  390\nMsMpEng     3104  210\nexplorer    5520  140", "")
                    } else {
                        result(
                            &command_id,
                            true,
                            0,
                            &format!(
                                "{}\nCompleted on {} at {}",
                                "OK",
                                PROFILES[i].host,
                                Local::now().format("%H:%M:%S")
                            ),
                            "",
                        )
                    };
                    apply_agent_msg(msg, &id2, &st);
                });
            }
            MasterMsg::InstallFromUrl {
                command_id, url, ..
            } => {
                std::thread::sleep(Duration::from_millis(800));
                send(result(
                    &command_id,
                    true,
                    0,
                    &format!("downloaded {url}\ninstaller exited with 0"),
                    "",
                ));
            }
            MasterMsg::FileBegin {
                transfer_id,
                file_name,
                run_after,
                ..
            } => {
                transfers.insert(transfer_id, (0, run_after, file_name));
            }
            MasterMsg::FileChunk { transfer_id, data } => {
                if let Some(t) = transfers.get_mut(&transfer_id) {
                    t.0 += data.len() as u64;
                    send(AgentMsg::FileAck {
                        transfer_id,
                        received: t.0,
                    });
                }
            }
            MasterMsg::FileEnd { transfer_id } => {
                if let Some((_, Some(run), name)) = transfers.remove(&transfer_id) {
                    send(result(
                        &run.command_id,
                        true,
                        0,
                        &format!("{name} installed"),
                        "",
                    ));
                }
            }
            MasterMsg::SetConfig {
                command_id,
                updates,
            } => {
                let keys: Vec<String> = updates.iter().map(|u| u.key.clone()).collect();
                send(result(
                    &command_id,
                    true,
                    0,
                    &format!("updated: {}; restarting", keys.join(", ")),
                    "",
                ));
            }
            MasterMsg::NotifyUser { title, .. } => {
                send(AgentMsg::Log {
                    level: "info".into(),
                    message: format!("notification shown: {title}"),
                });
            }
            MasterMsg::RequestScreenshot { .. } => send(frame(p.host, i)),
            MasterMsg::StartScreenStream { interval_ms, .. } => {
                let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
                streaming = Some(flag.clone());
                let st = state.clone();
                let id2 = id.clone();
                std::thread::spawn(move || {
                    while flag.load(Ordering::Relaxed) {
                        apply_agent_msg(frame(PROFILES[i].host, i), &id2, &st);
                        std::thread::sleep(Duration::from_millis(interval_ms.max(100)));
                    }
                });
            }
            MasterMsg::StopScreenStream => {
                if let Some(f) = streaming.take() {
                    f.store(false, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
}

/// A fake desktop: wallpaper gradient, a taskbar, a couple of windows, and a
/// moving clock bar so live view visibly updates.
fn frame(host: &str, i: usize) -> AgentMsg {
    let (w, h) = (1280u32, 720u32);
    let t = Local::now();
    let phase = (t.timestamp_millis() % 10_000) as f32 / 10_000.0;
    let mut img = image::RgbImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        *px = image::Rgb([
            (20.0 + 40.0 * fy + 10.0 * i as f32 % 30.0) as u8,
            (60.0 + 80.0 * fx) as u8,
            (120.0 + 100.0 * (1.0 - fy)) as u8,
        ]);
    }
    let mut rect = |x0: u32, y0: u32, rw: u32, rh: u32, c: [u8; 3]| {
        for y in y0..(y0 + rh).min(h) {
            for x in x0..(x0 + rw).min(w) {
                img.put_pixel(x, y, image::Rgb(c));
            }
        }
    };
    rect(0, h - 48, w, 48, [32, 32, 38]);
    rect(160, 90, 620, 420, [245, 245, 248]);
    rect(160, 90, 620, 34, [225, 228, 236]);
    for k in 0..9 {
        rect(190, 150 + k * 36, 380 + (k * 53 % 170), 14, [200, 204, 214]);
    }
    rect(700, 230, 440, 320, [250, 250, 252]);
    rect(700, 230, 440, 34, [60, 110, 220]);
    rect(730, 300, (380.0 * phase) as u32 + 4, 18, [60, 180, 120]);
    rect(w - 120, h - 36, 90, 24, [70, 70, 80]);
    let _ = host;
    let mut jpeg = Vec::new();
    let _ = image::DynamicImage::ImageRgb8(img).write_to(
        &mut std::io::Cursor::new(&mut jpeg),
        image::ImageFormat::Jpeg,
    );
    AgentMsg::ScreenFrame {
        command_id: None,
        width: w,
        height: h,
        jpeg,
    }
}
