//! Phase 2: specialization (spec line) assignment for a cloned class.
//!
//! The real bug this fixes: ClassXSpecialization is keyed by raw numeric ClassID with no
//! inheritance whatsoever (confirmed in SkillBase.cs's class-career loader) - so a cloned class,
//! even though its .cs file correctly subclasses an existing one, starts with zero specs, zero
//! combat styles, and zero weapon/armor proficiency (those flow from spec-granted abilities)
//! until ClassXSpecialization rows exist for its own new ID. Literal C# inheritance alone does
//! not carry this over; it has to be done here, as data.
//!
//! Every spec assignment is a full, independent clone - the Specialization row itself, its
//! SpellLine(s), their LineXSpell entries, and its Style (melee skill) rows are all duplicated
//! under a new KeyName/display Name the caller chooses. This used to also offer a "reuse" mode
//! (sharing the exact same spec/styles with the source class, no duplication) - removed after a
//! real, reproduced bug: `SkillBase.GetStyleList(specID, classId)` looks up a class-locked
//! style's availability by an *exact* class-ID match, so a reused spec whose styles are locked to
//! the *source* class's ID (a very common case for any real combat spec) would train and show a
//! level, but its styles would never actually appear for the new class at all - silently,
//! nothing in the UI or server logs flags it. Cloning always repoints a class-locked style's
//! ClassId to the new class, so this can't happen. Procs (StyleXSpell) are not cloned yet - a
//! known gap, noted in the UI.

