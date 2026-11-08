//! Tracks which `Spell` rows this tool has cloned specifically for one custom class, closing a
//! real gap found while auditing spec cloning: `clone_spec` (specs.rs) deep-clones
//! `Specialization`/`SpellLine`/`Style`/`SpecXAbility`, but its `LineXSpell` entries used to just
//! reuse the source's own literal `SpellID` verbatim - so a cloned spec's spells were never
//! actually independent rows, just new pointers at whatever the source (or any other class
//! sharing that same spell) already had. Editing a spell's own values (damage, duration, a
//! proc, ...) would have silently edited it for every class/spec line referencing that same row,
//! including the one it was cloned from. Style procs (`StyleXSpell`) had the same problem, and
//! were simply never cloned at all (a second, previously-undocumented-as-fixed gap).
//!
//! The fix: every spec/spell clone from now on gets its own fully independent `Spell` row (new
//! `SpellID`, every other column copied verbatim), and this module's own bookkeeping table
//! (`ClassEditorSpellClone` - not part of the real server schema, created on demand) records
//! `NewSpellID -> (SourceSpellID, OwningClassId)` for every one of them. That table is the single
//! source of truth for two things: safe deletion (never touch a `Spell` row this tool didn't
//! itself clone - a shared/un-owned ID must never be deleted or edited here), and the spell
//! editing UI's own "which spells can I safely edit for this class" query.

use crate::deploy::{SshConfig, run_remote_sql, run_remote_sql_json};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

