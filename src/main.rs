#![cfg_attr(windows, windows_subsystem = "windows")]
//! TrafficBar — a TrafficMonitor-style network speed meter embedded in the Windows taskbar.

mod config;
mod format;
#[cfg(windows)]
mod settings;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod stats;
#[cfg(windows)]
mod sys;
#[cfg(windows)]
mod taskbar;

#[cfg(windows)]
fn main() {
    let open_settings = std::env::args().any(|a| a.eq_ignore_ascii_case("--settings"));
    app::run(open_settings);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("TrafficBar only runs on Windows.");
}
