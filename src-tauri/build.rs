fn main() {
    // Window chrome (WS4) + core commands (src/core/commands.rs COMMANDS); keep in sync with capabilities/*.json.
    let cmds = &[
        "win_minimize", "win_toggle_maximize", "win_state", "win_close", "win_force_close", "live_set",
        "app_state", "settings_update", "activity", "job_get", "job_cancel", "set_focus", "originals_list", "import_bytes", "import_path",
        "missing_delete", "original_get", "original_patch", "original_delete", "original_reveal", "estimate", "loop_search", "loop_point",
        "version_create", "version_patch", "version_delete", "version_send", "version_remake", "version_retry", "sources_list", "browse_set",
        "link_add", "source_set", "source_items", "source_add", "gallery_set_dir", "gallery_open", "gallery_sync", "device_state",
        "device_check", "device_restart", "device_setup", "device_restore", "device_reconnect",
    ];
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(cmds)))
        .expect("tauri build");
}
