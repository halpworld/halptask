fn main() {
    // Declare the app's own commands so capabilities can grant them by name
    // (allow-dispatch, ...). Anything not listed here cannot be invoked.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "dispatch",
            "key_status",
            "unlock",
            "hide_capture",
            "integrations_set_token",
            "integrations_secret_hint",
            "ext_widget",
            "ext_call",
            "ext_run_transform",
        ]),
    ))
    .expect("failed to run tauri-build");
}
