fn main() {
    println!("cargo:rerun-if-env-changed=CLIPGOBLIN_STEAM_PACKAGE");

    if std::env::var_os("CLIPGOBLIN_STEAM_PACKAGE").is_some() {
        tauri_build::try_build(
            tauri_build::Attributes::new().capabilities_path_pattern("./capabilities/steam.json"),
        )
        .expect("failed to build the updater-free Steam package");
    } else {
        tauri_build::build()
    }
}
