mod classdata;
mod codegen;
mod deploy;
mod deployed;
mod specs;
mod trainer;

use classdata::{ClassInfo, KnownOptions};
use codegen::CloneRequest;
use deploy::SshConfig;
use specs::{ClassSpecRow, LineSpellRow, SpecAssignment, SpecSpellLineRow, SpecializationRow};
use tauri::Manager;
use trainer::{TrainerNpcCandidate, TrainerRequest};

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

#[derive(serde::Serialize)]
struct ClassListResult {
    classes: Vec<ClassInfo>,
    /// Set if the deployed-class lookup itself failed (e.g. server unreachable) - the vendored
    /// classes are still returned so browsing/cloning those isn't blocked by a bad connection.
    deployed_error: Option<String>,
}

#[tauri::command]
fn list_classes_with_deployed(
    app: tauri::AppHandle,
    cfg: SshConfig,
) -> Result<ClassListResult, String> {
    let dir = resources_dir(&app)?;
    let (extra, deployed_error) = match deployed::fetch_deployed_class_infos(&cfg) {
        Ok(classes) => (classes, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    Ok(ClassListResult {
        classes: classdata::load_all_classes_with_extra(&dir, extra),
        deployed_error,
    })
}

#[tauri::command]
fn suggest_next_id(app: tauri::AppHandle, cfg: SshConfig) -> Result<i32, String> {
    let dir = resources_dir(&app)?;
    let mut classes = classdata::load_all_classes(&dir);
    // Best-effort: if the server's unreachable, still suggest a ID that's free among the
    // vendored classes - better than blocking id suggestion entirely over a bad connection.
    if let Ok(deployed) = deployed::fetch_deployed_class_infos(&cfg) {
        classes.extend(deployed);
    }
    Ok(classdata::next_free_id_from(&classes))
}

/// Takes the frontend's already-fetched combined class list (vendored + deployed) rather than
/// refetching it, since the caller (the trainer-setup step) already has it in memory from
/// `list_classes_with_deployed` - this just reuses `classdata::root_archetype`'s tested
/// inheritance walk instead of duplicating it in TypeScript.
#[tauri::command]
fn root_archetype(classes: Vec<ClassInfo>, class_ident: String) -> Option<ClassInfo> {
    classdata::root_archetype(&classes, &class_ident).cloned()
}

#[tauri::command]
fn known_options(app: tauri::AppHandle) -> Result<KnownOptions, String> {
    let dir = resources_dir(&app)?;
    Ok(classdata::load_known_options(&dir))
}

#[tauri::command]
fn generate_trainer_code(req: TrainerRequest) -> String {
    trainer::generate_trainer_code(&req)
}

#[tauri::command]
fn find_trainer_npcs(cfg: SshConfig, search: String) -> Result<Vec<TrainerNpcCandidate>, String> {
    trainer::find_trainer_npcs(&cfg, &search)
}

#[tauri::command]
fn spawn_trainer_npc(
    cfg: SshConfig,
    reference_mob_id: String,
    npc_name: String,
    guild_name: String,
    class_type: String,
    offset_x: i64,
    offset_y: i64,
) -> Result<String, String> {
    trainer::spawn_trainer_npc(
        &cfg,
        &reference_mob_id,
        &npc_name,
        &guild_name,
        &class_type,
        offset_x,
        offset_y,
    )
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
fn list_spec_spell_lines(cfg: SshConfig, spec_key: String) -> Result<Vec<SpecSpellLineRow>, String> {
    specs::list_spec_spell_lines(&cfg, &spec_key)
}

#[tauri::command]
fn list_line_spells(cfg: SshConfig, line_key: String) -> Result<Vec<LineSpellRow>, String> {
    specs::list_line_spells(&cfg, &line_key)
}

#[tauri::command]
fn apply_class_specs(
    cfg: SshConfig,
    new_class_id: i64,
    assignments: Vec<SpecAssignment>,
    previous_keys: Vec<String>,
) -> Result<String, String> {
    specs::apply_class_specs(&cfg, new_class_id, &assignments, &previous_keys)
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
            list_classes_with_deployed,
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
            list_spec_spell_lines,
            list_line_spells,
            apply_class_specs,
            save_project_file,
            load_project_file,
            delete_deployed_class,
            root_archetype,
            generate_trainer_code,
            find_trainer_npcs,
            spawn_trainer_npc,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
