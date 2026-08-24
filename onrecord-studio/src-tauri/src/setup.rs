//! macOS runtime environment hooks, folder provisioning, and window decoration profiles.

use std::fs;
use tauri::{App, Manager, Runtime};
use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
use crate::storage::sqlite::SqliteRepository;
use crate::commands::ingest::{AppQueueState, initialize_queue_runtime};

/// Provisions native assets and sets up local database tables prior to showing the interface.
pub fn configure_onrecord_studio_runtime<R: Runtime>(app: &mut App<R>) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Resolve and provision explicit app data tracking slots in the home directory path
    let app_data_dir = app.path().app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("./.onrecord_studio_cache"));
    
    fs::create_dir_all(&app_data_dir)?;
    let sqlite_db_file = app_data_dir.join("onrecord_studio_core.db");

    // 2. Initialize the relational SQLite manager and run structural tracking migrations
    let repository = SqliteRepository::new(&sqlite_db_file)?;
    
    // Safely load schema tracking strings from your migration slot
    let initial_migration = include_str!("../migrations/0001_initial_schema.sql");
    tauri::async_runtime::block_on(repository.run_migrations(initial_migration))?;
    
    // Register the database instance globally as shared app state
    app.manage(repository);

    // 3. Boot up the asynchronous queue workspace pipeline
    let app_handle = app.handle().clone();
    let dispatcher = tauri::async_runtime::block_on(initialize_queue_runtime(app_handle));
    app.manage(AppQueueState { dispatcher });

    // 4. Polish the primary window window with native frosted glass effect styles
    if let Some(main_window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        let _ = apply_vibrancy(
            &main_window, 
            NSVisualEffectMaterial::Sidebar, 
            None, 
            None
        );
    }

    Ok(())
}
