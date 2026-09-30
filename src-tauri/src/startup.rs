//! Bounded, local-only startup diagnostics.
//!
//! Juniper has no telemetry and no crash reporting. These lines are written to
//! the process's own stderr so a person who launches Juniper from a terminal
//! can see which startup stage failed, and release smoke tests can require a
//! positive frontend-ready signal instead of treating survival as success.
//! Reports never include credentials, prompts, conversation contents, or model
//! output; frontend reports are reduced to a bounded single-line error summary.

use std::{
    fmt::Display,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Stable prefix that `scripts/linux-launch-probe.sh` matches.
const PREFIX: &str = "[juniper-startup]";
const MAX_FRONTEND_REPORT_CHARS: usize = 800;

static FRONTEND_READY: AtomicBool = AtomicBool::new(false);

fn emit(message: &str) {
    eprintln!("{PREFIX} {message}");
}

pub fn announce() {
    emit(&format!(
        "Juniper {VERSION} starting on {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    #[cfg(target_os = "linux")]
    emit(&linux_environment(|name| std::env::var_os(name).is_some()));
}

pub fn stage_ok(stage: &str) {
    emit(&format!("stage {stage}: ok"));
}

/// Records a failed startup stage and returns the error Tauri's setup hook
/// propagates. The path is included because most failures here are local
/// filesystem or database problems the user can act on.
pub fn stage_failed(
    stage: &str,
    path: Option<&Path>,
    error: &dyn Display,
) -> Box<dyn std::error::Error> {
    let message = match path {
        Some(path) => format!("stage {stage} failed ({}): {error}", path.display()),
        None => format!("stage {stage} failed: {error}"),
    };
    emit(&message);
    message.into()
}

/// Reports where the bundled desktop llama-server resolved. A missing runtime
/// is not fatal (external providers still work) but must be visible.
#[cfg(not(target_os = "android"))]
pub fn local_runtime(resolved: Result<std::path::PathBuf, String>) {
    match resolved {
        Ok(path) => emit(&format!("local runtime: {}", path.display())),
        Err(error) => emit(&format!("local runtime unavailable: {error}")),
    }
}

pub fn frontend_ready() {
    if !FRONTEND_READY.swap(true, Ordering::SeqCst) {
        emit("frontend ready");
    }
}

pub fn frontend_fatal(report: &str) {
    emit(&format!("frontend fatal: {}", single_line(report)));
}

pub fn exit_after_run_error(error: &dyn Display) -> ! {
    emit(&format!("fatal: {error}"));
    #[cfg(target_os = "linux")]
    emit(
        "Linux troubleshooting: https://github.com/Cinqic-Research/Juniper-App/blob/main/docs/release/linux-troubleshooting.md",
    );
    std::process::exit(1);
}

fn single_line(report: &str) -> String {
    let flattened: String = report
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_FRONTEND_REPORT_CHARS)
        .collect();
    flattened.trim().to_owned()
}

/// Names only: which display protocols are present and which documented
/// WebKitGTK or GTK graphics overrides are set. Values are never echoed.
#[cfg(any(target_os = "linux", test))]
fn linux_environment(is_set: impl Fn(&str) -> bool) -> String {
    let session = match (is_set("WAYLAND_DISPLAY"), is_set("DISPLAY")) {
        (true, true) => "wayland+x11",
        (true, false) => "wayland",
        (false, true) => "x11",
        (false, false) => "no-display",
    };
    let overrides: Vec<&str> = [
        "WEBKIT_DISABLE_DMABUF_RENDERER",
        "WEBKIT_DISABLE_COMPOSITING_MODE",
        "__NV_DISABLE_EXPLICIT_SYNC",
        "GDK_BACKEND",
    ]
    .into_iter()
    .filter(|name| is_set(name))
    .collect();
    format!(
        "linux display={session} appimage={} graphics-overrides={}",
        is_set("APPIMAGE"),
        if overrides.is_empty() {
            "none".to_owned()
        } else {
            overrides.join(",")
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_reports_are_single_line_and_bounded() {
        let report = format!(
            "ReferenceError: x is not defined\n    at App\r\n{}",
            "a".repeat(5000)
        );
        let line = single_line(&report);
        assert!(!line.contains('\n') && !line.contains('\r'));
        assert_eq!(line.chars().count(), MAX_FRONTEND_REPORT_CHARS);
        assert!(line.starts_with("ReferenceError: x is not defined"));
    }

    #[test]
    fn linux_environment_names_overrides_without_values() {
        let set = ["DISPLAY", "APPIMAGE", "WEBKIT_DISABLE_DMABUF_RENDERER"];
        let line = linux_environment(|name| set.contains(&name));
        assert_eq!(
            line,
            "linux display=x11 appimage=true graphics-overrides=WEBKIT_DISABLE_DMABUF_RENDERER"
        );
        assert_eq!(
            linux_environment(|_| false),
            "linux display=no-display appimage=false graphics-overrides=none"
        );
    }

    #[test]
    fn stage_failures_name_the_stage_and_path() {
        let error = stage_failed(
            "open-database",
            Some(Path::new("/data/juniper.db")),
            &"file is not a database",
        );
        assert_eq!(
            error.to_string(),
            "stage open-database failed (/data/juniper.db): file is not a database"
        );
    }
}
