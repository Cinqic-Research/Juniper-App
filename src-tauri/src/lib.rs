#[cfg(target_os = "android")]
mod android_runtime;
mod catalog;
mod commands;
mod device;
pub mod device_link;
mod domain;
#[cfg(not(target_os = "android"))]
mod local_runtime;
mod managed_models;
mod providers;
mod runtime_registry;
mod startup;
mod storage;
mod tools;

use commands::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(juniper_local_runtime::init())
        .setup(|app| {
            startup::announce();
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| startup::stage_failed("resolve-app-data-dir", None, &error))?;
            std::fs::create_dir_all(&data_dir).map_err(|error| {
                startup::stage_failed("create-app-data-dir", Some(&data_dir), &error)
            })?;
            let database = data_dir.join("juniper.db");
            storage::initialize(&database)
                .map_err(|error| startup::stage_failed("open-database", Some(&database), &error))?;
            #[cfg(not(target_os = "android"))]
            startup::local_runtime(local_runtime::runtime_executable(app.handle()));
            startup::stage_ok("native-setup");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::system_info,
            commands::model_catalog,
            commands::device_capabilities,
            commands::runtime_registry,
            commands::managed_models,
            commands::download_managed_model,
            commands::cancel_managed_model,
            commands::delete_managed_model,
            commands::runtime_logs,
            commands::load_app_data,
            commands::save_app_data,
            commands::health_check,
            commands::list_models,
            commands::inspect_model,
            commands::pull_model,
            commands::cancel_model_pull,
            commands::import_gguf,
            commands::cancel_gguf_import,
            commands::delete_model,
            commands::running_models,
            commands::chat_stream,
            commands::cancel_chat,
            commands::resolve_permission,
            commands::pick_attachment,
            commands::pick_gguf,
            commands::read_attachment,
            commands::secure_set_credential,
            commands::secure_delete_credential,
            commands::frontend_ready,
            commands::frontend_fatal,
            commands::window_insets
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| startup::exit_after_run_error(&error))
        .run(|_app, _event| {
            #[cfg(not(target_os = "android"))]
            if let tauri::RunEvent::Exit = _event {
                _app.state::<AppState>().local_runtimes.terminate_all();
            }
        });
}
