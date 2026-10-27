//! Clones a source class's Realm Abilities onto a new class - a real, reported gap found live:
//! Drengr (cloned from Warrior) had zero Realm Abilities at all, even though the whole point of
//! cloning from an existing, fully-built class is to inherit everything it offers. Confirmed in
//! the real server source and by direct comparison in the DB: `ClassXRealmAbility_Atlas` (which
//! Realm Abilities a class can train, keyed by `CharClass` - the server's raw numeric class ID)
//! has exactly the same "zero DB inheritance" shape `specs.rs`'s own module doc comment already
//! documents for `ClassXSpecialization` - a cloned class's C# subclassing relationship carries no
//! weight here at all; Warrior's own 32 real Atlas Realm Abilities belong to ClassId 22
//! specifically, and nothing automatically extends that to a brand-new ClassId.
//!
//! Deliberately a plain, one-time "copy everything the source currently has" - not yet a
//! per-ability picker (a real, explicitly deferred follow-up the owner already flagged wanting
//! later). Idempotent: a class that already has *any* Realm Ability rows of its own is left
//! completely alone, never re-copied or overwritten - this is what actually makes the one-time
//! copy safe to call unconditionally on every deploy (the natural redeploy flow already relies on
//! elsewhere in this tool), and it's also what will let a future picker safely customize a
//! class's list later without this function undoing that customization on the next redeploy.

use crate::deploy::{SshConfig, run_remote_sql, run_remote_sql_json};
use serde_json::Value;
use uuid::Uuid;

