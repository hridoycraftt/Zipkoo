fn main() {
    // The App Store build has no self-updater, so its permission file must not be loaded.
    let attrs = if std::env::var_os("CARGO_FEATURE_APPSTORE").is_some() {
        tauri_build::Attributes::new().capabilities_path_pattern("./capabilities/default.json")
    } else {
        tauri_build::Attributes::new()
    };
    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}
