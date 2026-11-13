//! CoolerScreen engine owned by Unified Control. No window, tray or public endpoint.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[path = "../uc_worker.rs"]
mod uc_worker;
#[path = "../uc_sources_worker.rs"]
mod uc_sources_worker;

fn main() {
    let sources = std::env::args().any(|arg| arg == "--uc-sources-worker");
    // Unified Control supervises this process: a panic is logged to device.log, and an unguarded one ends the
    // worker (exit code 101) so UC restarts a clean one, as it did when release builds aborted on panic.
    cs_core::core::sync::install_panic_hook(if sources { "cooler-sources-worker" } else { "cooler-worker" });
    cs_core::core::sync::exit_on_unguarded_panic();
    if sources{
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
