//! User-session helper process (`fleetview-agent helper --ipc-port N --ipc-token T`).
//!
//! Launched by the service into the interactive session. Responsibilities:
//!   * show the startup splash (transparency: the user sees the agent running),
//!   * own the always-visible tray icon,
//!   * perform screen capture on request (only the interactive session can),
//!   * show notifications the master pushes.
//!
//! It talks to the service over the authenticated loopback channel. The tray /
//! Win32 message loop runs on the main thread; a background tokio runtime runs
//! the IPC client. They communicate via a std channel.

use std::sync::mpsc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc as tmpsc;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::ipc::{HelperCommand, HelperEvent};
use crate::tray_i18n::{self, TRAY_LANG};

use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MessageBoxW, PeekMessageW, TranslateMessage, MB_ICONINFORMATION, MB_OK,
    MB_SETFOREGROUND, MB_TOPMOST, MSG, PM_REMOVE,
};

enum UiMsg {
    Notify { title: String, body: String },
    ShowSplash,
    Status { connected: bool, master: String },
}

pub fn run() {
    let (port, token) = parse_args();
    init_logging();
    tracing::info!("helper starting (ipc port {port})");

    let (ui_tx, ui_rx) = mpsc::channel::<UiMsg>();

    if port != 0 && !token.is_empty() {
        let ui_tx2 = ui_tx.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("helper runtime: {e}");
                    return;
                }
            };
            rt.block_on(ipc_client(port, token, ui_tx2));
        });
    }

    // Show the splash right away on logon.
    std::thread::spawn(|| crate::splash::show_blocking());

    tray_loop(ui_rx);
}

fn parse_args() -> (u16, String) {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 0u16;
    let mut token = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ipc-port" => {
                if let Some(v) = args.get(i + 1) {
                    port = v.parse().unwrap_or(0);
                    i += 1;
                }
            }
            "--ipc-token" => {
                if let Some(v) = args.get(i + 1) {
                    token = v.clone();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (port, token)
}

fn init_logging() {
    if let Some(dir) = dirs_local_app() {
        let logdir = dir.join("FleetView");
        let _ = std::fs::create_dir_all(&logdir);
        let appender = tracing_appender::rolling::daily(logdir, "helper.log");
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(appender)
            .try_init();
    }
}

fn dirs_local_app() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
}

// ---------------------------------------------------------------------------
// IPC client (runs on the tokio thread)
// ---------------------------------------------------------------------------

