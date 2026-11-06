mod classdata;
mod codegen;
mod deploy;
mod deployed;
mod realm_abilities;
mod spell_ownership;
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
    deploy::ensure_safe_to_write(&cfg)?;
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
    deploy::ensure_safe_to_write(&cfg)?;
    deploy::deploy_class_file(&cfg, &filename, &content)
}

#[tauri::command]
fn run_remote_sql(cfg: SshConfig, sql: String) -> Result<String, String> {
    deploy::ensure_safe_to_write(&cfg)?;
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
    deploy::ensure_safe_to_write(&cfg)?;
    specs::apply_class_specs(&cfg, new_class_id, &assignments, &previous_keys)
}

#[tauri::command]
fn clone_realm_abilities(
    cfg: SshConfig,
    source_class_id: i64,
    new_class_id: i64,
) -> Result<String, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    realm_abilities::clone_realm_abilities(&cfg, source_class_id, new_class_id)
}

#[derive(serde::Serialize, Clone)]
struct MigrateProgress {
    processed: usize,
    total: usize,
}

#[tauri::command]
fn migrate_class_to_owned_spells(
    app: tauri::AppHandle,
    cfg: SshConfig,
    class_id: i64,
) -> Result<String, String> {
    use tauri::Emitter;
    deploy::ensure_safe_to_write(&cfg)?;
    specs::migrate_class_to_owned_spells(&cfg, class_id, |processed, total| {
        let _ = app.emit("migrate-progress", MigrateProgress { processed, total });
    })
}

#[tauri::command]
fn list_owned_spells(cfg: SshConfig, class_id: i64) -> Result<Vec<serde_json::Value>, String> {
    spell_ownership::list_owned_spells(&cfg, class_id)
}

#[tauri::command]
fn list_class_spec_summaries(cfg: SshConfig, class_id: i64) -> Result<Vec<specs::ClassSpecSummary>, String> {
    specs::list_class_spec_summaries(&cfg, class_id)
}

#[tauri::command]
fn list_spell_lines_for_spec(cfg: SshConfig, spec_key: String) -> Result<Vec<specs::SpellLineGroup>, String> {
    specs::list_spell_lines_for_spec(&cfg, &spec_key)
}

#[tauri::command]
fn list_styles_for_spec(cfg: SshConfig, class_id: i64, spec_key: String) -> Result<Vec<specs::StyleInfo>, String> {
    specs::list_styles_for_spec(&cfg, class_id, &spec_key)
}

#[tauri::command]
fn create_new_combat_spec(
    cfg: SshConfig,
    class_id: i64,
    name: String,
    level_acquired: i64,
) -> Result<specs::ClassSpecSummary, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    specs::create_new_combat_spec(&cfg, class_id, &name, level_acquired)
}

#[tauri::command]
fn create_new_spell_spec(
    cfg: SshConfig,
    class_id: i64,
    name: String,
    level_acquired: i64,
) -> Result<specs::ClassSpecSummary, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    specs::create_new_spell_spec(&cfg, class_id, &name, level_acquired)
}

#[tauri::command]
fn update_line_spell_level(
    cfg: SshConfig,
    class_id: i64,
    line_xspell_id: String,
    new_level: i64,
) -> Result<String, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    specs::update_line_spell_level(&cfg, class_id, &line_xspell_id, new_level)
}

#[tauri::command]
fn update_style_level(
    cfg: SshConfig,
    class_id: i64,
    spec_key: String,
    style_id: i64,
    new_level: i64,
) -> Result<String, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    specs::update_style_level(&cfg, class_id, &spec_key, style_id, new_level)
}

#[tauri::command]
fn update_owned_spell_fields(
    cfg: SshConfig,
    spell_id: i64,
    fields: std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    spell_ownership::update_owned_spell_fields(&cfg, spell_id, &fields)
}

#[derive(serde::Serialize)]
struct SaveLineSpellResult {
    spell_id: i64,
    message: String,
}

