//! FleetView master — desktop control app. Made by Akulav.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agentpkg;
mod app;
mod demo;
mod installer;
mod packages;
mod paths;
mod policy;
mod prefs;
mod server;
mod settings;
mod state;
mod ui;
mod wol;

use std::sync::{Arc, Mutex};

use state::{Shared, SharedState};

fn main() -> eframe::Result<()> {
    let demo = demo::requested();
    if demo {
        demo::use_demo_data_dir();
    }
    init_logging();

    let settings = match settings::MasterSettings::load_or_create() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to initialise master settings: {e:#}");
            // Release builds have no console, so show the error.
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("FleetView — cannot start")
                .set_description(format!("Failed to initialise master settings:\n\n{e:#}"))
                .show();
            std::process::exit(1);
        }
    };

    let master_name = if demo {
        "FleetView demo".to_string()
    } else {
        settings.master_name.clone()
    };
    let shared = Shared::new(master_name);
    let state: SharedState = Arc::new(Mutex::new(shared));

    if demo {
        demo::start(state.clone());
    } else {
        // Seed shared state with every agent we have ever seen (as offline).
        {
            let mut s = state.lock().unwrap();
            for known in state::load_known() {
                s.agents.insert(
                    known.agent_id.clone(),
                    state::AgentView::offline_from(&known),
                );
            }
            s.log("master started");
        }
        // Start the networking backend.
        server::spawn_backend(settings.clone(), state.clone());
    }

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([980.0, 620.0])
            .with_title(if demo {
                "FleetView — Master (demo)"
            } else {
                "FleetView — Master"
            }),
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
