//! Standalone CLI export tool for the Character Planner (`Charplanner/`) - deliberately a
//! separate binary, not a button in the GUI app, per the owner's own explicit preference: this
//! is meant to be run as a local script ("Run a local .sh script that regenerates the data,
//! upload the resulting website pages to my server manually"), kept entirely out of the normal
//! write-gated GUI workflow since it never writes anything to the live server at all - purely a
//! read-only export of whatever's already there into one JSON file this app's own frontend code
//! never touches.
//!
//! Reuses every already-tested piece of this project's own data layer directly (as a second
//! binary target in the same Cargo package, linking against the `daoc_class_editor_lib` crate -
//! see Cargo.toml's own `[lib]`/this file's location under `src/bin/`) rather than re-implementing
//! any of class/spec/spell/style parsing a second time in a different tool: the exact same
//! `classdata`/`specs`/`deployed` modules the GUI app's own Tauri commands call are called here
//! too, so there is only ever one place that logic can drift out of sync with the real schema.
//!
//! Realm Ability training costs are the one piece of real data this project had never needed
//! before this export: see `realm_ability_costs.rs`'s own module doc comment for why a shared,
//! resolver-based approach (not a hardcoded per-ability table) was the only honest way to get
//! these right at all, since they're compiled into the server's own C# source, never the database.

use daoc_class_editor_lib::{classdata, deploy, deployed, realm_ability_costs, specs};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// The same curated field lists `ClassSpecsSection.tsx`'s own quick-edit form uses
// (`EDITABLE_SPELL_FIELD_HINTS`/`EDITABLE_STYLE_FIELD_HINTS`) - a `Spell`/`Style` row's real
// schema has dozens of columns that vary by server fork and aren't worth exporting at all; these
// are the ones a player actually cares about seeing on a character planner.
const SPELL_FIELDS: &[&str] = &[
    "Name",
    "Type",
    "Description",
    "Message1",
    "Message2",
    "Damage",
    "DamageType",
    "Value",
    "Range",
    "Radius",
    "Duration",
    "Frequency",
    "Power",
    "CastTime",
    "RecastDelay",
    "Target",
];

const STYLE_FIELDS: &[&str] = &[
    "Name",
    "EnduranceCost",
    "BonusToHit",
    "BonusToDefense",
    "GrowthOffset",
    "GrowthRate",
    "WeaponTypeRequirement",
    "AttackResultRequirement",
    "OpeningRequirementType",
    "OpeningRequirementValue",
    "StealthRequirement",
    "ArmorHitLocation",
    // Not shown directly in the quick-edit form (ClassSpecsSection.tsx's own
    // EDITABLE_STYLE_FIELD_HINTS doesn't include it either) - this is Style's *other* ID column,
    // separate from the real DB primary key StyleID, that a chain style's own
    // OpeningRequirementValue actually references (confirmed in specs.rs's own clone_spec chain
    // -repoint logic). Exported purely so the Character Planner can resolve a chain requirement
    // to the prerequisite style's real Name instead of a meaningless raw number.
    "ID",
];

fn pick_fields(fields: &Value, keys: &[&str]) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    if let Some(obj) = fields.as_object() {
        for k in keys {
            if let Some(v) = obj.get(*k) {
                out.insert(k.to_string(), v.clone());
            }
        }
    }
    out
}

fn str_col(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Always resolved relative to this crate's own manifest dir (`src-tauri/`), the same dev
/// fallback `lib.rs`'s own `resources_dir` uses - this tool is only ever run via `cargo run`
/// from a source checkout, never from a bundled app, so there is no installed-resource-dir case
/// to handle here at all. Note this is `resources/classes` itself (what `classdata::load_all_classes`
/// expects - it appends `core/` internally), one level *above* where the vendored realm-ability
/// source actually lives (`resources/classes/core/realmabilities/` - see `ra_core_dir`).
fn classes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/classes")
}

/// `RealmAbilityCostResolver::load` expects the dir that directly contains `realmabilities/`
/// (`resources/classes/core`), not `classes_dir()` itself - a real bug found while first testing
/// this tool end to end: passing `classes_dir()` here silently loaded zero vendored realm-ability
/// classes at all (the `realmabilities/` folder simply wasn't there), so every single ability's
/// cost/max level came back `None` even though the exact same resolver, given the right path,
/// already resolved all 126 of the live server's own realm abilities correctly in its own tests.
fn ra_core_dir() -> PathBuf {
    classes_dir().join("core")
}

fn charplanner_data_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri always has a parent (the repo root)")
        .join("Charplanner/data/classes.json")
}

