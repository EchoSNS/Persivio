//! Persivio desktop entry point.

// Release builds on Windows are GUI apps: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = persivio_desktop::run() {
        eprintln!("Persivio failed to start: {e:#}");
        std::process::exit(1);
    }
}