fn int_field(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn str_field(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

pub(crate) fn sql_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// Renders any JSON scalar as a literal SQL value - a number as itself (preserving whatever
/// precision `sqlite3 -json` gave it, so a `REAL` column like `Style.GrowthRate` is never
/// round-tripped through an integer), text single-quote-escaped, `null`/missing as `NULL`. Used
/// wherever a field's value has to be read back into Rust and re-emitted as a literal (the spell
/// field editor); the clone paths below prefer copying a row via `SELECT ... FROM Spell WHERE
/// SpellID = ...` instead, specifically to avoid ever needing this for a value this tool doesn't
/// otherwise care about the contents of.
pub(crate) fn sql_literal(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => {
            if *b {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("'{}'", sql_quote(s)),
        other => format!("'{}'", sql_quote(&other.to_string())),
    }
}

const ENSURE_TRACKING_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS ClassEditorSpellClone (\
     NewSpellID INTEGER PRIMARY KEY, SourceSpellID INTEGER NOT NULL, OwningClassId INTEGER NOT NULL);";

pub fn ensure_tracking_table(cfg: &SshConfig) -> Result<(), String> {
    run_remote_sql(cfg, ENSURE_TRACKING_TABLE_SQL)?;
    Ok(())
}

/// Every real column of `table` except whatever its own primary key is - fetched fresh per call
/// (via `PRAGMA table_info`) rather than hardcoded, so this keeps working unmodified against any
/// server fork's own exact `Spell` schema (which has dozens of fields - damage, range, duration,
/// message text, etc. - and genuinely varies between forks), the same "rediscover the real
/// schema, don't assume it" approach this project's sibling class-ID renumbering tool already
/// uses for its own column auditing.
pub(crate) fn columns_excluding_pk(cfg: &SshConfig, table: &str) -> Result<Vec<String>, String> {
    let rows = run_remote_sql_json(cfg, &format!("PRAGMA table_info({table});"))?;
    let cols: Vec<String> = rows
        .iter()
        .filter(|r| int_field(r, "pk") == 0)
        .map(|r| str_field(r, "name"))
        .collect();
    if cols.is_empty() {
        return Err(format!(
            "could not read {table}'s own columns (PRAGMA table_info returned nothing - is the \
             table name right, and does the connection actually work?)"
        ));
    }
    Ok(cols)
}

/// For a table whose own primary key isn't a plain autoincrement rowid (SQLite only gives that
/// for an `INTEGER PRIMARY KEY` column) - e.g. the real schema's `LineXSpell_ID`/`StyleXSpell_ID`
/// style TEXT GUID columns, which this project found the hard way never auto-populate the way a
/// true rowid does (a bare `INSERT ... SELECT` producing several rows at once collided on the
/// column's shared default) - finds that column's name by introspecting the table's own schema
/// instead of hardcoding it, so this keeps working whether or not a given fork's exact naming
/// matches what's already been reverse-engineered elsewhere in this project. Returns `None` if
/// the table's primary key is already a plain `INTEGER PRIMARY KEY` (e.g. `Style.StyleID`),
/// which needs no explicit value at insert time at all.
pub(crate) fn explicit_guid_pk_column(cfg: &SshConfig, table: &str) -> Result<Option<String>, String> {
    let rows = run_remote_sql_json(cfg, &format!("PRAGMA table_info({table});"))?;
    for row in &rows {
        if int_field(row, "pk") != 0 {
            let col_type = str_field(row, "type").to_uppercase();
            if !col_type.contains("INT") {
                return Ok(Some(str_field(row, "name")));
            }
        }
    }
    Ok(None)
}

/// `StyleXSpell`'s own GUID-style primary key column, if it has one - resolved once per
/// `clone_spec` call (see `fn explicit_guid_pk_column`'s own doc comment) and threaded through
/// every proc this tool clones in that same call.
pub fn style_x_spell_guid_column(cfg: &SshConfig) -> Result<Option<String>, String> {
    explicit_guid_pk_column(cfg, "StyleXSpell")
}

/// Clones `Style` row `representative_style_id` under `new_spec_key_q` (already-`sql_quote`d),
/// with `ClassId` set to `class_expr` (a literal, e.g. `"64"` or `"0"`) and both `StyleID` and
/// `ID` set to the same freshly, atomically allocated value - returns that real new ID.
///
/// Computes its own new `StyleID` via a `(SELECT COALESCE(MAX(StyleID),0)+1 FROM Style)`
/// subquery evaluated *inside* the same `INSERT`, wrapped in `BEGIN IMMEDIATE`/`COMMIT` - never a
/// value pre-read into Rust earlier and incremented locally. See `SpellCloner::clone_for_class`'s
/// own doc comment for the real, reproduced bug class this avoids: a value guessed once and
/// incremented in Rust across many slow round-trips goes stale the instant anything else (a
/// second click before the first call finishes, or a second class cloned moments later) writes
/// to the same table in between - both end up computing the identical "next free" ID, and
/// whichever actually runs second hits a real `UNIQUE constraint failed`.
pub fn clone_style_with_fresh_id(
    cfg: &SshConfig,
    representative_style_id: i64,
    new_spec_key_q: &str,
    class_expr: &str,
) -> Result<i64, String> {
    let script = format!(
        "BEGIN IMMEDIATE;\n\
         INSERT INTO Style (StyleID, ClassId, Name, SpecKeyName, SpecLevelRequirement, Icon, \
         EnduranceCost, StealthRequirement, OpeningRequirementType, OpeningRequirementValue, \
         AttackResultRequirement, WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, \
         BonusToDefense, TwoHandAnimation, RandomProc, ArmorHitLocation, ID) \
         SELECT (SELECT COALESCE(MAX(StyleID), 0) + 1 FROM Style), {class_expr}, Name, \
         '{new_spec_key_q}', SpecLevelRequirement, Icon, EnduranceCost, StealthRequirement, \
         OpeningRequirementType, OpeningRequirementValue, AttackResultRequirement, \
         WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, BonusToDefense, \
         TwoHandAnimation, RandomProc, ArmorHitLocation, \
         (SELECT COALESCE(MAX(StyleID), 0) + 1 FROM Style) \
         FROM Style WHERE StyleID = {representative_style_id};\n\
         SELECT MAX(StyleID) AS NewStyleID FROM Style;\n\
         COMMIT;\n"
    );
    let rows = run_remote_sql_json(cfg, &script)?;
    rows.first()
        .map(|r| int_field(r, "NewStyleID"))
        .ok_or_else(|| {
            format!(
                "could not clone style {representative_style_id} - it may no longer exist \
                 (no row came back after insert)"
            )
        })
}

/// Allocates and clones independent `Spell` rows on demand for one owning class. Unlike most of
/// this tool's other cloning (which builds one big SQL batch string and executes it once at the
/// very end), every clone this makes executes - and is fully committed - immediately, so the
/// returned ID is always real, not merely planned; see `clone_for_class`'s own doc comment for
/// why.
pub struct SpellCloner {
    owning_class_id: i64,
    /// Every real `Spell` column except `SpellID` itself (needs a fresh, separately-unique
    /// numeric value on every clone - see `clone_for_class`'s own doc comment) and whatever the
    /// table's own real primary key is, if it's a distinct column from `SpellID` at all.
    columns: Vec<String>,
    /// `Spell`'s own GUID-style primary key column, if it has one distinct from `SpellID` - a
    /// real, reproduced case found live: the actual server schema's `Spell` table is primary-
    /// keyed on a separate `Spell_ID` (TEXT) column, with `SpellID` itself just an ordinary,
    /// *separately* unique-indexed business field (`U_Spell_SpellID`) - not the table's own SQL
    /// primary key at all, the same GUID-style-PK shape this project already knew to special
    /// -case for `LineXSpell_ID`/`StyleXSpell_ID`, just not yet applied here. Treating `SpellID`
    /// as if it were the thing `columns_excluding_pk` would exclude (it isn't - only `Spell_ID`
    /// is) silently left `SpellID` doubly listed in the INSERT's own column list, and never gave
    /// the real `Spell_ID` primary key a fresh value at all - this is what actually caused
    /// `UNIQUE constraint failed: Spell.SpellID` live, independent from (and still present after)
    /// the separate ID-allocation race this module's other fix already closed.
    guid_pk_column: Option<String>,
    /// The same source `SpellID` cloned twice for the same class (it appears on two different
    /// spec lines, or as both a cast spell and a style's proc) must reuse the one clone already
    /// made for this class in this call, not make a second independent copy of it.
    memo: HashMap<i64, i64>,
}

impl SpellCloner {
    pub fn new(cfg: &SshConfig, owning_class_id: i64) -> Result<Self, String> {
        ensure_tracking_table(cfg)?;
        let guid_pk_column = explicit_guid_pk_column(cfg, "Spell")?;
        let mut columns = columns_excluding_pk(cfg, "Spell")?;
        // SpellID is never just verbatim-copied - it always gets its own fresh value (see
        // clone_for_class), whether or not it also happens to be excluded already by virtue of
        // being flagged as the table's own PK (it isn't, on the real schema, but a future or
        // different fork's schema might shape this differently - stay correct either way).
        columns.retain(|c| c != "SpellID");
        Ok(Self {
            owning_class_id,
            columns,
            guid_pk_column,
            memo: HashMap::new(),
        })
    }

    /// Returns this call's owning class's own independent clone of `source_spell_id`, cloning it
    /// right now if it doesn't already have one (a no-op, returning the same ID as before, if
    /// this exact source was already cloned earlier in this same call). `source_spell_id <= 0`
    /// (no real spell - e.g. a pure melee style with no cast component) is returned unchanged;
    /// there is nothing to clone.
    ///
    /// A real, reproduced bug this used to have: the new `SpellID` was computed once (`MAX + 1`)
    /// when the caller's whole operation started, then just incremented in Rust for every
    /// further spell cloned in the same call, with every actual `INSERT` deferred into one big
    /// batch string executed only at the very end. That value is correct at the instant it's
    /// read, but goes stale the moment anything else writes to `Spell` before this call's own
    /// batch finally runs - a second click on the same button before the first finished (a real,
    /// reproduced case, since this can touch dozens of spells across many slow round-trips with
    /// no visual feedback that anything is happening), or a second class cloned/migrated moments
    /// later, would read the exact same "next free" value and collide on insert. Fixed by never
    /// guessing in Rust at all: the new ID is computed by a `(SELECT COALESCE(MAX(SpellID),0)+1
    /// FROM Spell)` subquery evaluated *inside* the same `INSERT`, the whole insert-and-readback
    /// wrapped in `BEGIN IMMEDIATE`/`COMMIT` so SQLite's own write-locking (not this tool's own
    /// bookkeeping) is what actually serializes two overlapping calls - whichever runs second
    /// simply computes its own, now-correct, distinct ID against the first's already-committed
    /// state, instead of racing it.
    pub fn clone_for_class(&mut self, cfg: &SshConfig, source_spell_id: i64) -> Result<i64, String> {
        if source_spell_id <= 0 {
            return Ok(source_spell_id);
        }
        if let Some(&existing) = self.memo.get(&source_spell_id) {
            return Ok(existing);
        }
        let exists = run_remote_sql_json(
            cfg,
            &format!("SELECT 1 FROM Spell WHERE SpellID = {source_spell_id};"),
        )?;
        if exists.is_empty() {
            return Err(format!(
                "source spell {source_spell_id} does not exist in Spell - cannot clone it \
                 (the referencing LineXSpell/StyleXSpell row may be stale)"
            ));
        }
        let col_list = self.columns.join(", ");
        let (pk_col, pk_val_expr) = match &self.guid_pk_column {
            Some(col) => (format!(", {col}"), format!(", '{}'", Uuid::new_v4())),
            None => (String::new(), String::new()),
        };
        let script = format!(
            "BEGIN IMMEDIATE;\n\
             INSERT INTO Spell ({col_list}, SpellID{pk_col}) \
             SELECT {col_list}, (SELECT COALESCE(MAX(SpellID), 0) + 1 FROM Spell){pk_val_expr} \
             FROM Spell WHERE SpellID = {source_spell_id};\n\
             INSERT INTO ClassEditorSpellClone (NewSpellID, SourceSpellID, OwningClassId) \
             SELECT MAX(SpellID), {source_spell_id}, {} FROM Spell;\n\
             SELECT MAX(SpellID) AS NewSpellID FROM Spell;\n\
             COMMIT;\n",
            self.owning_class_id
        );
        let rows = run_remote_sql_json(cfg, &script)?;
        let new_id = rows
            .first()
            .map(|r| int_field(r, "NewSpellID"))
            .ok_or_else(|| {
                format!("could not clone spell {source_spell_id} - no row came back after insert")
            })?;
        self.memo.insert(source_spell_id, new_id);
        Ok(new_id)
    }
}

/// Deletes every row in `Spell` this tool cloned specifically for one of `spell_ids` (silently
/// ignoring any ID in the list that isn't actually one of this tool's own tracked clones - a
/// shared/un-owned `SpellID`, still possibly in active use elsewhere, must never be touched
/// here). Safe to call with an empty slice, or in an environment where nothing was ever cloned
/// (the tracking table is created, found empty, nothing deleted).
pub fn delete_owned_spells(cfg: &SshConfig, spell_ids: &[i64]) -> Result<(), String> {
    let ids: Vec<i64> = spell_ids.iter().copied().filter(|id| *id > 0).collect();
    if ids.is_empty() {
        return Ok(());
    }
    ensure_tracking_table(cfg)?;
    let ids_csv = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let owned = run_remote_sql_json(
        cfg,
        &format!("SELECT NewSpellID FROM ClassEditorSpellClone WHERE NewSpellID IN ({ids_csv});"),
    )?;
    if owned.is_empty() {
        return Ok(());
    }
    let owned_csv = owned
        .iter()
        .map(|r| int_field(r, "NewSpellID").to_string())
        .collect::<Vec<_>>()
        .join(",");
    run_remote_sql(cfg, &format!("DELETE FROM Spell WHERE SpellID IN ({owned_csv});"))?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM ClassEditorSpellClone WHERE NewSpellID IN ({owned_csv});"),
    )?;
    Ok(())
}

/// Deletes every `StyleXSpell` row attached to any of `style_ids`, plus (via
/// `delete_owned_spells`) whatever independent proc `Spell` rows those procs owned - the `Style`
/// counterpart to `delete_owned_spells`, used wherever a class's own `Style` rows are about to be
/// deleted outright (never for a style still kept, e.g. a plain spell-line exclusion). Safe to
/// call with an empty slice.
pub fn delete_style_procs_for_styles(cfg: &SshConfig, style_ids: &[i64]) -> Result<(), String> {
    if style_ids.is_empty() {
        return Ok(());
    }
    let ids_csv = style_ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let procs = run_remote_sql_json(
        cfg,
        &format!("SELECT SpellID FROM StyleXSpell WHERE StyleID IN ({ids_csv});"),
    )?;
    let proc_spell_ids: Vec<i64> = procs.iter().map(|r| int_field(r, "SpellID")).collect();
    run_remote_sql(
        cfg,
        &format!("DELETE FROM StyleXSpell WHERE StyleID IN ({ids_csv});"),
    )?;
    delete_owned_spells(cfg, &proc_spell_ids)
}

/// Every `Spell` row this tool independently cloned for `class_id` - the editing UI's own "what
/// can I safely edit for this class" list. Returned as raw JSON rows (not a fixed Rust struct):
/// `Spell`'s real schema has dozens of fields (damage, range, duration, procs, message text, ...)
/// that genuinely vary between server forks, so the editing UI renders whatever columns actually
/// come back rather than this tool assuming a fixed shape.
pub fn list_owned_spells(cfg: &SshConfig, class_id: i64) -> Result<Vec<Value>, String> {
    ensure_tracking_table(cfg)?;
    run_remote_sql_json(
        cfg,
        &format!(
            "SELECT * FROM Spell WHERE SpellID IN (\
                 SELECT NewSpellID FROM ClassEditorSpellClone WHERE OwningClassId = {class_id}\
             ) ORDER BY SpellID;"
        ),
    )
}

/// Updates one or more fields on a spell this tool independently cloned - refuses outright if
/// `spell_id` isn't a tracked clone at all (the critical safety check this whole module exists
/// for: never let the editing UI reach a shared/un-owned `Spell` row, even by mistake), and
/// refuses any field name that isn't a real `Spell` column (checked fresh via `PRAGMA
/// table_info`, not a hardcoded allowlist - see `columns_excluding_pk`'s own doc comment) or is
/// the primary key itself.
pub fn update_owned_spell_fields(
    cfg: &SshConfig,
    spell_id: i64,
    fields: &HashMap<String, Value>,
) -> Result<String, String> {
    if fields.is_empty() {
        return Ok("nothing to update".to_string());
    }
    ensure_tracking_table(cfg)?;
    let owned = run_remote_sql_json(
        cfg,
        &format!("SELECT 1 FROM ClassEditorSpellClone WHERE NewSpellID = {spell_id};"),
    )?;
    if owned.is_empty() {
        return Err(format!(
            "spell {spell_id} is not an independently-owned clone - refusing to edit it, since it \
             may still be shared with the class it was cloned from (or any other class). Migrate \
             this class's spells to independent copies first."
        ));
    }

    let real_columns = columns_excluding_pk(cfg, "Spell")?;
    let mut set_clauses = Vec::new();
    for (field, value) in fields {
        if !real_columns.iter().any(|c| c == field) {
            return Err(format!(
                "'{field}' is not a real column on Spell - refusing to update it"
            ));
        }
        set_clauses.push(format!("{field} = {}", sql_literal(value)));
    }
    let sql = format!(
        "UPDATE Spell SET {} WHERE SpellID = {spell_id};",
        set_clauses.join(", ")
    );
    run_remote_sql(cfg, &sql)?;
    Ok(format!(
        "updated {} field(s) on spell {spell_id}",
        fields.len()
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
        let dir = std::env::temp_dir().join(format!("daoc-class-editor-spellown-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let db = dir.join("data/opendaoc.sqlite3.db");
        // Mirrors the *real* server schema's own shape, confirmed live: `Spell`'s actual SQL
        // primary key is `Spell_ID` (a separate GUID-style TEXT column, the exact same ORM
        // pattern as `LineXSpell_ID`/`StyleXSpell_ID`) - `SpellID` (the game-meaningful numeric
        // field everything else in this codebase actually cares about) is merely a *separately*
        // unique-indexed ordinary column, not the table's own primary key at all. A real,
        // reproduced bug found live: treating `SpellID` as if it were the thing to exclude as
        // "the primary key" (it isn't) duplicated it in a clone's own INSERT column list and
        // never gave the real `Spell_ID` a fresh value, causing `UNIQUE constraint failed:
        // Spell.SpellID` on every single clone attempt, deterministically - this exact fixture
        // shape (a flat `SpellID INTEGER PRIMARY KEY`) would never have caught that; the fixture
        // must mirror the real dual-identity shape, not simplify it away.
        let sql = "
            CREATE TABLE Spell (SpellID INTEGER NOT NULL DEFAULT 0, Name TEXT, Damage INTEGER DEFAULT 0, SpellGroup INTEGER DEFAULT 0, Spell_ID TEXT NOT NULL DEFAULT '', PRIMARY KEY (Spell_ID));
            CREATE UNIQUE INDEX U_Spell_SpellID ON Spell (SpellID);
            CREATE TABLE Style (StyleID INTEGER PRIMARY KEY AUTOINCREMENT, ClassId INTEGER);
            CREATE TABLE StyleXSpell (StyleXSpell_ID TEXT PRIMARY KEY DEFAULT '', SpellID INTEGER, ClassID INTEGER, StyleID INTEGER, Chance INTEGER);
            INSERT INTO Spell (SpellID, Name, Damage, SpellGroup, Spell_ID) VALUES (9001, 'Smite', 50, 500, 'seed-spell-9001');
            INSERT INTO Spell (SpellID, Name, Damage, SpellGroup, Spell_ID) VALUES (9002, 'Greater Smite', 80, 500, 'seed-spell-9002');
            INSERT INTO Style (StyleID, ClassId) VALUES (1, 64);
        ";
        let status = Command::new("sqlite3").arg(&db).arg(sql).status().unwrap();
        assert!(status.success());
        dir
    }

    #[test]
    fn clone_for_class_copies_every_other_column_and_assigns_a_new_id() {
        let dir = setup_test_db("clone-basic");
        let cfg = test_cfg(&dir);
        let mut cloner = SpellCloner::new(&cfg, 64).unwrap();

        let new_id = cloner.clone_for_class(&cfg, 9001).unwrap();
        assert_ne!(new_id, 9001, "must be a brand-new ID, not the source's own");

        let rows = run_remote_sql_json(
            &cfg,
            &format!("SELECT Name, Damage, SpellGroup FROM Spell WHERE SpellID = {new_id};"),
        )
        .unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(str_field(&rows[0], "Name"), "Smite");
        assert_eq!(int_field(&rows[0], "Damage"), 50);

        // The source spell must be completely untouched.
        let original = run_remote_sql_json(&cfg, "SELECT Name FROM Spell WHERE SpellID = 9001;")
            .unwrap();
        assert_eq!(str_field(&original[0], "Name"), "Smite");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_for_class_memoizes_the_same_source_within_one_call() {
        let dir = setup_test_db("clone-memo");
        let cfg = test_cfg(&dir);
        let mut cloner = SpellCloner::new(&cfg, 64).unwrap();

        let first = cloner.clone_for_class(&cfg, 9001).unwrap();
        let second = cloner.clone_for_class(&cfg, 9001).unwrap();
        assert_eq!(
            first, second,
            "the same source spell cloned twice for the same class must reuse one clone"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_for_class_is_safe_even_if_two_calls_race_to_allocate_at_once() {
        // The exact real bug this module's own doc comment describes: a "next free ID" computed
        // once and incremented in Rust across many slow round-trips would go stale and collide
        // the moment a second, concurrent clone (e.g. a double-clicked button) also wrote to
        // Spell in between. Two independent cloners, interleaved by hand here to simulate that
        // overlap, must each still get their own distinct, real ID - never collide.
        let dir = setup_test_db("clone-race");
        let cfg = test_cfg(&dir);
        let mut cloner_a = SpellCloner::new(&cfg, 64).unwrap();
        let mut cloner_b = SpellCloner::new(&cfg, 65).unwrap();

        let id_a = cloner_a.clone_for_class(&cfg, 9001).unwrap();
        let id_b = cloner_b.clone_for_class(&cfg, 9002).unwrap();
        assert_ne!(id_a, id_b, "two overlapping clones must never collide on the same new ID");

        let rows = run_remote_sql_json(&cfg, "SELECT SpellID FROM Spell;").unwrap();
        assert_eq!(
            rows.len(),
            4,
            "both new clones plus both originals must exist: {rows:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn editing_an_owned_clone_never_touches_the_shared_source() {
        let dir = setup_test_db("edit-safety");
        let cfg = test_cfg(&dir);
        let mut cloner = SpellCloner::new(&cfg, 64).unwrap();
        let new_id = cloner.clone_for_class(&cfg, 9001).unwrap();

        let mut fields = HashMap::new();
        fields.insert("Damage".to_string(), Value::from(999));
        update_owned_spell_fields(&cfg, new_id, &fields).unwrap();

        let edited = run_remote_sql_json(
            &cfg,
            &format!("SELECT Damage FROM Spell WHERE SpellID = {new_id};"),
        )
        .unwrap();
        assert_eq!(int_field(&edited[0], "Damage"), 999);

        let source = run_remote_sql_json(&cfg, "SELECT Damage FROM Spell WHERE SpellID = 9001;")
            .unwrap();
        assert_eq!(
            int_field(&source[0], "Damage"),
            50,
            "the source spell must be completely unaffected by editing its clone"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_refuses_to_edit_a_spell_that_was_never_cloned() {
        let dir = setup_test_db("edit-refuses-unowned");
        let cfg = test_cfg(&dir);
        ensure_tracking_table(&cfg).unwrap();

        let mut fields = HashMap::new();
        fields.insert("Damage".to_string(), Value::from(999));
        let result = update_owned_spell_fields(&cfg, 9001, &fields);
        assert!(
            result.is_err(),
            "editing a shared, never-cloned spell must be refused"
        );

        let untouched = run_remote_sql_json(&cfg, "SELECT Damage FROM Spell WHERE SpellID = 9001;")
            .unwrap();
        assert_eq!(int_field(&untouched[0], "Damage"), 50);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_refuses_an_unknown_field_name() {
        let dir = setup_test_db("edit-refuses-unknown-field");
        let cfg = test_cfg(&dir);
        let mut cloner = SpellCloner::new(&cfg, 64).unwrap();
        let new_id = cloner.clone_for_class(&cfg, 9001).unwrap();

        let mut fields = HashMap::new();
        fields.insert("NotARealColumn".to_string(), Value::from(1));
        let result = update_owned_spell_fields(&cfg, new_id, &fields);
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_owned_spells_removes_the_clone_but_never_a_shared_id() {
        let dir = setup_test_db("delete-safety");
        let cfg = test_cfg(&dir);
        let mut cloner = SpellCloner::new(&cfg, 64).unwrap();
        let owned_id = cloner.clone_for_class(&cfg, 9001).unwrap();

        // Ask it to delete both the real owned clone and a shared/un-owned ID (9002) in the same
        // call - only the owned one may actually go.
        delete_owned_spells(&cfg, &[owned_id, 9002]).unwrap();

        let owned_gone = run_remote_sql_json(
            &cfg,
            &format!("SELECT 1 FROM Spell WHERE SpellID = {owned_id};"),
        )
        .unwrap();
        assert!(owned_gone.is_empty(), "the owned clone must be deleted");

        let shared_still_there =
            run_remote_sql_json(&cfg, "SELECT 1 FROM Spell WHERE SpellID = 9002;").unwrap();
        assert!(
            !shared_still_there.is_empty(),
            "a shared/un-owned spell must never be deleted, even if asked to"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn style_x_spell_guid_column_discovers_the_real_text_primary_key() {
        let dir = setup_test_db("styleprocs-guid-pk");
        let cfg = test_cfg(&dir);
        let col = style_x_spell_guid_column(&cfg).unwrap();
        assert_eq!(col, Some("StyleXSpell_ID".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_style_procs_for_styles_removes_rows_and_owned_proc_spells() {
        let dir = setup_test_db("delete-style-procs");
        let cfg = test_cfg(&dir);
        let mut cloner = SpellCloner::new(&cfg, 64).unwrap();
        let proc_spell_id = cloner.clone_for_class(&cfg, 9002).unwrap();
        run_remote_sql(
            &cfg,
            &format!(
                "INSERT INTO StyleXSpell (StyleXSpell_ID, SpellID, ClassID, StyleID, Chance) \
                 VALUES ('proc-1', {proc_spell_id}, 64, 1, 50);"
            ),
        )
        .unwrap();

        delete_style_procs_for_styles(&cfg, &[1]).unwrap();

        let procs = run_remote_sql_json(&cfg, "SELECT 1 FROM StyleXSpell WHERE StyleID = 1;")
            .unwrap();
        assert!(procs.is_empty(), "the StyleXSpell row must be removed");
        let spell_gone = run_remote_sql_json(
            &cfg,
            &format!("SELECT 1 FROM Spell WHERE SpellID = {proc_spell_id};"),
        )
        .unwrap();
        assert!(spell_gone.is_empty(), "its owned proc spell must be removed too");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_owned_spells_only_returns_this_classs_own_clones() {
        let dir = setup_test_db("list-owned");
        let cfg = test_cfg(&dir);
        let mut cloner_a = SpellCloner::new(&cfg, 64).unwrap();
        cloner_a.clone_for_class(&cfg, 9001).unwrap();

        let mut cloner_b = SpellCloner::new(&cfg, 65).unwrap();
        cloner_b.clone_for_class(&cfg, 9002).unwrap();

        let owned_64 = list_owned_spells(&cfg, 64).unwrap();
        assert_eq!(owned_64.len(), 1, "{owned_64:?}");
        assert_eq!(str_field(&owned_64[0], "Name"), "Smite");

        let owned_65 = list_owned_spells(&cfg, 65).unwrap();
        assert_eq!(owned_65.len(), 1, "{owned_65:?}");
        assert_eq!(str_field(&owned_65[0], "Name"), "Greater Smite");

        std::fs::remove_dir_all(&dir).ok();
    }
}
