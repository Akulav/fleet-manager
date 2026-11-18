//! Windows service glue and the top-level async worker.

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::{paths, SERVICE_DESC, SERVICE_DISPLAY, SERVICE_NAME};

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

/// Install the service to run as LocalSystem, auto-start at boot, and restart on
/// failure. Must be run elevated.
pub fn install_service() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;

    let exe = std::env::current_exe()?;
    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: SERVICE_DISPLAY.into(),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec!["run-service".into()],
        dependencies: vec![],
        account_name: None, // None => LocalSystem
        account_password: None,
    };

    let access = ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS;
    // On upgrade/reinstall the service already exists: update it in place
    // instead of failing (which used to leave it stopped after an upgrade).
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(existing) => {
            existing.change_config(&info)?;
            existing
        }
        Err(_) => manager.create_service(&info, access)?,
    };
    service.set_description(SERVICE_DESC)?;

    // Configure automatic restart on crash: reset count daily, restart after 5s.
    configure_recovery().ok();

    // Start it right away (unless it is somehow already running).
    let running = service
        .query_status()
        .map(|s| s.current_state != ServiceState::Stopped)
        .unwrap_or(false);
    if !running {
        service.start::<&str>(&[])?;
    }
    Ok(())
}

/// Use `sc.exe failure` to set recovery actions (restart on each failure). The
/// windows-service crate doesn't expose SERVICE_FAILURE_ACTIONS directly, so we
/// shell out — simplest and robust.
fn configure_recovery() -> anyhow::Result<()> {
    std::process::Command::new("sc.exe")
        .args([
            "failure",
            SERVICE_NAME,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/5000/restart/5000",
        ])
        .status()
        .context("sc.exe failure")?;
    Ok(())
}

pub fn uninstall_service() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
    )?;
    let _ = service.stop();
    service.delete()?;
    Ok(())
}

static SHUTDOWN: OnceLock<CancellationToken> = OnceLock::new();

pub fn shutdown_token() -> CancellationToken {
    SHUTDOWN.get_or_init(CancellationToken::new).clone()
}

/// Called from the SCM-invoked `service_main`.
pub fn run() -> anyhow::Result<()> {
    init_logging("service");
    let shutdown = shutdown_token();

    let handler_shutdown = shutdown.clone();
    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                handler_shutdown.cancel();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })?;

    let set_state = |state: ServiceState, controls: ServiceControlAccept| {
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted: controls,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(5),
            process_id: None,
        });
    };

    set_state(ServiceState::StartPending, ServiceControlAccept::empty());
    set_state(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
    );

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(worker_main(shutdown.clone()));

    set_state(ServiceState::Stopped, ServiceControlAccept::empty());
    Ok(())
}

pub fn init_logging(tag: &str) {
    let _ = paths::ensure_data_dir();
    let Some(file_appender) = rolling_log(paths::log_dir(), &format!("{tag}.log")) else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(file_appender)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    tracing::info!(
        "FleetView agent {} starting ({tag})",
        env!("CARGO_PKG_VERSION")
    );
}

/// Daily-rolling log file that keeps only the last two weeks (the default
/// appender keeps every file forever).
pub fn rolling_log(
    dir: std::path::PathBuf,
    name: &str,
) -> Option<tracing_appender::rolling::RollingFileAppender> {
    tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(name)
        .max_log_files(14)
        .build(dir)
        .ok()
}

/// The real work. Supervises the user-session helper and the master connection.
pub async fn worker_main(shutdown: CancellationToken) {
    if let Err(e) = paths::ensure_data_dir() {
        tracing::error!("cannot create data dir: {e:#}");
    }
    paths::lock_down_data_dir();
    // Leftovers from a previous run (crash / power loss mid-install).
    let _ = std::fs::remove_dir_all(paths::staging_dir());
    let _ = std::fs::create_dir_all(paths::staging_dir());

    let cfg = match crate::config::AgentConfig::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                "no valid agent config ({e:#}); the agent will idle. \
                 Install via the FleetView agent installer to provision it."
            );
            // Idle until stopped so the service doesn't crash-loop.
            shutdown.cancelled().await;
            return;
        }
    };

    // Shared IPC hub between service and the user-session helper.
    let hub = crate::ipc::Hub::new();

    // Task 1: loopback IPC server for the helper.
    let ipc_task = {
        let hub = hub.clone();
        let sd = shutdown.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::ipc::serve(hub, sd).await {
                tracing::error!("ipc server stopped: {e:#}");
            }
        })
    };

    // Task 2: keep the helper alive in the active user session.
    let helper_task = {
        let sd = shutdown.clone();
        let hub = hub.clone();
        tokio::spawn(async move { crate::session::helper_supervisor(hub, sd).await })
    };

    // Task 3: discover + connect to the master, forever (with backoff).
    let net_task = {
        let sd = shutdown.clone();
        let hub = hub.clone();
        tokio::spawn(async move { crate::net::connection_loop(cfg, hub, sd).await })
    };

    shutdown.cancelled().await;
    tracing::info!("shutdown requested; stopping tasks");
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        let _ = ipc_task.await;
        let _ = helper_task.await;
        let _ = net_task.await;
    })
    .await;
}
