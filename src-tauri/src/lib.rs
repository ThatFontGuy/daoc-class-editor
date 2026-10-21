mod classdata;
mod codegen;
mod deploy;
mod deployed;
mod specs;

use classdata::{ClassInfo, KnownOptions};
use codegen::CloneRequest;
use deploy::SshConfig;
use deployed::DeployedClass;
use specs::{ClassSpecRow, SpecAssignment, SpecializationRow};
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

#[tauri::command]
fn list_specializations(cfg: SshConfig) -> Result<Vec<SpecializationRow>, String> {
    specs::list_specializations(&cfg)
}

#[tauri::command]
fn list_class_specs(cfg: SshConfig, class_id: i64) -> Result<Vec<ClassSpecRow>, String> {
    specs::list_class_specs(&cfg, class_id)
}

#[tauri::command]
fn apply_class_specs(
    cfg: SshConfig,
    new_class_id: i64,
    source_class_id: i64,
    assignments: Vec<SpecAssignment>,
) -> Result<String, String> {
    specs::apply_class_specs(&cfg, new_class_id, source_class_id, &assignments)
}

#[tauri::command]
fn save_project_file(path: String, json: String) -> Result<(), String> {
    std::fs::write(&path, json).map_err(|e| format!("could not write {path}: {e}"))
}

#[tauri::command]
fn load_project_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("could not read {path}: {e}"))
}

#[tauri::command]
fn list_deployed_classes(cfg: SshConfig) -> Result<Vec<DeployedClass>, String> {
    deployed::list_deployed_classes(&cfg)
}

#[tauri::command]
fn delete_deployed_class(
    cfg: SshConfig,
    filename: String,
    class_id: i64,
    cleanup_cloned_specs: bool,
) -> Result<String, String> {
    deployed::delete_deployed_class(&cfg, &filename, class_id, cleanup_cloned_specs)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            list_specializations,
            list_class_specs,
            apply_class_specs,
            save_project_file,
            load_project_file,
            list_deployed_classes,
            delete_deployed_class,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
