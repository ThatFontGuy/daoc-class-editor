mod classdata;
mod codegen;
mod deploy;

use classdata::{ClassInfo, KnownOptions};
use codegen::CloneRequest;
use deploy::SshConfig;
use tauri::Manager;

fn resources_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    // In `tauri dev`, resources aren't copied into a bundle yet - fall back to the source tree's
    // own resources/classes folder so the app works both in dev and once built/installed.
    if let Ok(dir) = app.path().resource_dir() {
        let candidate = dir.join("classes");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    let dev_fallback = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/classes");
    if dev_fallback.exists() {
        return Ok(dev_fallback);
    }
    Err("could not locate the bundled class reference files".to_string())
}

#[tauri::command]
fn list_classes(app: tauri::AppHandle) -> Result<Vec<ClassInfo>, String> {
    let dir = resources_dir(&app)?;
    Ok(classdata::load_all_classes(&dir))
}

#[tauri::command]
fn suggest_next_id(app: tauri::AppHandle) -> Result<i32, String> {
    let dir = resources_dir(&app)?;
    Ok(classdata::next_free_id(&dir))
}

#[tauri::command]
fn known_options(app: tauri::AppHandle) -> Result<KnownOptions, String> {
    let dir = resources_dir(&app)?;
    Ok(classdata::load_known_options(&dir))
}

#[tauri::command]
fn generate_class_code(req: CloneRequest) -> String {
    codegen::generate(&req)
}

#[tauri::command]
fn sanitize_class_ident(name: String) -> String {
    codegen::sanitize_ident(&name)
}

#[tauri::command]
fn load_ssh_config() -> SshConfig {
    deploy::load_config()
}

#[tauri::command]
fn save_ssh_config(cfg: SshConfig) -> Result<(), String> {
    deploy::save_config(&cfg)
}

#[tauri::command]
fn test_ssh_connection(cfg: SshConfig) -> Result<String, String> {
    deploy::test_connection(&cfg)
}

#[tauri::command]
fn deploy_class(cfg: SshConfig, filename: String, content: String) -> Result<String, String> {
    deploy::deploy_class_file(&cfg, &filename, &content)
}

#[tauri::command]
fn run_remote_sql(cfg: SshConfig, sql: String) -> Result<String, String> {
    deploy::run_remote_sql(&cfg, &sql)
}

#[tauri::command]
fn server_status(cfg: SshConfig) -> Result<String, String> {
    deploy::remote_server_status(&cfg)
}

#[tauri::command]
fn server_control(cfg: SshConfig, action: String) -> Result<String, String> {
    deploy::remote_server_control(&cfg, &action)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            list_classes,
            suggest_next_id,
            known_options,
            generate_class_code,
            sanitize_class_ident,
            load_ssh_config,
            save_ssh_config,
            test_ssh_connection,
            deploy_class,
            run_remote_sql,
            server_status,
            server_control,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
