//! Tray strings. Per requirement, the tray defaults to **Romanian**; the master
//! desktop app is English-only. English strings are kept here as a fallback and
//! in case the tray language is made configurable later.
#![allow(dead_code)] // English fallback + splash strings are kept for future use.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ro,
    En,
}

/// The agent tray defaults to Romanian.
pub const TRAY_LANG: Lang = Lang::Ro;

pub struct Strings {
    pub tooltip_connected: &'static str, // takes a {master} via format
    pub tooltip_searching: &'static str,
    pub menu_status_connected: &'static str,
    pub menu_status_searching: &'static str,
    pub menu_about: &'static str,
    pub menu_open_logs: &'static str,
    pub splash_title: &'static str,
    pub splash_body: &'static str,
    pub running_title: &'static str,
    pub running_body: &'static str,
    pub about_text: &'static str,
}

pub fn strings(lang: Lang) -> Strings {
    match lang {
        Lang::Ro => Strings {
            tooltip_connected: "FleetView — conectat la",
            tooltip_searching: "FleetView — se caută serverul…",
            menu_status_connected: "Stare: conectat",
            menu_status_searching: "Stare: se caută serverul…",
            menu_about: "Despre FleetView",
            menu_open_logs: "Deschide jurnalele",
            splash_title: "FleetView",
            splash_body: "Agentul de administrare rulează pe acest computer.",
            running_title: "FleetView",
            running_body: "Agentul rulează pe acest computer.",
            about_text: "FleetView Agent\nAdministrare la distanță a stației.\nRealizat de Akulav.",
        },
        Lang::En => Strings {
            tooltip_connected: "FleetView — connected to",
            tooltip_searching: "FleetView — searching for server…",
            menu_status_connected: "Status: connected",
            menu_status_searching: "Status: searching…",
            menu_about: "About FleetView",
            menu_open_logs: "Open logs",
            splash_title: "FleetView",
            splash_body: "The management agent is running on this computer.",
            running_title: "FleetView",
            running_body: "The agent is running on this computer.",
            about_text: "FleetView Agent\nRemote workstation management.\nMade by Akulav.",
        },
    }
}
