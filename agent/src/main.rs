//! FleetView agent entry point.
//!
//! A single executable with several modes, selected by the first argument:
//!
//!   (none) | run-service   Service control dispatcher (how the SCM starts us).
//!   install              Register the Windows service (run elevated once).
//!   uninstall            Remove the Windows service.
//!   console              Run the service worker in the foreground (debugging).
//!   helper               User-session helper: tray icon, splash, screen capture.
//!   version              Print version and exit.
//!
//! The service runs as LocalSystem (always elevated) and performs all network,
//! telemetry and command work. It launches `helper` into the active user
//! session so the tray icon, startup splash and screen capture run where the
//! user can see them and where the desktop is actually reachable.

// On release builds the service has no console; the helper is a GUI app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod helper;
mod inventory;
mod ipc;
mod net;
mod paths;
mod screenshot;
mod service;
mod session;
mod splash;
mod telemetry;
mod tls;
mod tray_i18n;

use std::ffi::OsString;

const SERVICE_NAME: &str = "FleetViewAgent";
const SERVICE_DISPLAY: &str = "FleetView Agent";
const SERVICE_DESC: &str =
    "FleetView remote management agent (telemetry, software deployment). Made by Akulav.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(|s| s.as_str()).unwrap_or("run-service");

    match mode {
        "version" | "--version" | "-V" => {
            println!("FleetView Agent {}", env!("CARGO_PKG_VERSION"));
        }
        "install" => {
            if let Err(e) = service::install_service() {
                eprintln!("install failed: {e:#}");
                std::process::exit(1);
            }
            println!("Service installed.");
        }
        "uninstall" => {
            if let Err(e) = service::uninstall_service() {
                eprintln!("uninstall failed: {e:#}");
                std::process::exit(1);
            }
            println!("Service removed.");
        }
        "console" => {
            // Foreground run for debugging (Ctrl-C to stop).
            service::init_logging("console");
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let shutdown = service::shutdown_token();
            let sd = shutdown.clone();
            rt.spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                sd.cancel();
            });
            rt.block_on(service::worker_main(shutdown));
        }
        "helper" => {
            helper::run();
        }
        "run-service" => {
            // Hand control to the SCM dispatcher. This blocks until the service
            // stops. If we were NOT started by the SCM this returns an error,
            // which we report for convenience.
            if let Err(e) = run_as_service() {
                eprintln!(
                    "Not started by the Service Control Manager ({e}).\n\
                     Use `fleetview-agent install` then start the service, or \
                     `fleetview-agent console` to run in the foreground."
                );
                std::process::exit(1);
            }
        }
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
}

fn run_as_service() -> anyhow::Result<()> {
    windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| anyhow::anyhow!(e))
}

// The SCM calls into this symbol; it must have this exact signature.
windows_service::define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = service::run() {
        tracing::error!("service failed: {e:#}");
    }
}
