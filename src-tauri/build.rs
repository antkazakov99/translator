use std::path::Path;

fn main() {
    // Манифест Windows с Common Controls v6 нужен не только приложению: тестовые .exe, которые тянут код окна,
    // без него не запускаются (STATUS_ENTRYPOINT_NOT_FOUND). tauri-build вшивает его ресурсом только в приложение,
    // поэтому манифест передаётся компоновщику для всех целей, а ресурс tauri-build выключен.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let mut windows = tauri_build::WindowsAttributes::new();
    if target_os == "windows" && target_env == "msvc" {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)).expect("сборка Tauri");
}
