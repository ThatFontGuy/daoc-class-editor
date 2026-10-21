//! Lists and removes classes that have actually been deployed to a server's `scripts/` folder -
//! for debugging a clone that didn't work, or general cleanup, per the owner's request.
//!
//! Unlike the vendored reference classes (classdata.rs), a deployed custom class's attribute
//! carries a raw integer ID directly (`[CharacterClass(64, "Name", "Base")]`, from codegen.rs's
//! template) rather than an `(int)eCharacterClass.X` enum reference, since it has no enum entry
//! at all - so this uses its own, simpler parser rather than reusing classdata.rs's.

use crate::deploy::{
    SshConfig, is_local, run_command, run_remote_sql, run_remote_sql_json, ssh_base_args,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployedClass {
    pub filename: String,
    pub class_ident: String,
    pub name: String,
    pub id: i64,
    pub base_class_name: String,
}

fn parse_deployed(content: &str, filename: &str) -> Option<DeployedClass> {
    let attr_re =
        Regex::new(r#"\[CharacterClass\(\s*(\d+)\s*,\s*"([^"]+)"\s*,\s*"([^"]+)""#).ok()?;
    let caps = attr_re.captures(content)?;
    let id = caps[1].parse().ok()?;
    let name = caps[2].to_string();
    let base_class_name = caps[3].to_string();
    let class_re = Regex::new(r"(?m)^\s*public\s+class\s+(\w+)\s*:").ok()?;
    let class_ident = class_re.captures(content)?[1].to_string();
    Some(DeployedClass {
        filename: filename.to_string(),
        class_ident,
        name,
        id,
        base_class_name,
    })
}

const MARKER: &str = "===DAOC-CLASS-EDITOR-FILE===";

/// Every `.cs` file in `<server_dir>/scripts/` that looks like a class definition (has the
/// `[CharacterClass(...)]` attribute) - not every file in that folder is necessarily one of this
/// tool's clones, but anything that isn't a recognizable class definition is simply skipped.
pub fn list_deployed_classes(cfg: &SshConfig) -> Result<Vec<DeployedClass>, String> {
    let scripts_dir = Path::new(&cfg.remote_server_dir).join("scripts");
    if is_local(cfg) {
        if !scripts_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&scripts_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("cs") {
                continue;
            }
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            if let Some(info) = parse_deployed(&content, &filename) {
                out.push(info);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(out);
    }

    let remote_cmd = format!(
        "cd '{}/scripts' 2>/dev/null && for f in *.cs; do [ -f \"$f\" ] || continue; echo '{MARKER}'\"$f\"; cat \"$f\"; done",
        shell_escape(&cfg.remote_server_dir)
    );
    let mut cmd = Command::new("ssh");
    cmd.args(ssh_base_args(cfg))
        .arg(format!("{}@{}", cfg.user, cfg.host))
        .arg(remote_cmd);
    let output = run_command(cmd)?;
    let mut out = Vec::new();
    for chunk in output.split(MARKER).skip(1) {
        let (filename, content) = chunk.split_once('\n').unwrap_or((chunk, ""));
        if let Some(info) = parse_deployed(content, filename.trim()) {
            out.push(info);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\"'\"'")
}

fn str_field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Removes a deployed class: the `.cs` file, its `ClassXSpecialization` rows, and - if
/// `cleanup_cloned_specs` is set - every Specialization/SpellLine/LineXSpell/Style row whose
/// KeyName/Spec carries this exact class's `_<id>` suffix (the convention `apply_class_specs`'s
/// "clone" mode uses), so a reused/shared spec is never touched, only ones genuinely created for
/// this one class.
pub fn delete_deployed_class(
    cfg: &SshConfig,
    filename: &str,
    class_id: i64,
    cleanup_cloned_specs: bool,
) -> Result<String, String> {
    let mut log = String::new();

    let script_path = Path::new(&cfg.remote_server_dir)
        .join("scripts")
        .join(filename);
    if is_local(cfg) {
        std::fs::remove_file(&script_path)
            .map_err(|e| format!("could not remove {}: {e}", script_path.display()))?;
    } else {
        let mut cmd = Command::new("ssh");
        cmd.args(ssh_base_args(cfg))
            .arg(format!("{}@{}", cfg.user, cfg.host))
            .arg(format!(
                "rm -f '{}'",
                shell_escape(&script_path.to_string_lossy())
            ));
        run_command(cmd)?;
    }
    log.push_str(&format!("removed {}\n", script_path.display()));

    run_remote_sql(
        cfg,
        &format!("DELETE FROM ClassXSpecialization WHERE ClassID = {class_id};"),
    )?;
    log.push_str(&format!(
        "removed ClassXSpecialization rows for class {class_id}\n"
    ));

    if cleanup_cloned_specs {
        let suffix = format!("_{class_id}");

        let specs = run_remote_sql_json(cfg, "SELECT KeyName FROM Specialization;")?;
        let cloned_keys: Vec<String> = specs
            .iter()
            .map(|s| str_field(s, "KeyName"))
            .filter(|k| k.ends_with(&suffix))
            .collect();

        let lines = run_remote_sql_json(cfg, "SELECT KeyName, Spec FROM SpellLine;")?;
        let cloned_lines: Vec<String> = lines
            .iter()
            .filter(|l| cloned_keys.contains(&str_field(l, "Spec")))
            .map(|l| str_field(l, "KeyName"))
            .collect();

        for line_key in &cloned_lines {
            run_remote_sql(
                cfg,
                &format!(
                    "DELETE FROM LineXSpell WHERE LineName = '{}';",
                    sql_quote(line_key)
                ),
            )?;
        }
        if !cloned_lines.is_empty() {
            let list = cloned_lines
                .iter()
                .map(|k| format!("'{}'", sql_quote(k)))
                .collect::<Vec<_>>()
                .join(", ");
            run_remote_sql(
                cfg,
                &format!("DELETE FROM SpellLine WHERE KeyName IN ({list});"),
            )?;
        }

        for spec_key in &cloned_keys {
            run_remote_sql(
                cfg,
                &format!(
                    "DELETE FROM Style WHERE SpecKeyName = '{}';",
                    sql_quote(spec_key)
                ),
            )?;
            run_remote_sql(
                cfg,
                &format!(
                    "DELETE FROM Specialization WHERE KeyName = '{}';",
                    sql_quote(spec_key)
                ),
            )?;
        }
        log.push_str(&format!(
            "removed {} cloned spec(s), {} spell line(s)\n",
            cloned_keys.len(),
            cloned_lines.len()
        ));
    }

    Ok(log)
}

fn sql_quote(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn test_cfg(dir: &Path) -> SshConfig {
        SshConfig {
            host: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            remote_server_dir: dir.to_string_lossy().to_string(),
            start_script_name: "daoc-server.sh".to_string(),
        }
    }

    /// A fresh temp "server folder" with a `scripts/` folder holding one deployed custom class
    /// (`GrovekeeperClass.cs`, id 64, cloned from "Acolyte") and one unrelated `.cs` file that
    /// should be skipped entirely, plus a database seeded with that class's `ClassXSpecialization`
    /// row and spec data both cloned for class 64 (`earthshatter_64` + its spell line) and a
    /// stock, never-cloned spec (`slash`) that must survive any cleanup untouched.
    fn setup_test_server(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("daoc-class-editor-deployed-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::create_dir_all(dir.join("scripts")).unwrap();

        std::fs::write(
            dir.join("scripts/GrovekeeperClass.cs"),
            "using DOL.GS;\n\nnamespace DOL.GS\n{\n    [CharacterClass(64, \"Grovekeeper\", \"Acolyte\")]\n    public class ClassGrovekeeper : ClassAcolyte\n    {\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("scripts/NotAClass.cs"),
            "// just a helper file, no attribute here\n",
        )
        .unwrap();

        let db = dir.join("data/opendaoc.sqlite3.db");
        let sql = "
            CREATE TABLE Specialization (KeyName TEXT, Name TEXT, Description TEXT, Icon INTEGER, Implementation TEXT);
            CREATE TABLE ClassXSpecialization (ClassID INTEGER, SpecKeyName TEXT, LevelAcquired INTEGER);
            CREATE TABLE SpellLine (KeyName TEXT, Name TEXT, Spec TEXT, IsBaseLine INTEGER, ClassIDHint INTEGER);
            CREATE TABLE LineXSpell (LineName TEXT, SpellID INTEGER, Level INTEGER);
            CREATE TABLE Style (StyleID INTEGER PRIMARY KEY AUTOINCREMENT, ClassId INTEGER, Name TEXT, SpecKeyName TEXT);

            INSERT INTO Specialization VALUES ('slash', 'Slash', 'Slashing weapons', 100, NULL);
            INSERT INTO Specialization VALUES ('earthshatter_64', 'Earthshatter', 'Holy damage spells', 200, NULL);
            INSERT INTO ClassXSpecialization VALUES (1, 'slash', 0);
            INSERT INTO ClassXSpecialization VALUES (64, 'earthshatter_64', 0);
            INSERT INTO SpellLine VALUES ('earthshatter_64_smite_base', 'Earthshatter Spells', 'earthshatter_64', 0, 64);
            INSERT INTO LineXSpell VALUES ('earthshatter_64_smite_base', 9001, 1);
            INSERT INTO Style VALUES (NULL, 64, 'Shatter', 'earthshatter_64');
        ";
        let status = Command::new("sqlite3").arg(&db).arg(sql).status().unwrap();
        assert!(status.success());
        dir
    }

    #[test]
    fn lists_only_recognizable_class_files() {
        let dir = setup_test_server("list");
        let cfg = test_cfg(&dir);

        let classes = list_deployed_classes(&cfg).unwrap();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].filename, "GrovekeeperClass.cs");
        assert_eq!(classes[0].class_ident, "ClassGrovekeeper");
        assert_eq!(classes[0].name, "Grovekeeper");
        assert_eq!(classes[0].id, 64);
        assert_eq!(classes[0].base_class_name, "Acolyte");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_without_cleanup_only_removes_file_and_classxspec_row() {
        let dir = setup_test_server("delete-plain");
        let cfg = test_cfg(&dir);

        delete_deployed_class(&cfg, "GrovekeeperClass.cs", 64, false).unwrap();

        assert!(!dir.join("scripts/GrovekeeperClass.cs").exists());
        let rows = run_remote_sql_json(
            &cfg,
            "SELECT * FROM ClassXSpecialization WHERE ClassID = 64;",
        )
        .unwrap();
        assert!(rows.is_empty());
        // Without cleanup, the cloned Specialization/SpellLine rows are left behind untouched.
        let specs = run_remote_sql_json(&cfg, "SELECT KeyName FROM Specialization;").unwrap();
        assert_eq!(specs.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_with_cleanup_removes_only_the_cloned_rows_for_that_class() {
        let dir = setup_test_server("delete-cleanup");
        let cfg = test_cfg(&dir);

        delete_deployed_class(&cfg, "GrovekeeperClass.cs", 64, true).unwrap();

        let specs = run_remote_sql_json(&cfg, "SELECT KeyName FROM Specialization;").unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(str_field(&specs[0], "KeyName"), "slash");

        let lines = run_remote_sql_json(&cfg, "SELECT KeyName FROM SpellLine;").unwrap();
        assert!(lines.is_empty());
        let line_spells = run_remote_sql_json(&cfg, "SELECT * FROM LineXSpell;").unwrap();
        assert!(line_spells.is_empty());
        let styles = run_remote_sql_json(
            &cfg,
            "SELECT * FROM Style WHERE SpecKeyName = 'earthshatter_64';",
        )
        .unwrap();
        assert!(styles.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