/// Clones a spell on the spot (if it isn't an independent copy for this class already) and
/// applies the field edits in one call - this is what lets the "Edit spells" section's own Save
/// button work immediately on any spell, with no separate "Migrate" step needed first.
#[tauri::command]
fn save_line_spell_fields(
    cfg: SshConfig,
    class_id: i64,
    line_xspell_id: String,
    fields: std::collections::HashMap<String, serde_json::Value>,
) -> Result<SaveLineSpellResult, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    let spell_id = specs::ensure_line_spell_owned(&cfg, class_id, &line_xspell_id)?;
    let message = if fields.is_empty() {
        format!("cloned spell {spell_id} independently for this class")
    } else {
        spell_ownership::update_owned_spell_fields(&cfg, spell_id, &fields)?
    };
    Ok(SaveLineSpellResult { spell_id, message })
}

#[derive(serde::Serialize)]
struct SaveStyleResult {
    style_id: i64,
    message: String,
}

/// Grants/clones this class its own copy of a combat style on the spot (if it doesn't have one
/// already) and applies the field edits in one call - the combat-spec equivalent of
/// `save_line_spell_fields`.
#[tauri::command]
fn save_style_fields(
    cfg: SshConfig,
    class_id: i64,
    spec_key: String,
    style_id: i64,
    fields: std::collections::HashMap<String, serde_json::Value>,
) -> Result<SaveStyleResult, String> {
    deploy::ensure_safe_to_write(&cfg)?;
    let owned_style_id = specs::ensure_style_owned(&cfg, class_id, &spec_key, style_id)?;
    let message = if fields.is_empty() {
        format!("cloned style {owned_style_id} independently for this class")
    } else {
        specs::update_style_fields(&cfg, class_id, owned_style_id, &fields)?
    };
    Ok(SaveStyleResult { style_id: owned_style_id, message })
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
    deploy::ensure_safe_to_write(&cfg)?;
    deployed::delete_deployed_class(&cfg, &filename, class_id, cleanup_cloned_specs)
}

/// Every class ID the server's own `DISABLED_CLASSES` property currently blocks at character
/// creation for regular (non-GM) players - lets the Classes tab mark a disabled class instead of
/// just letting a player (or the owner) discover it only by trying to create one. Returned
/// sorted purely for a stable, readable order in the UI/logs - the set itself has no ordering.
#[tauri::command]
fn list_disabled_class_ids(cfg: SshConfig) -> Result<Vec<i32>, String> {
    let mut ids: Vec<i32> = deployed::fetch_disabled_class_ids(&cfg)?.into_iter().collect();
    ids.sort_unstable();
    Ok(ids)
}

/// Both halves of the write-safety gate at once (server-running + backup freshness), in a single
/// round trip - the Server connection tab's own persistent status indicator polls this, and the
/// exact same two checks (`is_server_running`/`backup_status`) are what `ensure_safe_to_write`
/// itself relies on for every mutating command, so what the UI shows is always consistent with
/// what will actually be allowed.
#[derive(serde::Serialize)]
struct WriteSafetyStatus {
    server_running: bool,
    backup: deploy::BackupStatus,
}

#[tauri::command]
fn write_safety_status(cfg: SshConfig) -> Result<WriteSafetyStatus, String> {
    Ok(WriteSafetyStatus {
        server_running: deploy::is_server_running(&cfg)?,
        backup: deploy::backup_status(&cfg)?,
    })
}

#[tauri::command]
fn backup_database(cfg: SshConfig) -> Result<String, String> {
    deploy::backup_database(&cfg)
}

#[tauri::command]
fn revert_database(cfg: SshConfig) -> Result<String, String> {
    deploy::revert_database(&cfg)
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
            clone_realm_abilities,
            migrate_class_to_owned_spells,
            list_owned_spells,
            list_class_spec_summaries,
            list_spell_lines_for_spec,
            list_styles_for_spec,
            create_new_combat_spec,
            create_new_spell_spec,
            update_line_spell_level,
            update_style_level,
            update_owned_spell_fields,
            save_line_spell_fields,
            save_style_fields,
            save_project_file,
            load_project_file,
            delete_deployed_class,
            list_disabled_class_ids,
            write_safety_status,
            backup_database,
            revert_database,
            root_archetype,
            generate_trainer_code,
            find_trainer_npcs,
            spawn_trainer_npc,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
