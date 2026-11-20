// Embeds the application icon and version metadata into the agent executable.
//
// NOTE: the agent deliberately uses an `asInvoker` manifest (the default), not
// `requireAdministrator`. Elevation comes from running as the LocalSystem
// service; the *same* exe also runs as the (possibly non-admin) logged-in user
// for the tray/screen-capture helper, which must not trigger a UAC prompt.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = "../installer/assets/agent.ico";
        if std::path::Path::new(icon).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon);
            res.set("ProductName", "FleetView Agent");
            res.set("FileDescription", "FleetView Agent (Made by Akulav)");
            res.set("CompanyName", "Akulav");
            res.set("LegalCopyright", "© 2026 Akulav. Source-available license.");
            let _ = res.compile();
        }
        println!("cargo:rerun-if-changed=../installer/assets/agent.ico");
    }
}