fn str_field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn sql_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// Copies every Realm Ability `source_class_id` currently has onto `new_class_id`, each under its
/// own freshly generated `ClassXRealmAbility_Atlas_ID` (a GUID-style primary key the real schema
/// requires explicitly, not an autoincrement DB rowid - the same hidden-PK gotcha class already
/// documented at length in `specs.rs`). A clean no-op if `new_class_id` already has any Realm
/// Ability of its own (nothing to safely redo - see this module's own doc comment for why that
/// matters), or if `source_class_id` has none to copy in the first place.
pub fn clone_realm_abilities(
    cfg: &SshConfig,
    source_class_id: i64,
    new_class_id: i64,
) -> Result<String, String> {
    let already_has_own = run_remote_sql_json(
        cfg,
        &format!("SELECT 1 FROM ClassXRealmAbility_Atlas WHERE CharClass = {new_class_id};"),
    )?;
    if !already_has_own.is_empty() {
        return Ok(String::new());
    }

    let source_abilities = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT AbilityKey FROM ClassXRealmAbility_Atlas WHERE CharClass = {source_class_id};"
        ),
    )?;
    if source_abilities.is_empty() {
        return Ok(String::new());
    }

    let mut batch = String::new();
    for row in &source_abilities {
        let ability_key_q = sql_quote(&str_field(row, "AbilityKey"));
        let row_guid = Uuid::new_v4();
        batch.push_str(&format!(
            "INSERT INTO ClassXRealmAbility_Atlas (CharClass, AbilityKey, ClassXRealmAbility_Atlas_ID) \
             VALUES ({new_class_id}, '{ability_key_q}', '{row_guid}');\n"
        ));
    }
    run_remote_sql(cfg, &batch)?;

    Ok(format!(
        "copied {} Realm Abilit{} from class {} to class {} (first deploy only - a class that already has its own Realm Abilities is never auto-overwritten)\n",
        source_abilities.len(),
        if source_abilities.len() == 1 { "y" } else { "ies" },
        source_class_id,
        new_class_id
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::SshConfig;
    use std::process::Command;

    fn test_cfg(dir: &std::path::Path) -> SshConfig {
        SshConfig {
            host: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            remote_server_dir: dir.to_string_lossy().to_string(),
            start_script_name: "daoc-server.sh".to_string(),
        }
    }

    fn setup_test_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("daoc-class-editor-ra-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let db = dir.join("data/opendaoc.sqlite3.db");
        let sql = "
            CREATE TABLE ClassXRealmAbility_Atlas (CharClass INTEGER, AbilityKey TEXT, ClassXRealmAbility_Atlas_ID TEXT PRIMARY KEY DEFAULT '');
            INSERT INTO ClassXRealmAbility_Atlas (CharClass, AbilityKey, ClassXRealmAbility_Atlas_ID) VALUES (22, 'AtlasOF_AugStr', 'seed-1');
            INSERT INTO ClassXRealmAbility_Atlas (CharClass, AbilityKey, ClassXRealmAbility_Atlas_ID) VALUES (22, 'AtlasOF_MasteryOfArms', 'seed-2');
            INSERT INTO ClassXRealmAbility_Atlas (CharClass, AbilityKey, ClassXRealmAbility_Atlas_ID) VALUES (22, 'AtlasOF_AugDex', 'seed-3');
        ";
        let status = Command::new("sqlite3")
            .arg(&db)
            .arg(sql)
            .status()
            .expect("sqlite3 must be on PATH for this test");
        assert!(status.success());
        dir
    }

    #[test]
    fn clones_every_realm_ability_from_the_source_class_with_fresh_ids() {
        let dir = setup_test_db("clone-fresh");
        let cfg = test_cfg(&dir);

        let log = clone_realm_abilities(&cfg, 22, 65).unwrap();
        assert!(log.contains("copied 3 Realm Abilities"), "{log}");

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT AbilityKey, ClassXRealmAbility_Atlas_ID FROM ClassXRealmAbility_Atlas WHERE CharClass = 65 ORDER BY AbilityKey;",
        )
        .unwrap();
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(str_field(&rows[0], "AbilityKey"), "AtlasOF_AugDex");
        assert_eq!(str_field(&rows[1], "AbilityKey"), "AtlasOF_AugStr");
        assert_eq!(str_field(&rows[2], "AbilityKey"), "AtlasOF_MasteryOfArms");
        // Every row must get its own freshly generated ID, not the source's own seed values -
        // the same "hidden GUID PK, never copy verbatim" gotcha this tool has hit before.
        let ids: std::collections::HashSet<String> = rows
            .iter()
            .map(|r| str_field(r, "ClassXRealmAbility_Atlas_ID"))
            .collect();
        assert_eq!(ids.len(), 3, "every row must have its own unique ID: {ids:?}");
        assert!(
            ids.iter().all(|id| !id.starts_with("seed-")),
            "the source's own seed IDs must never be reused verbatim: {ids:?}"
        );

        // The source class's own rows must be untouched.
        let source_rows = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM ClassXRealmAbility_Atlas WHERE CharClass = 22;",
        )
        .unwrap();
        assert_eq!(source_rows.len(), 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_a_no_op_once_the_new_class_already_has_its_own_realm_abilities() {
        // A real, reproduced need this guards: once a future picker lets the owner customize a
        // class's own Realm Ability list away from its source's, redeploying the class must
        // never silently wipe that customization back to a fresh copy.
        let dir = setup_test_db("already-has-own");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO ClassXRealmAbility_Atlas (CharClass, AbilityKey, ClassXRealmAbility_Atlas_ID) VALUES (65, 'AtlasOF_CustomOnly', 'existing-1');",
        )
        .unwrap();

        let log = clone_realm_abilities(&cfg, 22, 65).unwrap();
        assert_eq!(log, "", "must be a silent no-op: {log}");

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT AbilityKey FROM ClassXRealmAbility_Atlas WHERE CharClass = 65;",
        )
        .unwrap();
        assert_eq!(rows.len(), 1, "the existing customization must survive untouched: {rows:?}");
        assert_eq!(str_field(&rows[0], "AbilityKey"), "AtlasOF_CustomOnly");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_a_no_op_if_the_source_class_has_no_realm_abilities_at_all() {
        let dir = setup_test_db("source-has-none");
        let cfg = test_cfg(&dir);

        let log = clone_realm_abilities(&cfg, 999, 65).unwrap();
        assert_eq!(log, "", "{log}");

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM ClassXRealmAbility_Atlas WHERE CharClass = 65;",
        )
        .unwrap();
        assert!(rows.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
