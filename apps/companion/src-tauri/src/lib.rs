mod commands;
mod db;
mod protocol;
mod types;

use std::sync::Mutex;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = db::Db::open().expect("could not open the companion database");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState { db: Mutex::new(db) })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_catalog,
            commands::get_sales,
            commands::record_sale,
            commands::pair,
            commands::sync_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Stride Companion");
}