/// Reads the real, live `use_new_actives_ras_scaling`/`use_new_passives_ras_scaling`
/// `ServerProperty` rows - see `realm_ability_costs::ScalingFlags`'s own doc comment for why
/// these specifically (not any other property) matter to cost resolution. Missing rows default
/// to `false`, matching the real server's own property-loading default for an unset boolean.
fn fetch_scaling_flags(cfg: &deploy::SshConfig) -> realm_ability_costs::ScalingFlags {
    let rows = deploy::run_remote_sql_json(
        cfg,
        "SELECT Key, Value FROM ServerProperty WHERE Key IN ('use_new_actives_ras_scaling', 'use_new_passives_ras_scaling');",
    )
    .unwrap_or_default();
    let mut new_actives = false;
    let mut new_passives = false;
    for row in &rows {
        let key = str_col(row, "Key");
        let val = str_col(row, "Value").eq_ignore_ascii_case("true");
        match key.as_str() {
            "use_new_actives_ras_scaling" => new_actives = val,
            "use_new_passives_ras_scaling" => new_passives = val,
            _ => {}
        }
    }
    realm_ability_costs::ScalingFlags { new_actives, new_passives }
}

fn export_one_class(
    cfg: &deploy::SshConfig,
    class: &classdata::ClassInfo,
    disabled: bool,
    ra_resolver: &realm_ability_costs::RealmAbilityCostResolver,
    scaling: realm_ability_costs::ScalingFlags,
) -> Result<Value, String> {
    let class_id = class.id as i64;
    let summaries = specs::list_class_spec_summaries(cfg, class_id)?;

    let mut spec_values = Vec::new();
    for s in &summaries {
        // Internal server-only bookkeeping specs (e.g. "Viking Career", "Warrior Career" -
        // confirmed live: these carry a negative sentinel LevelAcquired like -1000/-100 and
        // grant neither styles nor spell lines of their own) aren't something a player ever
        // actually trains or sees in the trainer window - skip them rather than cluttering the
        // planner with specs that have nothing to show either way.
        if !s.has_styles && !s.has_spell_lines {
            continue;
        }
        let mut styles_json = Vec::new();
        let mut spell_lines_json = Vec::new();

        if s.has_styles {
            let styles = specs::list_styles_for_spec(cfg, class_id, &s.key_name)?;
            for st in &styles {
                let mut obj = pick_fields(&st.fields, STYLE_FIELDS);
                // SpecLevelRequirement isn't in STYLE_FIELDS (it's not meant to be a quick-edit
                // field - see ClassSpecsSection.tsx's own EDITABLE_STYLE_FIELD_HINTS comment,
                // "it has its own quick-edit in the Level column instead"), so it has to be read
                // straight from the style's own full fields object, not the already-filtered one.
                let level = st.fields.get("SpecLevelRequirement").cloned().unwrap_or(Value::Null);
                obj.insert("level".to_string(), level);
                styles_json.push(Value::Object(obj));
            }
        }
        // Combat classification takes priority - see ClassSpecsSection.tsx's own doc comment
        // ("Shields" has both styles and an incidental spell line) for why has_styles implies
        // this spec is never also shown as a spell line set.
        if s.has_spell_lines && !s.has_styles {
            let lines = specs::list_spell_lines_for_spec(cfg, &s.key_name)?;
            for line in &lines {
                let spells_json: Vec<Value> = line
                    .spells
                    .iter()
                    .map(|sp| {
                        let mut obj = pick_fields(&sp.fields, SPELL_FIELDS);
                        obj.insert("level".to_string(), json!(sp.level));
                        Value::Object(obj)
                    })
                    .collect();
                spell_lines_json.push(json!({
                    "line_name": line.line_name,
                    "spells": spells_json,
                }));
            }
        }

        spec_values.push(json!({
            "key_name": s.key_name,
            "name": s.name,
            "level_acquired": s.level_acquired,
            "styles": styles_json,
            "spell_lines": spell_lines_json,
        }));
    }

    let ra_rows = deploy::run_remote_sql_json(
        cfg,
        &format!(
            "SELECT a.KeyName, a.Name, COALESCE(a.Description,'') AS Description, a.Implementation \
             FROM Ability a JOIN ClassXRealmAbility_Atlas c ON c.AbilityKey = a.KeyName \
             WHERE c.CharClass = {class_id} ORDER BY a.Name;"
        ),
    )?;
    let realm_abilities: Vec<Value> = ra_rows
        .iter()
        .map(|row| {
            let implementation = str_col(row, "Implementation");
            let cost_info = ra_resolver.resolve(&implementation, scaling);
            json!({
                "key_name": str_col(row, "KeyName"),
                "name": str_col(row, "Name"),
                "description": str_col(row, "Description"),
                "max_level": cost_info.max_level,
                "costs": cost_info.costs,
                "amounts": cost_info.amounts,
            })
        })
        .collect();

    Ok(json!({
        "id": class.id,
        "name": class.name,
        "realm": class.realm,
        "profession": class.profession,
        "primary_stat": class.primary_stat,
        "secondary_stat": class.secondary_stat,
        "tertiary_stat": class.tertiary_stat,
        "mana_stat": class.mana_stat,
        "spec_points_multiplier": class.specialization_multiplier,
        "eligible_races": class.effective_eligible_races,
        "autotrainable_specs": class.autotrainable_specs,
        "is_custom": class.is_custom,
        "disabled": disabled,
        "specs": spec_values,
        "realm_abilities": realm_abilities,
    }))
}