use crate::deploy::{SshConfig, run_remote_sql, run_remote_sql_json};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecializationRow {
    pub key_name: String,
    pub name: String,
    pub description: String,
    pub icon: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassSpecRow {
    pub spec_key_name: String,
    pub spec_name: String,
    pub level_acquired: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecAssignment {
    pub source_key: String,
    pub new_key: String,
    pub new_name: String,
    pub level_acquired: i64,
}

fn str_field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn int_field(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// Every specialization that exists on the server, for the "reuse an existing spec" picker.
pub fn list_specializations(cfg: &SshConfig) -> Result<Vec<SpecializationRow>, String> {
    let rows = run_remote_sql_json(
        cfg,
        "SELECT KeyName, Name, COALESCE(Description,'') AS Description, Icon FROM Specialization ORDER BY Name;",
    )?;
    Ok(rows
        .iter()
        .map(|r| SpecializationRow {
            key_name: str_field(r, "KeyName"),
            name: str_field(r, "Name"),
            description: str_field(r, "Description"),
            icon: int_field(r, "Icon"),
        })
        .collect())
}

/// What a specific class (by numeric ID) currently has assigned - used to pre-fill the picker
/// with the source class's own specs when starting a clone.
pub fn list_class_specs(cfg: &SshConfig, class_id: i64) -> Result<Vec<ClassSpecRow>, String> {
    let sql = format!(
        "SELECT cxs.SpecKeyName AS SpecKeyName, cxs.LevelAcquired AS LevelAcquired, s.Name AS SpecName \
         FROM ClassXSpecialization cxs JOIN Specialization s ON s.KeyName = cxs.SpecKeyName \
         WHERE cxs.ClassID = {class_id} ORDER BY cxs.LevelAcquired, s.Name;"
    );
    let rows = run_remote_sql_json(cfg, &sql)?;
    Ok(rows
        .iter()
        .map(|r| ClassSpecRow {
            spec_key_name: str_field(r, "SpecKeyName"),
            spec_name: str_field(r, "SpecName"),
            level_acquired: int_field(r, "LevelAcquired"),
        })
        .collect())
}

fn sql_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// Applies a full set of spec assignments to `new_class_id` - every one is a clone (see the
/// module doc comment for why "reuse" was removed). Runs as one multi-statement SQL batch per
/// assignment (sqlite3 accepts semicolon-separated statements in a single invocation), so a
/// clone's several INSERTs either all land or the batch fails visibly - not a silent partial
/// clone.
pub fn apply_class_specs(
    cfg: &SshConfig,
    new_class_id: i64,
    assignments: &[SpecAssignment],
) -> Result<String, String> {
    let mut log = String::new();
    for a in assignments {
        // Embedding the new class's ID directly in the stored KeyName (not just the spell lines
        // under it) gives every row this clone creates a reliable, greppable suffix -
        // `delete_deployed_class` relies on exactly this to find and remove only what was cloned
        // for a given class, never a spec shared by other classes.
        let final_key = format!("{}_{new_class_id}", a.new_key);
        clone_spec(cfg, &a.source_key, &final_key, &a.new_name, new_class_id)?;
        let sql = format!(
            "INSERT INTO ClassXSpecialization (ClassID, SpecKeyName, LevelAcquired) VALUES ({new_class_id}, '{}', {});",
            sql_quote(&final_key),
            a.level_acquired
        );
        run_remote_sql(cfg, &sql)?;
        log.push_str(&format!(
            "cloned {} -> {} ('{}') at level {}\n",
            a.source_key, final_key, a.new_name, a.level_acquired
        ));
    }
    Ok(log)
}

/// Deep-clones one specialization: the Specialization row itself, every SpellLine attached to it
/// (plus their LineXSpell entries), and every Style attached to it - all under the new KeyName,
/// as a single SQL batch.
fn clone_spec(
    cfg: &SshConfig,
    source_key: &str,
    new_key: &str,
    new_name: &str,
    new_class_id: i64,
) -> Result<(), String> {
    let source_key_q = sql_quote(source_key);
    let new_key_q = sql_quote(new_key);
    let new_name_q = sql_quote(new_name);

    let mut batch = format!(
        "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation) \
         SELECT '{new_key_q}', '{new_name_q}', Description, Icon, Implementation \
         FROM Specialization WHERE KeyName = '{source_key_q}';\n"
    );

    // Spell lines attached to this spec each need their own new unique KeyName - read them first
    // so each clone statement can be built with a name we already know is unique (derived from
    // the new spec key plus the new class ID).
    let lines = run_remote_sql_json(
        cfg,
        &format!("SELECT KeyName, Name, IsBaseLine FROM SpellLine WHERE Spec = '{source_key_q}';"),
    )?;
    for line in &lines {
        let old_line_key = str_field(line, "KeyName");
        let line_name = str_field(line, "Name");
        let is_base = line.get("IsBaseLine").and_then(Value::as_i64).unwrap_or(0);
        let new_line_key = format!("{new_key}_{}", old_line_key);
        let old_line_key_q = sql_quote(&old_line_key);
        let new_line_key_q = sql_quote(&new_line_key);
        let line_name_q = sql_quote(&line_name);
        batch.push_str(&format!(
            "INSERT INTO SpellLine (KeyName, Name, Spec, IsBaseLine, ClassIDHint) \
             VALUES ('{new_line_key_q}', '{line_name_q}', '{new_key_q}', {is_base}, {new_class_id});\n"
        ));
        batch.push_str(&format!(
            "INSERT INTO LineXSpell (LineName, SpellID, Level) \
             SELECT '{new_line_key_q}', SpellID, Level FROM LineXSpell WHERE LineName = '{old_line_key_q}';\n"
        ));
    }

    // Styles: any non-zero ClassId locks a style to that *exact* class only - confirmed in the
    // real server source (Specialization.GetStylesForLiving only falls back to the ClassId=0
    // bucket when the player's own class has zero styles at all, so a non-matching lock hides a
    // style completely, silently). This isn't limited to styles locked to whatever class we're
    // cloning *from* - a spec can carry styles locked to an entirely unrelated third class (a
    // real, reproduced case: cloning "Celtic Dual" from a Midgard class, whose styles are
    // actually locked to three different classes, none of them the source or destination) - so
    // repoint *any* non-zero lock to the new class unconditionally. ClassId 0 (genuinely
    // unrestricted) is left as-is.
    //
    // GROUP BY Name, SpecLevelRequirement collapses exactly that "same style, one row per
    // originally-eligible class" pattern into a single row before the clone - without it, cloning
    // a spec like that would produce several duplicate, identically-named styles all now locked
    // to the same new class (confirmed real: Celtic Dual has every style name three times, once
    // per class, with every other field - EnduranceCost, Icon, etc. - identical between them).
    let style_class_expr = format!("CASE WHEN ClassId != 0 THEN {new_class_id} ELSE 0 END");
    batch.push_str(&format!(
        "INSERT INTO Style (ClassId, Name, SpecKeyName, SpecLevelRequirement, Icon, EnduranceCost, \
         StealthRequirement, OpeningRequirementType, OpeningRequirementValue, AttackResultRequirement, \
         WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, BonusToDefense, TwoHandAnimation, \
         RandomProc, ArmorHitLocation) \
         SELECT {style_class_expr}, Name, '{new_key_q}', SpecLevelRequirement, Icon, EnduranceCost, \
         StealthRequirement, OpeningRequirementType, OpeningRequirementValue, AttackResultRequirement, \
         WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, BonusToDefense, TwoHandAnimation, \
         RandomProc, ArmorHitLocation \
         FROM Style WHERE SpecKeyName = '{source_key_q}' GROUP BY Name, SpecLevelRequirement;\n"
    ));

    run_remote_sql(cfg, &batch)?;
    Ok(())
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

    /// A fresh temp "server folder" with data/opendaoc.sqlite3.db, seeded with a real-shaped
    /// schema and two sample specs: "Slash" (a pure melee spec, one Style, no spell line) and
    /// "Smite" (a caster spec, one SpellLine with two spells), both assigned to class 1 (the
    /// "source" class), mirroring a stock class's real data shape.
    fn setup_test_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("daoc-class-editor-specs-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let db = dir.join("data/opendaoc.sqlite3.db");
        let sql = "
            CREATE TABLE Specialization (KeyName TEXT, Name TEXT, Description TEXT, Icon INTEGER, Implementation TEXT);
            CREATE TABLE ClassXSpecialization (ClassID INTEGER, SpecKeyName TEXT, LevelAcquired INTEGER);
            CREATE TABLE SpellLine (KeyName TEXT, Name TEXT, Spec TEXT, IsBaseLine INTEGER, ClassIDHint INTEGER);
            CREATE TABLE LineXSpell (LineName TEXT, SpellID INTEGER, Level INTEGER);
            CREATE TABLE Style (StyleID INTEGER PRIMARY KEY AUTOINCREMENT, ClassId INTEGER, Name TEXT, SpecKeyName TEXT,
                SpecLevelRequirement INTEGER, Icon INTEGER, EnduranceCost INTEGER, StealthRequirement INTEGER,
                OpeningRequirementType INTEGER, OpeningRequirementValue INTEGER, AttackResultRequirement INTEGER,
                WeaponTypeRequirement INTEGER, GrowthOffset REAL, GrowthRate REAL, BonusToHit INTEGER,
                BonusToDefense INTEGER, TwoHandAnimation INTEGER, RandomProc INTEGER, ArmorHitLocation INTEGER);

            INSERT INTO Specialization VALUES ('slash', 'Slash', 'Slashing weapons', 100, NULL);
            INSERT INTO Specialization VALUES ('smite', 'Smite', 'Holy damage spells', 200, NULL);
            INSERT INTO ClassXSpecialization VALUES (1, 'slash', 0);
            INSERT INTO ClassXSpecialization VALUES (1, 'smite', 0);
            INSERT INTO Style VALUES (NULL, 1, 'Pulverize', 'slash', 5, 300, 5, 0, 1, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0);
            INSERT INTO Style VALUES (NULL, 0, 'Backlash', 'slash', 10, 301, 5, 1, 2, 1, 1, 1, 1.0, 1.0, 5, 0, 0, 0, 0);
            INSERT INTO SpellLine VALUES ('smite_base', 'Smiting Spells', 'smite', 0, 1);
            INSERT INTO LineXSpell VALUES ('smite_base', 9001, 1);
            INSERT INTO LineXSpell VALUES ('smite_base', 9002, 10);
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
    fn lists_specializations_and_class_specs() {
        let dir = setup_test_db("list");
        let cfg = test_cfg(&dir);

        let specs = list_specializations(&cfg).unwrap();
        assert_eq!(specs.len(), 2);
        assert!(
            specs
                .iter()
                .any(|s| s.key_name == "slash" && s.name == "Slash")
        );
        assert!(specs.iter().any(|s| s.key_name == "smite" && s.icon == 200));

        let class_specs = list_class_specs(&cfg, 1).unwrap();
        assert_eq!(class_specs.len(), 2);
        assert!(
            class_specs
                .iter()
                .any(|c| c.spec_key_name == "slash" && c.spec_name == "Slash")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_assignment_deep_clones_spec_spellline_and_styles() {
        let dir = setup_test_db("clone-melee");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "slash".to_string(),
            new_key: "earthshatter".to_string(),
            new_name: "Earthshatter".to_string(),
            level_acquired: 0,
        }];
        apply_class_specs(&cfg, 64, &assignments).unwrap();

        // The new Specialization row exists, independent from the source.
        let all_specs = list_specializations(&cfg).unwrap();
        assert_eq!(all_specs.len(), 3, "{all_specs:?}");
        let cloned = all_specs
            .iter()
            .find(|s| s.key_name == "earthshatter_64")
            .unwrap();
        assert_eq!(cloned.name, "Earthshatter");
        assert_eq!(
            cloned.icon, 100,
            "icon should be copied from the source spec"
        );

        // The new class is assigned the cloned spec, not the original.
        let new_class_specs = list_class_specs(&cfg, 64).unwrap();
        assert_eq!(new_class_specs.len(), 1);
        assert_eq!(new_class_specs[0].spec_key_name, "earthshatter_64");

        // Styles: 2 on the source ("slash"), both must now exist under the new spec key too -
        // one was ClassId=1 (locked to the source class, must now point at the new class), one
        // was ClassId=0 (unrestricted, must stay 0).
        let rows = run_remote_sql_json(
            &cfg,
            "SELECT Name, ClassId FROM Style WHERE SpecKeyName='earthshatter_64' ORDER BY Name;",
        )
        .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let pulverize = rows
            .iter()
            .find(|r| str_field(r, "Name") == "Pulverize")
            .unwrap();
        assert_eq!(
            int_field(pulverize, "ClassId"),
            64,
            "class-locked style should repoint to the new class"
        );
        let backlash = rows
            .iter()
            .find(|r| str_field(r, "Name") == "Backlash")
            .unwrap();
        assert_eq!(
            int_field(backlash, "ClassId"),
            0,
            "unrestricted style should stay unrestricted"
        );

        // Original "slash" spec and its styles must be completely untouched.
        let original_styles = run_remote_sql_json(
            &cfg,
            "SELECT Name, ClassId FROM Style WHERE SpecKeyName='slash' ORDER BY Name;",
        )
        .unwrap();
        assert_eq!(original_styles.len(), 2);
        let original_pulverize = original_styles
            .iter()
            .find(|r| str_field(r, "Name") == "Pulverize")
            .unwrap();
        assert_eq!(
            int_field(original_pulverize, "ClassId"),
            1,
            "source style must be untouched"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_repoints_a_style_locked_to_an_unrelated_third_class_too() {
        // Reproduces a real bug found live: "Celtic Dual" was reused (not cloned) by a class
        // cloned from Shadowblade, but its styles were locked to three *other* classes entirely -
        // not Shadowblade, not the new class. The old repoint logic only handled a lock matching
        // the literal class being cloned *from*, so it would have copied this style over still
        // locked to a class that isn't the new one either - just as invisible as before. The real
        // data also had the exact same style (same name/level/every other field) once per
        // eligible class - three rows for "Misty Gloom" alone - so this also confirms the clone
        // collapses those into one row instead of producing three duplicates all locked to the
        // same new class.
        let dir = setup_test_db("clone-third-party-lock");
        let cfg = test_cfg(&dir);
        let sql = "
            INSERT INTO Specialization VALUES ('dual', 'Celtic Dual', 'Dual wield', 300, NULL);
            INSERT INTO Style VALUES (NULL, 999, 'Misty Gloom', 'dual', 2, 400, 0, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0);
            INSERT INTO Style VALUES (NULL, 888, 'Misty Gloom', 'dual', 2, 400, 0, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0);
            INSERT INTO Style VALUES (NULL, 777, 'Misty Gloom', 'dual', 2, 400, 0, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0);
        ";
        let status = Command::new("sqlite3")
            .arg(dir.join("data/opendaoc.sqlite3.db"))
            .arg(sql)
            .status()
            .unwrap();
        assert!(status.success());

        let assignments = vec![SpecAssignment {
            source_key: "dual".to_string(),
            new_key: "nosferatusclaw".to_string(),
            new_name: "Nosferatu's Claw".to_string(),
            level_acquired: 0,
        }];
        // 999/888/777 are neither a class involved in the clone request at all - exactly the
        // real-world case this guards against.
        apply_class_specs(&cfg, 64, &assignments).unwrap();

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT ClassId FROM Style WHERE SpecKeyName='nosferatusclaw_64';",
        )
        .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "three same-name/level rows for different classes must collapse to one, not {rows:?}"
        );
        assert_eq!(
            int_field(&rows[0], "ClassId"),
            64,
            "a style locked to an unrelated third class must still repoint to the new class"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_assignment_deep_clones_spell_line_and_its_spells() {
        let dir = setup_test_db("clone-caster");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
        }];
        apply_class_specs(&cfg, 64, &assignments).unwrap();

        let lines = run_remote_sql_json(
            &cfg,
            "SELECT KeyName, Name, Spec, IsBaseLine FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let new_line_key = str_field(&lines[0], "KeyName");
        assert_ne!(
            new_line_key, "smite_base",
            "cloned line must have its own unique key"
        );
        assert_eq!(str_field(&lines[0], "Name"), "Smiting Spells");

        let spells = run_remote_sql_json(
            &cfg,
            &format!("SELECT SpellID, Level FROM LineXSpell WHERE LineName='{new_line_key}' ORDER BY Level;"),
        )
        .unwrap();
        assert_eq!(spells.len(), 2, "{spells:?}");
        assert_eq!(int_field(&spells[0], "SpellID"), 9001);
        assert_eq!(int_field(&spells[0], "Level"), 1);
        assert_eq!(int_field(&spells[1], "SpellID"), 9002);
        assert_eq!(int_field(&spells[1], "Level"), 10);

        // Original spell line/assignments untouched.
        let original_spells = run_remote_sql_json(
            &cfg,
            "SELECT SpellID FROM LineXSpell WHERE LineName='smite_base';",
        )
        .unwrap();
        assert_eq!(original_spells.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}
