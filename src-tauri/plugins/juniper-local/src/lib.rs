#[cfg(target_os = "android")]
use serde::{Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Manager, Runtime,
    plugin::{Builder, PluginApi, PluginHandle, TauriPlugin},
};

#[cfg(target_os = "android")]
const ANDROID_PLUGIN_IDENTIFIER: &str = "com.cinqic.juniper.local_runtime";
#[cfg(target_os = "android")]
const ANDROID_PLUGIN_CLASS: &str = "JuniperLocalRuntimePlugin";

/// Handle retained by the application so Rust commands can invoke the native
/// Android controller without exposing a localhost server or a webview bridge.
#[derive(Debug)]
pub struct LocalRuntimeHandle<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("juniper-local-runtime")
        .setup(|app, api: PluginApi<R, ()>| {
            #[cfg(not(target_os = "android"))]
            let _ = (&app, &api);
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin(ANDROID_PLUGIN_IDENTIFIER, ANDROID_PLUGIN_CLASS)?;
                app.manage(LocalRuntimeHandle(handle));
            }
            Ok(())
        })
        .build()
}

pub fn handle<R: Runtime>(app: &AppHandle<R>) -> Result<PluginHandle<R>, String> {
    app.try_state::<LocalRuntimeHandle<R>>()
        .map(|state| state.0.clone())
        .ok_or_else(|| "LOCAL_RUNTIME_UNAVAILABLE: Native local runtime is not registered.".into())
}

#[cfg(target_os = "android")]
pub fn invoke<R, T, P>(app: &AppHandle<R>, command: &'static str, payload: P) -> Result<T, String>
where
    R: Runtime,
    T: DeserializeOwned,
    P: Serialize,
{
    handle(app)?
        .run_mobile_plugin(command, payload)
        .map_err(|error| format!("LOCAL_RUNTIME_PLUGIN_ERROR: {error}"))
}

#[cfg(target_os = "android")]
pub fn secure_set_credential<R: Runtime>(
    app: &AppHandle<R>,
    reference: &str,
    secret: &str,
) -> Result<(), String> {
    invoke::<R, serde_json::Value, _>(
        app,
        "secureSetCredential",
        serde_json::json!({ "reference": reference, "secret": secret }),
    )
    .map(|_| ())
}

#[cfg(target_os = "android")]
pub fn secure_get_credential<R: Runtime>(
    app: &AppHandle<R>,
    reference: &str,
) -> Result<String, String> {
    let payload: serde_json::Value = invoke(
        app,
        "secureGetCredential",
        serde_json::json!({ "reference": reference }),
    )?;
    payload
        .get("secret")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            "LOCAL_RUNTIME_PLUGIN_ERROR: Secure credential payload was malformed.".into()
        })
}

#[cfg(target_os = "android")]
pub fn secure_delete_credential<R: Runtime>(
    app: &AppHandle<R>,
    reference: &str,
) -> Result<(), String> {
    invoke::<R, serde_json::Value, _>(
        app,
        "secureDeleteCredential",
        serde_json::json!({ "reference": reference }),
    )
    .map(|_| ())
}
