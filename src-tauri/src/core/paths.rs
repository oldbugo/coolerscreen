//! Same locations as 1.2 (`src/paths.js`, `gallery.documentsDir`), so 1.2 users need no migration.
use std::path::PathBuf;

/// %APPDATA%\CoolerScreen (or COOLERSCREEN_HOME).
pub fn base() -> PathBuf {
    if let Some(h) = std::env::var_os("COOLERSCREEN_HOME") {
        return PathBuf::from(h);
    }
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"));
    appdata.join("CoolerScreen")
}

pub fn library() -> PathBuf {
    base().join("library")
}

pub fn settings() -> PathBuf {
    base().join("settings.json")
}

/// Downloads and other temporary files of sources.
pub fn downloads() -> PathBuf {
    base().join("downloads")
}

/// The user's Documents folder (Known Folder, follows OneDrive redirection like 1.2's PowerShell lookup).
pub fn documents_dir() -> PathBuf {
    dirs::document_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Documents"))
}

/// Documents\CoolerScreen\Sources (JSON plug-ins), or COOLERSCREEN_SOURCES_DIR.
pub fn sources_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("COOLERSCREEN_SOURCES_DIR") {
        return p.into();
    }
    documents_dir().join("CoolerScreen").join("Sources")
}

/// ffmpeg: CS_FFMPEG, else ffmpeg.exe next to the app exe (bundled), else the repo's ffmpeg-static (development),
/// else "ffmpeg" on PATH.
pub fn ffmpeg() -> PathBuf {
    if let Some(p) = std::env::var_os("CS_FFMPEG") {
        return p.into();
    }
    let exe_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            for c in [d.join(exe_name), d.join("resources").join(exe_name)] {
                if c.exists() {
                    return c;
                }
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("node_modules").join("ffmpeg-static").join(exe_name);
    if dev.exists() {
        return dev;
    }
    PathBuf::from(exe_name)
}

/// The repo / install folder holding `scripts/*.ps1` (setup, restore, reconnect).
pub fn scripts_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("COOLERSCREEN_SCRIPTS") {
        return p.into();
    }
    // Installed: next to the exe (bundled resources); development: the repo this was built from.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join("scripts");
            if p.join("setup-winusb.ps1").exists() {
                return p;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("scripts")
}