fn main() {
    let cfg = deploy::load_config();
    if cfg.remote_server_dir.trim().is_empty() {
        eprintln!(
            "No server connection is configured yet. Open the DAoC Class Editor app once, fill \
             in Server connection, and save it - this tool reuses that same saved config."
        );
        std::process::exit(1);
    }

    let classes_dir = classes_dir();
    let mut all_classes = classdata::load_all_classes(&classes_dir);
    match deployed::fetch_deployed_class_infos(&cfg) {
        Ok(extra) => all_classes = classdata::load_all_classes_with_extra(&classes_dir, extra),
        Err(e) => eprintln!("warning: could not fetch deployed classes ({e}) - vendored classes only"),
    }
    let leaf_classes: Vec<_> = all_classes
        .into_iter()
        .filter(|c| c.parent_ident != "CharacterClassBase")
        .collect();

    let disabled_ids: HashSet<i32> = deployed::fetch_disabled_class_ids(&cfg).unwrap_or_default();

    eprintln!("Loading Realm Ability cost tables from vendored server source...");
    let ra_resolver = realm_ability_costs::RealmAbilityCostResolver::load(&ra_core_dir());
    let scaling = fetch_scaling_flags(&cfg);

    let mut exported = Vec::new();
    let mut failed = Vec::new();
    for class in &leaf_classes {
        eprintln!("exporting {} (id {})...", class.name, class.id);
        match export_one_class(&cfg, class, disabled_ids.contains(&class.id), &ra_resolver, scaling) {
            Ok(v) => exported.push(v),
            Err(e) => {
                eprintln!("  SKIPPED {}: {e}", class.name);
                failed.push(class.name.clone());
            }
        }
    }

    let generated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let output = json!({
        "generated_at_unix": generated_at,
        "max_level": 50,
        "classes": exported,
    });

    let out_path = charplanner_data_path();
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("could not create Charplanner/data/");
    }
    std::fs::write(&out_path, serde_json::to_string_pretty(&output).unwrap())
        .unwrap_or_else(|e| panic!("could not write {}: {e}", out_path.display()));

    eprintln!(
        "\nWrote {} classes to {}{}",
        exported.len(),
        out_path.display(),
        if failed.is_empty() {
            String::new()
        } else {
            format!(" ({} skipped: {})", failed.len(), failed.join(", "))
        }
    );

    restamp_cache_buster(generated_at);
}

/// Rewrites `planner.html`'s own `<script src="planner.js?v=...">` tag to a fresh version number
/// every time this tool runs - a real problem found while first testing this tool end to end:
/// without this, a browser that already cached an older `planner.js` (python's `http.server`,
/// and most real webservers, send no explicit `Cache-Control` header at all, so a browser is free
/// to reuse its own disk cache heuristically) would keep showing stale planner logic even after
/// the folder was re-uploaded with real fixes in it, with nothing on the page itself to indicate
/// anything was wrong. Self-maintaining: every time the data is regenerated, the one HTML file
/// that references the JS gets a guaranteed-fresh query string too, so a simple re-upload of the
/// whole folder is all that's ever needed - no separate manual step to remember.
fn restamp_cache_buster(generated_at: u64) {
    let html_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("Charplanner/planner.html");
    let Ok(html) = std::fs::read_to_string(&html_path) else {
        return;
    };
    let re = regex::Regex::new(r#"planner\.js(\?v=\d+)?""#).unwrap();
    let replaced = re.replace(&html, format!(r#"planner.js?v={generated_at}""#));
    let _ = std::fs::write(&html_path, replaced.as_ref());
}
