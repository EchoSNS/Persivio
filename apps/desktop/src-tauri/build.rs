//! Generates Tauri's context and the permission set for the app's own commands.

/// Every app command. Listing them makes Tauri generate `allow-<command>` permissions
/// and deny any command not granted in `capabilities/` (least privilege, ADR-018).
const COMMANDS: &[&str] = &[
    "list_tasks",
    "add_task",
    "pause_task",
    "resume_task",
    "cancel_task",
    "remove_task",
    "pause_all",
    "resume_all",
    "move_task",
    "select_files",
    "get_settings",
    "set_settings",
    "subscribe_progress",
    "default_download_dir",
    "reveal_task",
    "get_task_detail",
    "inspect_torrent",
];

fn main() {
    let mut attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Tauri embeds its Windows manifest (Common Controls v6) only into the app
        // binary; test binaries then fail to start with STATUS_ENTRYPOINT_NOT_FOUND.
        // `rustc-link-arg` embeds the same manifest into every binary of this crate.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    if let Err(e) = tauri_build::try_build(attributes) {
        // A build script reports failure through its exit status; print the cause.
        eprintln!("tauri-build failed: {e:#}");
        std::process::exit(1);
    }
}
