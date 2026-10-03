// Prevents a persistent terminal console window from showing on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = onrecord_studio_lib::run_isolated_worker() {
        std::process::exit(code);
    }
    // Passes execution constraints directly to our synchronized workspace library engine
    onrecord_studio_lib::run();
}
