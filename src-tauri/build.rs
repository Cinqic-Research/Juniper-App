use tauri_build::{Attributes, WindowsAttributes};

fn main() {
    // Tauri's default Windows resource carries the application manifest that
    // requests Common Controls 6, but it is linked into binaries only. The
    // test harness is a separate executable: without that manifest the loader
    // bound it to the System32 `comctl32.dll`, which has no
    // `TaskDialogIndirect`, and `cargo test` aborted with
    // `STATUS_ENTRYPOINT_NOT_FOUND` before any test ran. The manifest is
    // therefore embedded by the linker for every linked target instead
    // (`windows-app.manifest`, the same content), and left out of the
    // resource so it is not embedded twice.
    let windows = WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
    embed_windows_manifest();
}

fn embed_windows_manifest() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