async fn ipc_client(port: u16, token: String, ui_tx: mpsc::Sender<UiMsg>) {
    loop {
        match ipc_session(port, &token, &ui_tx).await {
            Ok(()) => tracing::info!("ipc session closed; reconnecting"),
            Err(e) => tracing::warn!("ipc session error: {e:#}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn ipc_session(
    port: u16,
    token: &str,
    ui_tx: &mpsc::Sender<UiMsg>,
) -> anyhow::Result<()> {
    let stream = TcpStream::connect(("127.0.0.1", port)).await?;
    stream.set_nodelay(true).ok();
    let mut framed = Framed::new(
        stream,
        LengthDelimitedCodec::builder()
            .max_frame_length(fleetview_shared::MAX_FRAME_BYTES)
            .new_codec(),
    );

    // Authenticate, then say hello with the logged-in user.
    framed.send(bytes::Bytes::from(token.as_bytes().to_vec())).await?;
    let hello = HelperEvent::Hello { username: current_user() };
    framed.send(bytes::Bytes::from(rmp_serde::to_vec_named(&hello)?)).await?;

    // Outbound queue for screen frames produced by capture tasks.
    let (out_tx, mut out_rx) = tmpsc::channel::<HelperEvent>(16);

    // Current screen-stream task (if any).
    let mut stream_task: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        tokio::select! {
            Some(ev) = out_rx.recv() => {
                framed.send(bytes::Bytes::from(rmp_serde::to_vec_named(&ev)?)).await?;
            }
            incoming = framed.next() => {
                let Some(frame) = incoming else { break };
                let frame = frame?;
                let cmd: HelperCommand = rmp_serde::from_slice(&frame)?;
                match cmd {
                    HelperCommand::CaptureOnce { command_id } => {
                        let out = out_tx.clone();
                        tokio::spawn(async move {
                            if let Some(ev) = capture_event(Some(command_id), 70).await {
                                let _ = out.send(ev).await;
                            }
                        });
                    }
                    HelperCommand::StartStream { interval_ms, jpeg_quality } => {
                        if let Some(t) = stream_task.take() { t.abort(); }
                        let out = out_tx.clone();
                        stream_task = Some(tokio::spawn(async move {
                            let mut tick = tokio::time::interval(
                                std::time::Duration::from_millis(interval_ms.max(200)));
                            loop {
                                tick.tick().await;
                                if let Some(ev) = capture_event(None, jpeg_quality).await {
                                    if out.send(ev).await.is_err() { break; }
                                }
                            }
                        }));
                    }
                    HelperCommand::StopStream => {
                        if let Some(t) = stream_task.take() { t.abort(); }
                    }
                    HelperCommand::Notify { title, body } => {
                        let _ = ui_tx.send(UiMsg::Notify { title, body });
                    }
                    HelperCommand::ShowSplash => {
                        let _ = ui_tx.send(UiMsg::ShowSplash);
                    }
                    HelperCommand::Status { connected, master_name } => {
                        let _ = ui_tx.send(UiMsg::Status { connected, master: master_name });
                    }
                }
            }
            else => break,
        }
    }

    if let Some(t) = stream_task.take() {
        t.abort();
    }
    Ok(())
}

async fn capture_event(command_id: Option<String>, quality: u8) -> Option<HelperEvent> {
    let res = tokio::task::spawn_blocking(move || crate::screenshot::capture_primary(quality))
        .await
        .ok()?;
    match res {
        Ok((width, height, jpeg)) => Some(HelperEvent::ScreenFrame {
            command_id,
            width,
            height,
            jpeg: serde_bytes::ByteBuf::from(jpeg),
        }),
        Err(e) => {
            tracing::warn!("capture failed: {e:#}");
            None
        }
    }
}

fn current_user() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    if domain.is_empty() {
        user
    } else {
        format!("{domain}\\{user}")
    }
}

// ---------------------------------------------------------------------------
// Tray + Win32 message loop (runs on the main thread)
// ---------------------------------------------------------------------------

fn tray_loop(ui_rx: mpsc::Receiver<UiMsg>) {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::TrayIconBuilder;

    let s = tray_i18n::strings(TRAY_LANG);

    let menu = Menu::new();
    let status_item = MenuItem::new(s.menu_status_searching, false, None);
    let about_item = MenuItem::new(s.menu_about, true, None);
    let logs_item = MenuItem::new(s.menu_open_logs, true, None);
    let _ = menu.append(&status_item);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&about_item);
    let _ = menu.append(&logs_item);

    let about_id = about_item.id().clone();
    let logs_id = logs_item.id().clone();

    let tray = TrayIconBuilder::new()
        .with_tooltip(s.tooltip_searching)
        .with_menu(Box::new(menu));
    let tray = match load_tray_icon() {
        Some(icon) => tray.with_icon(icon),
        None => tray,
    };
    let _tray = match tray.build() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("tray build failed: {e}");
            return;
        }
    };

    let menu_channel = MenuEvent::receiver();

    loop {
        // Pump Win32 messages so the tray icon works.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Tray menu clicks.
        while let Ok(ev) = menu_channel.try_recv() {
            if ev.id == about_id {
                let s = tray_i18n::strings(TRAY_LANG);
                show_message_box(s.menu_about, s.about_text);
            } else if ev.id == logs_id {
                open_logs_folder();
            }
        }

        // Commands from the IPC thread.
        while let Ok(msg) = ui_rx.try_recv() {
            match msg {
                UiMsg::Notify { title, body } => show_message_box(&title, &body),
                UiMsg::ShowSplash => {
                    std::thread::spawn(|| crate::splash::show_blocking());
                }
                UiMsg::Status { connected, master } => {
                    let s = tray_i18n::strings(TRAY_LANG);
                    let (tip, status) = if connected {
                        (format!("{} {}", s.tooltip_connected, master), s.menu_status_connected)
                    } else {
                        (s.tooltip_searching.to_string(), s.menu_status_searching)
                    };
                    let _ = _tray.set_tooltip(Some(tip));
                    status_item.set_text(status);
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(80));
    }
}

fn load_tray_icon() -> Option<tray_icon::Icon> {
    let img = image::load_from_memory(include_bytes!("../../installer/assets/tray.png"))
        .ok()?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn show_message_box(title: &str, body: &str) {
    let title = to_wide(title);
    let body = to_wide(body);
    std::thread::spawn(move || unsafe {
        MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND,
        );
    });
}

fn open_logs_folder() {
    if let Some(dir) = dirs_local_app() {
        let logdir = dir.join("FleetView");
        let _ = std::process::Command::new("explorer.exe").arg(logdir).spawn();
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
