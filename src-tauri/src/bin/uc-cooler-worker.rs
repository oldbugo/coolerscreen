//! CoolerScreen engine owned by Unified Control. No window, tray or public endpoint.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[path = "../uc_worker.rs"]
mod uc_worker;
#[path = "../uc_sources_worker.rs"]
mod uc_sources_worker;

fn main() {
    if std::env::args().any(|arg|arg=="--uc-sources-worker"){
        if uc_sources_worker::run().is_err(){eprintln!("UC source worker stopped: source_error");std::process::exit(1);}return;
    }
    if let Err(error) = uc_worker::run() {
        // Never print protocol requests, paths, settings or file contents.
        eprintln!(
            "Cooler worker stopped: {}",
            error.code.as_deref().unwrap_or("worker")
        );
        std::process::exit(1);
    }
}
