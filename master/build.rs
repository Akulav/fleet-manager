// Embeds the master app icon + version metadata and marks it as a GUI app.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = "../installer/assets/master.ico";
        if std::path::Path::new(icon).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon);
            res.set("ProductName", "FleetView Master");
            res.set("FileDescription", "FleetView Master (Made by Akulav)");
            res.set("CompanyName", "Akulav");
            res.set("LegalCopyright", "© 2026 Akulav. Source-available license.");
            let _ = res.compile();
        }
        println!("cargo:rerun-if-changed=../installer/assets/master.ico");
    }
}
