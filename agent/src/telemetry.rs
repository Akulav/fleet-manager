//! System telemetry collection via `sysinfo`. Runs in the LocalSystem service
//! (session 0) — all of this is available there; only screen capture needs the
//! interactive session.

use std::time::Duration;

use fleetview_shared::protocol::{DiskInfo, Telemetry};
use sysinfo::{Disks, Networks, System};

pub fn gather(logged_in_user: Option<String>) -> Telemetry {
    // Only CPU + memory are needed; `new_all()` would also enumerate every
    // process on each report.
    let mut sys = System::new();
    // CPU usage needs two samples spaced apart.
    sys.refresh_cpu_all();
    std::thread::sleep(Duration::from_millis(250));
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let cpu_usage_pct = sys.global_cpu_usage();
    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_default();
    let cpu_cores = sys.cpus().len();

    let mem_total_mb = sys.total_memory() / (1024 * 1024);
    let mem_used_mb = sys.used_memory() / (1024 * 1024);

    let disks = Disks::new_with_refreshed_list();
    let disks: Vec<DiskInfo> = disks
        .list()
        .iter()
        .map(|d| DiskInfo {
            mount: d.mount_point().to_string_lossy().to_string(),
            total_gb: d.total_space() as f64 / 1e9,
            available_gb: d.available_space() as f64 / 1e9,
        })
        .collect();

    let networks = Networks::new_with_refreshed_list();
    let mut ip_addresses = Vec::new();
    let mut mac_addresses = Vec::new();
    for data in networks.values() {
        for ipnet in data.ip_networks() {
            let ip = ipnet.addr;
            if ip.is_loopback() {
                continue;
            }
            ip_addresses.push(ip.to_string());
        }
        let mac = data.mac_address();
        if !mac.is_unspecified() {
            mac_addresses.push(mac.to_string());
        }
    }
    ip_addresses.sort();
    ip_addresses.dedup();
    mac_addresses.sort();
    mac_addresses.dedup();

    Telemetry {
        hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
        os_version: System::long_os_version().unwrap_or_else(|| "Windows".into()),
        logged_in_user,
        uptime_secs: System::uptime(),
        cpu_usage_pct,
        cpu_name,
        cpu_cores,
        mem_total_mb,
        mem_used_mb,
        disks,
        ip_addresses,
        mac_addresses,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}
