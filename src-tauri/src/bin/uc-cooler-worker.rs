//! CoolerScreen engine owned by Unified Control. No window, tray or public endpoint.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[path = "../uc_worker.rs"]
mod uc_worker;

fn main() {
    if let Err(error) = uc_worker::run() {
        // Never print protocol requests, paths, settings or file contents.
        eprintln!(
            "Cooler worker stopped: {}",
            error.code.as_deref().unwrap_or("worker")
        );
        std::process::exit(1);
    }
}
