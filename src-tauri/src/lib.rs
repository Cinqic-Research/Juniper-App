mod agent;
#[cfg(target_os = "android")]
mod android_runtime;
mod backend;
mod behavior;
mod catalog;
mod commands;
mod device;
pub mod device_link;
mod domain;
mod gguf_guard;
#[cfg(not(target_os = "android"))]
mod juniper1;
// Hardware tests: they stop the server with a Unix signal to test recovery.
#[cfg(all(test, target_os = "linux"))]
mod live_agent;
#[cfg(all(test, target_os = "linux"))]
mod live_catalog;
#[cfg(all(test, unix, not(target_os = "android")))]
mod live_gpt_oss;
#[cfg(all(test, target_os = "linux"))]
mod live_juniper1;
#[cfg(all(test, target_os = "linux"))]
mod live_services;
#[cfg(not(target_os = "android"))]
mod local_runtime;
mod managed_models;
mod model_licenses;
mod providers;
mod runtime_registry;
mod startup;
mod storage;
mod tools;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod window;

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
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let lock_path = data_dir.join("agent-runtime.lock");
                app.state::<AppState>()
                    .agent
                    .acquire_runtime_lock(&lock_path)
                    .map_err(|error| {
                        startup::stage_failed("agent-runtime-lock", Some(&lock_path), &error)
                    })?;
            }
            let database = data_dir.join("juniper.db");
            storage::initialize(&database)
                .map_err(|error| startup::stage_failed("open-database", Some(&database), &error))?;
            agent::start(
                app.handle().clone(),
                &app.state::<AppState>().agent,
                database.clone(),
                data_dir.clone(),
            );
            #[cfg(not(target_os = "android"))]
            startup::local_runtime(local_runtime::runtime_executable(app.handle()));
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            window::fit_main_window(app.handle());
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
            commands::model_license_accepted,
            commands::accept_model_license,
            commands::cancel_managed_model,
            commands::import_managed_model,
            commands::juniper1_models,
            commands::import_juniper1_model,
            commands::local_runtime_status,
            commands::unload_local_runtime,
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
            commands::window_insets,
            agent::commands::agent_overview,
            agent::commands::agent_tools,
            agent::commands::agent_profile_get,
            agent::commands::agent_profile_save,
            agent::commands::agent_profile_export,
            agent::commands::agent_profile_import,
            agent::commands::task_start,
            agent::commands::task_detail,
            agent::commands::task_list,
            agent::commands::task_pause,
            agent::commands::task_resume,
            agent::commands::task_cancel,
            agent::commands::task_steer,
            agent::commands::task_answer,
            agent::commands::approval_decide,
            agent::commands::grants_list,
            agent::commands::grant_revoke,
            agent::commands::agent_stop_all,
            agent::commands::agent_settings_set,
            agent::commands::project_save,
            agent::commands::project_delete,
            agent::commands::project_add_root,
            agent::commands::project_remove_root,
            agent::commands::project_index,
            agent::commands::project_export,
            agent::commands::project_import,
            agent::commands::artifact_content,
            agent::commands::artifact_export,
            agent::commands::inbox_mark_read,
            agent::commands::memory_changes,
            agent::commands::skills_list,
            agent::commands::skill_set_enabled,
            agent::commands::skill_remove,
            agent::commands::skill_import,
            agent::commands::skill_export,
            agent::commands::plugins_list,
            agent::commands::plugin_register_http,
            agent::commands::plugin_register_local,
            agent::commands::plugin_test,
            agent::commands::plugin_set_enabled,
            agent::commands::plugin_remove,
            agent::commands::schedules_list,
            agent::commands::schedule_preview,
            agent::commands::schedule_save,
            agent::commands::schedule_delete,
            agent::commands::schedule_run_now,
            agent::commands::schedule_runs
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| startup::exit_after_run_error(&error))
        .run(|_app, _event| {
            #[cfg(not(target_os = "android"))]
            if let tauri::RunEvent::Exit = _event {
                _app.state::<AppState>().local_runtime.terminate_all();
                agent::mcp::disconnect_all();
            }
        });
}
