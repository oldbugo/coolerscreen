fn main() {
    let cmds = &[
        "win_minimize", "win_toggle_maximize", "win_state", "win_close", "win_force_close",
        "device_state", "device_info", "device_restart", "device_upload",
    ];
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(cmds)))
        .expect("tauri build");
}