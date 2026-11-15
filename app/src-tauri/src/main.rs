// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Installer hooks call these to add or remove the right-click menu without opening a window.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--register-shell") {
        std::process::exit(if zipkoo_app_lib::shell::install().is_ok() {
            0
        } else {
            1
        });
    }
    if args.iter().any(|a| a == "--unregister-shell") {
        std::process::exit(if zipkoo_app_lib::shell::uninstall().is_ok() {
            0
        } else {
            1
        });
    }
    zipkoo_app_lib::run()
}
