//! FleetView master — desktop control app. Made by Akulav.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod paths;
mod server;
mod settings;
mod state;

use std::sync::{Arc, Mutex};

use state::{Shared, SharedState};

fn main() -> eframe::Result<()> {
    init_logging();

    let settings = match settings::MasterSettings::load_or_create() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to initialise master settings: {e:#}");
            std::process::exit(1);
        }
    };

    // Seed shared state with every agent we have ever seen (as offline).
    let shared = Shared::new(settings.master_name.clone(), settings.control_port);
    let state: SharedState = Arc::new(Mutex::new(shared));
    {
        let mut s = state.lock().unwrap();
        for known in state::load_known() {
            s.agents
                .insert(known.agent_id.clone(), state::AgentView::offline_from(&known));
        }
        s.log("master started");
    }

    // Start the networking backend.
    server::spawn_backend(settings.clone(), state.clone());

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([820.0, 560.0])
            .with_title("FleetView — Master"),
        ..Default::default()
    };

    eframe::run_native(
        "FleetView — Master",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, state.clone(), settings.clone())))),
    )
}

fn init_logging() {
    let _ = paths::ensure_data_dir();
    let dir = paths::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let appender = tracing_appender::rolling::daily(dir, "master.log");
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(appender)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}
