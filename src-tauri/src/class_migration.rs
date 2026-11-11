//! Migrates one class's locally-designed specs/spell-lines/spells/styles from a `design_session`
//! working copy onto the real live database, in one step - the counterpart `ClassSpecsSection.tsx`
//! needs that `ClonePanel.tsx` already got from `design_session.rs` (see that module's own doc
//! comment for the full rationale for designing against a disposable local snapshot at all).
//!
//! Unlike `ClonePanel.tsx`'s own deploy step - which can just re-invoke `apply_class_specs` a
//! second time against live, since that call is fully declarative (always specifies the *whole*
//! desired spec-assignment state, never an incremental delta) - `ClassSpecsSection.tsx`'s own
//! operations (`save_line_spell_fields`, `update_line_spell_level`, `save_style_fields`,
//! `add_new_spell_series_to_line`/`add_next_spell_in_series`, `add_style_to_spec`, ...) are all small, imperative edits to *specific
//! existing row identities* over however long a design session runs. There is no single "replay
//! the last call" primitive that could reproduce all of that - this module instead diffs the
//! session's own current state against live and reconciles it directly, one table at a time.
//!
//! The key simplifying fact this relies on: a design session's own database starts as a byte-for-
//! byte snapshot of live, so any row that already existed when the session started carries the
//! *identical* ID on both sides already - nothing to remap for it at all, whether or not its own
//! fields were edited in the meantime. Only a row created fresh *during* the session (a brand-new
//! spec/spell-line/spell/style, or a still-shared spec/style promoted to this class's own
//! independent copy for the first time mid-session) has an ID that only exists in the session's
//! own database - those need a fresh, live-side-collision-free ID allocated at migrate time (the
//! same atomic `(SELECT COALESCE(MAX(x),0)+1 FROM x)`-inside-the-INSERT pattern `SpellCloner`/
//! `clone_style_with_fresh_id` already use, for the same reason: two sessions each designing a
//! different class in parallel could otherwise race to allocate the exact same "next free" ID
//! against their own, now mutually stale, local snapshots), plus every other row in this same
//! migration that cross-references it (which spell a spell-line slot points at, which spell a
//! style's own proc casts, which other style's `ID` a chain style's own `OpeningRequirementValue`
//! resolves against) remapped to match.
//!
//! Sync order matters: each later step's own "does this already exist on live" check implicitly
//! depends on everything an earlier step may have just inserted (e.g. `LineXSpell` memberships
//! can only be correctly resolved once every `Spell` row they might reference is already on live),
//! so these always run in the same fixed order - Specialization/SpellLine, then Spell, then
//! Style, then style procs (`StyleXSpell`), then spell-line membership (`LineXSpell`), then the
//! class's own spec assignment (`ClassXSpecialization`) last of all, so a spec is never visible
//! as assigned on live before everything it actually grants is already there too.

use crate::deploy::{SshConfig, run_remote_sql, run_remote_sql_json};
use crate::spell_ownership::{
    columns_excluding_pk, delete_owned_spells, ensure_tracking_table, explicit_guid_pk_column, sql_literal, sql_quote,
    style_x_spell_guid_column,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

fn int_field(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn str_field(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// This class's own independently-owned spec keys currently assigned to it in `cfg` - the
/// `"{...}_<class_id>"` suffix convention `ensure_class_owns_spec`/`clone_spec` always name a
/// class's own clone with, confirmed live. Shared/stock spec keys the class merely has assigned
/// (e.g. a `NEVER_CLONE_SPECS` direct assignment like "Sword") are deliberately excluded here -
/// there is no `Specialization`/`SpellLine` row of its own to migrate for one of those at all
/// (its one real, shared row already exists identically on both live and session); only its own
/// class-locked `Style` rows (handled separately, by `ClassId` alone, in `sync_styles`) might.
fn owned_spec_keys(cfg: &SshConfig, class_id: i64) -> Result<Vec<String>, String> {
    let suffix = format!("_{class_id}");
    let rows = run_remote_sql_json(
        cfg,
        &format!("SELECT SpecKeyName FROM ClassXSpecialization WHERE ClassID = {class_id};"),
    )?;
    Ok(rows
        .iter()
        .map(|r| str_field(r, "SpecKeyName"))
        .filter(|k| k.ends_with(&suffix))
        .collect())
}

/// Inserts `row` into `table` on `cfg` completely verbatim, one column per key already present in
/// `row`'s own JSON object - used only for `Specialization`/`SpellLine`, whose own keys
/// (`KeyName`) are plain, already-unique-by-construction strings, never an autoincrement ID that
/// could collide with something already on live (see `unique_spec_key`'s own uniqueness
/// guarantee in specs.rs) - nothing here needs remapping at all.
fn insert_row_verbatim(cfg: &SshConfig, table: &str, row: &Value) -> Result<(), String> {
    let obj = row
        .as_object()
        .ok_or_else(|| format!("expected a JSON object row to insert into {table}"))?;
    let cols: Vec<&str> = obj.keys().map(String::as_str).collect();
    let vals: Vec<String> = obj.values().map(sql_literal).collect();
    run_remote_sql(
        cfg,
        &format!("INSERT INTO {table} ({}) VALUES ({});", cols.join(", "), vals.join(", ")),
    )
    .map(|_| ())
}

/// Updates every column `row` has (except `key_col` itself and anything in `excluded`) on
/// whichever `table` row already has `key_col = key_val` - the "sync an already-existing live row
/// to match whatever this session's own copy of it now looks like" half of this module's own
/// insert-or-update shape throughout. `excluded` is for a table's own separate GUID-style real
/// primary key (e.g. `Spell`'s own `Spell_ID`) that's expected to differ between live and session
/// by design (a fresh one is minted independently on each side the moment a row is newly
/// inserted) and must never be overwritten by whichever side happens to be the one calling this.
fn update_row_by_key(cfg: &SshConfig, table: &str, key_col: &str, key_val: &str, row: &Value, excluded: &[&str]) -> Result<(), String> {
    let obj = row
        .as_object()
        .ok_or_else(|| format!("expected a JSON object row to update {table} from"))?;
    let sets: Vec<String> = obj
        .iter()
        .filter(|(k, _)| k.as_str() != key_col && !excluded.contains(&k.as_str()))
        .map(|(k, v)| format!("{k} = {}", sql_literal(v)))
        .collect();
    if sets.is_empty() {
        return Ok(());
    }
    run_remote_sql(
        cfg,
        &format!(
            "UPDATE {table} SET {} WHERE {key_col} = '{}';",
            sets.join(", "),
            sql_quote(key_val)
        ),
    )
    .map(|_| ())
}

/// True if `a` and `b` have a different value for any column other than `key_col`/`excluded` -
/// used to decide whether an already-existing live row genuinely needs updating at all (and
/// whether to report it as "updated" in `migrate_class_design_to_live`'s own summary), not just
/// whether one happens to exist - see `update_row_by_key`'s own doc comment for why `excluded`
/// exists at all.
fn rows_differ(a: &Value, b: &Value, key_col: &str, excluded: &[&str]) -> bool {
    let (Some(a_obj), Some(b_obj)) = (a.as_object(), b.as_object()) else {
        return true;
    };
    a_obj.iter().any(|(k, av)| {
        k != key_col && !excluded.contains(&k.as_str()) && b_obj.get(k) != Some(av)
    })
}

/// Makes sure every one of this class's own owned specs (see `owned_spec_keys`) has a matching
/// `Specialization` row, and every `SpellLine` row under it, on live - inserted fresh (verbatim,
/// no remapping needed - see `insert_row_verbatim`'s own doc comment) if this spec was created or
/// newly promoted from a shared one entirely within this design session, or synced onto the
/// already-existing live row's own fields otherwise (there is no field-level editor for a spec's
/// own Specialization/SpellLine row in `ClassSpecsSection.tsx` today, but staying correct here
/// costs nothing and avoids this silently going stale the moment one is ever added).
fn sync_specializations_and_lines(session_cfg: &SshConfig, live_cfg: &SshConfig, class_id: i64) -> Result<usize, String> {
    let mut created = 0;
    for key in owned_spec_keys(session_cfg, class_id)? {
        let key_q = sql_quote(&key);
        let spec_rows = run_remote_sql_json(session_cfg, &format!("SELECT * FROM Specialization WHERE KeyName = '{key_q}';"))?;
        let Some(spec_row) = spec_rows.first() else { continue };
        let live_has_spec = !run_remote_sql_json(live_cfg, &format!("SELECT 1 FROM Specialization WHERE KeyName = '{key_q}';"))?.is_empty();
        if live_has_spec {
            update_row_by_key(live_cfg, "Specialization", "KeyName", &key, spec_row, &[])?;
        } else {
            insert_row_verbatim(live_cfg, "Specialization", spec_row)?;
            created += 1;
        }

        let line_rows = run_remote_sql_json(session_cfg, &format!("SELECT * FROM SpellLine WHERE Spec = '{key_q}';"))?;
        for line in &line_rows {
            let line_key = str_field(line, "KeyName");
            let line_key_q = sql_quote(&line_key);
            let live_has_line = !run_remote_sql_json(live_cfg, &format!("SELECT 1 FROM SpellLine WHERE KeyName = '{line_key_q}';"))?.is_empty();
            if live_has_line {
                update_row_by_key(live_cfg, "SpellLine", "KeyName", &line_key, line, &[])?;
            } else {
                insert_row_verbatim(live_cfg, "SpellLine", line)?;
            }
        }
    }
    Ok(created)
}

/// Allocates a fresh, live-side-collision-free `SpellID` (the same atomic `(SELECT
/// COALESCE(MAX(SpellID),0)+1 FROM Spell)`-inside-the-INSERT pattern `SpellCloner::clone_for_class`
/// already uses, for the identical reason: never guessed in Rust and incremented locally, so two
/// overlapping migrations can never race to the same "next free" value) and inserts `fields`
/// (read back from the session's own `Spell` row) under it, copying every other real column
/// verbatim. Returns the new live `SpellID`.
fn insert_spell_with_fresh_id(cfg: &SshConfig, fields: &Value) -> Result<i64, String> {
    let mut columns = columns_excluding_pk(cfg, "Spell")?;
    // Spell's real PK is a separate `Spell_ID` GUID column on the real schema (see
    // SpellCloner::new's own doc comment) - SpellID itself is "just" a separately unique-indexed
    // business field, not auto-excluded by columns_excluding_pk, so it's manually excluded here
    // too (it gets its own fresh value below, never copied from the session's own row).
    columns.retain(|c| c != "SpellID");
    let guid_pk_column = explicit_guid_pk_column(cfg, "Spell")?;

    let mut cols: Vec<String> = Vec::new();
    let mut vals: Vec<String> = Vec::new();
    for col in &columns {
        cols.push(col.clone());
        vals.push(sql_literal(fields.get(col).unwrap_or(&Value::Null)));
    }
    cols.push("SpellID".to_string());
    vals.push("(SELECT COALESCE(MAX(SpellID), 0) + 1 FROM Spell)".to_string());
    if let Some(col) = &guid_pk_column {
        cols.push(col.clone());
        vals.push(format!("'{}'", Uuid::new_v4()));
    }

    let script = format!(
        "BEGIN IMMEDIATE;\n\
         INSERT INTO Spell ({}) VALUES ({});\n\
         SELECT MAX(SpellID) AS NewSpellID FROM Spell;\n\
         COMMIT;\n",
        cols.join(", "),
        vals.join(", ")
    );
    let rows = run_remote_sql_json(cfg, &script)?;
    rows.first()
        .map(|r| int_field(r, "NewSpellID"))
        .ok_or_else(|| "could not insert the new spell on live - no row came back after insert".to_string())
}

/// Syncs every `Spell` row this class owns (per `ClassEditorSpellClone`, the same tracking table
/// every other independent spell clone in this tool already uses) from `session_cfg` onto
/// `live_cfg` - updating an already-existing live row's own fields to match (a field edit made
/// during the design session), or inserting a brand-new one with a fresh live `SpellID` (a spell
/// created entirely during the session, via `add_new_spell_series_to_line`/`add_next_spell_in_series`/`create_new_spell_spec`, or one
/// newly cloned the moment a still-shared line was first touched) otherwise, recording its own
/// `ClassEditorSpellClone` row on live too so it's immediately recognized as already-owned by any
/// later direct live edit. Returns every owned spell's own session-side `SpellID` mapped to
/// whatever `SpellID` it now has on live (the identical ID, for one that already existed there) -
/// every later step that references a spell by ID (`LineXSpell`/`StyleXSpell`) resolves its own
/// reference through this map.
fn sync_spells(session_cfg: &SshConfig, live_cfg: &SshConfig, class_id: i64) -> Result<(HashMap<i64, i64>, usize, usize), String> {
    ensure_tracking_table(session_cfg)?;
    ensure_tracking_table(live_cfg)?;
    let owned = run_remote_sql_json(
        session_cfg,
        &format!("SELECT NewSpellID, SourceSpellID FROM ClassEditorSpellClone WHERE OwningClassId = {class_id};"),
    )?;
    // Spell's own separate real GUID-style PK (see insert_spell_with_fresh_id's own doc comment)
    // is independently minted on each side the moment a row is first inserted there - it's
    // expected to differ between live and session even when every column that actually matters
    // is identical, and must never be treated as a real difference to sync or overwrite.
    let guid_pk_column = explicit_guid_pk_column(live_cfg, "Spell")?;
    let excluded: Vec<&str> = guid_pk_column.as_deref().into_iter().collect();

    let mut map = HashMap::new();
    let (mut created, mut updated) = (0, 0);
    for row in &owned {
        let session_spell_id = int_field(row, "NewSpellID");
        let source_spell_id = int_field(row, "SourceSpellID");
        let spell_rows = run_remote_sql_json(session_cfg, &format!("SELECT * FROM Spell WHERE SpellID = {session_spell_id};"))?;
        let Some(fields) = spell_rows.first() else { continue };

        let live_rows = run_remote_sql_json(live_cfg, &format!("SELECT * FROM Spell WHERE SpellID = {session_spell_id};"))?;
        if let Some(live_fields) = live_rows.first() {
            if rows_differ(fields, live_fields, "SpellID", &excluded) {
                update_row_by_key(live_cfg, "Spell", "SpellID", &session_spell_id.to_string(), fields, &excluded)?;
                updated += 1;
            }
            map.insert(session_spell_id, session_spell_id);
        } else {
            let new_id = insert_spell_with_fresh_id(live_cfg, fields)?;
            run_remote_sql(
                live_cfg,
                &format!(
                    "INSERT INTO ClassEditorSpellClone (NewSpellID, SourceSpellID, OwningClassId) \
                     VALUES ({new_id}, {source_spell_id}, {class_id});"
                ),
            )?;
            map.insert(session_spell_id, new_id);
            created += 1;
        }
    }
    Ok((map, created, updated))
}

/// `insert_spell_with_fresh_id`'s own `Style` counterpart - allocates a fresh, live-side-
/// collision-free `StyleID` (same atomic pattern as `clone_style_with_fresh_id`) and copies every
/// other column verbatim.
///
/// `Style.ID` (a separate business column from `StyleID` itself) needs special handling: a style
/// this tool created directly (`create_new_combat_spec`/`add_style_to_spec`/`clone_spec`) always
/// sets `ID = StyleID` at creation time, so it must get this row's own freshly allocated `StyleID`
/// too, not the stale session-side value. A style instead *granted* for a mechanic-locked spec
/// (`grant_styles_for_uncloneable_spec`) deliberately keeps the exact same `ID` every other
/// class's own locked copy of the same conceptual style already shares (its own doc comment: "the
/// real game's own chain-style `OpeningRequirementValue` references already stay correct across
/// every class that shares a spec, with no per-class remapping at all") - recognizable by its own
/// `ID` *not* equalling its own `StyleID` - and must be preserved exactly as-is, never remapped.
fn insert_style_with_fresh_id(cfg: &SshConfig, fields: &Value) -> Result<i64, String> {
    let session_style_id = int_field(fields, "StyleID");
    let preserve_id_as_is = int_field(fields, "ID") != session_style_id;

    let mut columns = columns_excluding_pk(cfg, "Style")?; // excludes StyleID itself already
    if !preserve_id_as_is {
        columns.retain(|c| c != "ID");
    }

    let mut cols: Vec<String> = Vec::new();
    let mut vals: Vec<String> = Vec::new();
    for col in &columns {
        cols.push(col.clone());
        vals.push(sql_literal(fields.get(col).unwrap_or(&Value::Null)));
    }
    let fresh_id_expr = "(SELECT COALESCE(MAX(StyleID), 0) + 1 FROM Style)";
    cols.push("StyleID".to_string());
    vals.push(fresh_id_expr.to_string());
    if !preserve_id_as_is {
        cols.push("ID".to_string());
        vals.push(fresh_id_expr.to_string());
    }

    let script = format!(
        "BEGIN IMMEDIATE;\n\
         INSERT INTO Style ({}) VALUES ({});\n\
         SELECT MAX(StyleID) AS NewStyleID FROM Style;\n\
         COMMIT;\n",
        cols.join(", "),
        vals.join(", ")
    );
    let rows = run_remote_sql_json(cfg, &script)?;
    rows.first()
        .map(|r| int_field(r, "NewStyleID"))
        .ok_or_else(|| "could not insert the new style on live - no row came back after insert".to_string())
}

/// `sync_spells`'s own `Style` counterpart - every `Style` row with `ClassId = class_id` is this
/// class's own (covers both this tool's own clones *and* a mechanic-locked spec's granted copies
/// alike, both locked the same way - see `ensure_style_owned`'s own resolution logic), synced the
/// same insert-or-update way, then (for brand-new styles only) a second pass remaps any chain
/// style's own `OpeningRequirementValue` that pointed at another style *also* newly created in
/// this same migration, from its own stale session-side `ID` to whatever `ID` it was actually
/// given on live (see `insert_style_with_fresh_id`'s own doc comment for why that's sometimes the
/// freshly allocated `StyleID` and sometimes a deliberately-preserved shared value). Guarded by
/// `AttackResultRequirement = 7` (the real "this style has a prerequisite" flag) so an ordinary,
/// non-chain style's own unrelated `OpeningRequirementValue` is never touched by coincidence.
fn sync_styles(session_cfg: &SshConfig, live_cfg: &SshConfig, class_id: i64) -> Result<(HashMap<i64, i64>, usize, usize), String> {
    let owned = run_remote_sql_json(session_cfg, &format!("SELECT * FROM Style WHERE ClassId = {class_id};"))?;

    let mut map = HashMap::new();
    let mut newly_created: Vec<Value> = Vec::new();
    let (mut created, mut updated) = (0, 0);
    for fields in &owned {
        let session_style_id = int_field(fields, "StyleID");
        let live_rows = run_remote_sql_json(live_cfg, &format!("SELECT * FROM Style WHERE StyleID = {session_style_id};"))?;
        if let Some(live_fields) = live_rows.first() {
            if rows_differ(fields, live_fields, "StyleID", &[]) {
                update_row_by_key(live_cfg, "Style", "StyleID", &session_style_id.to_string(), fields, &[])?;
                updated += 1;
            }
            map.insert(session_style_id, session_style_id);
        } else {
            let new_id = insert_style_with_fresh_id(live_cfg, fields)?;
            map.insert(session_style_id, new_id);
            newly_created.push(fields.clone());
            created += 1;
        }
    }

    for fields in &newly_created {
        if int_field(fields, "AttackResultRequirement") != 7 {
            continue;
        }
        let session_style_id = int_field(fields, "StyleID");
        let opening_value = int_field(fields, "OpeningRequirementValue");
        if let Some(&remapped_prereq_id) = map.get(&opening_value) {
            if remapped_prereq_id != opening_value {
                let new_live_id = map[&session_style_id];
                run_remote_sql(
                    live_cfg,
                    &format!("UPDATE Style SET OpeningRequirementValue = {remapped_prereq_id} WHERE StyleID = {new_live_id};"),
                )?;
            }
        }
    }
    Ok((map, created, updated))
}

/// Syncs every `StyleXSpell` proc row belonging to this class (`ClassID = class_id`) from session
/// onto live - inserted fresh (with its own `StyleID`/`SpellID` remapped through `style_id_map`/
/// `spell_id_map`, same as `sync_line_memberships` does for a `LineXSpell` row) if missing,
/// updating `Chance` on an already-existing live row if it changed (`update_style_proc_chance`,
/// the "Classes" tab's own secondary-effect editor, added after this function's own original
/// insert-only shape - see this function's own removal pass below for the matching, equally real
/// gap that left open at the same time: deleting one, `delete_style_proc`, never propagated
/// either), or removing a live row whose own GUID no longer exists in the session at all -
/// matched the same stable-key way `sync_line_memberships` already does for a `LineXSpell` row.
/// Falls back to matching by `(StyleID, SpellID, ClassID)` instead, and skips the removal pass
/// entirely, only on a fork whose `StyleXSpell` has no GUID-style primary key at all (see
/// `style_x_spell_guid_column`'s own doc comment) - there's no way to tell two otherwise-
/// identical-looking rows apart without one.
fn sync_style_procs(
    session_cfg: &SshConfig,
    live_cfg: &SshConfig,
    class_id: i64,
    spell_id_map: &HashMap<i64, i64>,
    style_id_map: &HashMap<i64, i64>,
) -> Result<(usize, usize, usize), String> {
    let guid_col = style_x_spell_guid_column(session_cfg)?;
    let rows = run_remote_sql_json(session_cfg, &format!("SELECT * FROM StyleXSpell WHERE ClassID = {class_id};"))?;
    let mut session_ids: HashSet<String> = HashSet::new();
    let (mut created, mut updated) = (0, 0);
    for row in &rows {
        let session_style_id = int_field(row, "StyleID");
        let session_spell_id = int_field(row, "SpellID");
        let live_style_id = style_id_map.get(&session_style_id).copied().unwrap_or(session_style_id);
        let live_spell_id = spell_id_map.get(&session_spell_id).copied().unwrap_or(session_spell_id);
        let chance = int_field(row, "Chance");

        let existing = if let Some(col) = &guid_col {
            let pk_val = str_field(row, col);
            session_ids.insert(pk_val.clone());
            run_remote_sql_json(live_cfg, &format!("SELECT Chance FROM StyleXSpell WHERE {col} = '{}';", sql_quote(&pk_val)))?
        } else {
            run_remote_sql_json(
                live_cfg,
                &format!("SELECT Chance FROM StyleXSpell WHERE StyleID = {live_style_id} AND SpellID = {live_spell_id} AND ClassID = {class_id};"),
            )?
        };
        if let Some(live_row) = existing.first() {
            if int_field(live_row, "Chance") != chance {
                if let Some(col) = &guid_col {
                    let pk_val = str_field(row, col);
                    run_remote_sql(
                        live_cfg,
                        &format!("UPDATE StyleXSpell SET Chance = {chance} WHERE {col} = '{}';", sql_quote(&pk_val)),
                    )?;
                } else {
                    run_remote_sql(
                        live_cfg,
                        &format!(
                            "UPDATE StyleXSpell SET Chance = {chance} WHERE StyleID = {live_style_id} AND SpellID = {live_spell_id} AND ClassID = {class_id};"
                        ),
                    )?;
                }
                updated += 1;
            }
            continue;
        }

        let obj = row.as_object().ok_or("expected a JSON object row for StyleXSpell")?;
        let mut cols: Vec<String> = Vec::new();
        let mut vals: Vec<String> = Vec::new();
        for (k, v) in obj {
            cols.push(k.clone());
            vals.push(match k.as_str() {
                "StyleID" => live_style_id.to_string(),
                "SpellID" => live_spell_id.to_string(),
                _ => sql_literal(v),
            });
        }
        run_remote_sql(live_cfg, &format!("INSERT INTO StyleXSpell ({}) VALUES ({});", cols.join(", "), vals.join(", ")))?;
        created += 1;
    }

    let mut removed = 0;
    if let Some(col) = &guid_col {
        let live_rows = run_remote_sql_json(
            live_cfg,
            &format!("SELECT {col} AS ProcId, SpellID FROM StyleXSpell WHERE ClassID = {class_id};"),
        )?;
        for row in &live_rows {
            let proc_id = str_field(row, "ProcId");
            if session_ids.contains(&proc_id) {
                continue;
            }
            let spell_id = int_field(row, "SpellID");
            run_remote_sql(live_cfg, &format!("DELETE FROM StyleXSpell WHERE {col} = '{}';", sql_quote(&proc_id)))?;
            delete_owned_spells(live_cfg, &[spell_id])?;
            removed += 1;
        }
    }

    Ok((created, updated, removed))
}

/// Syncs every `LineXSpell` row under this class's own owned spell line(s) (see
/// `owned_spec_keys`) from session onto live, matched by `LineXSpell_ID` (a stable GUID, already
/// identical on both sides for a row that existed before the session started - see this module's
/// own doc comment) - updating `SpellID` (remapped through `spell_id_map`)/`Level` on an already-
/// existing live row if either changed (a level edit, or a spell swapped in via
/// `ensure_line_owned`'s own first-touch cloning), or inserting a brand-new one (a new spell added
/// via `add_new_spell_series_to_line`/`add_next_spell_in_series`, or an entire line newly cloned
/// this session) otherwise.
///
/// Also removes any live `LineXSpell` row under these same owned lines whose own `LineXSpell_ID`
/// no longer exists in the session at all - a real, reported bug: `delete_spell_from_line` (the
/// "Delete" button on an individual spell) only ever touched the session's own database, and
/// every sync above is pure insert-or-update, so a spell deleted mid-session kept being granted
/// to (and working for) already-logged-in characters forever, no matter how many times the class
/// was re-migrated afterward. `LineXSpell_ID` is the same stable key the insert-or-update pass
/// above already keys on, so "deleted mid-session" is simply "this class's own owned line still
/// exists, but this specific slot's own `LineXSpell_ID` doesn't appear in it anymore" - the
/// underlying `Spell` row it pointed to on live is removed too via `delete_owned_spells` (a
/// no-op, left untouched, if it was never actually one of this tool's own independent clones -
/// the exact same safety net `delete_spell_from_line` itself already relies on).
fn sync_line_memberships(
    session_cfg: &SshConfig,
    live_cfg: &SshConfig,
    class_id: i64,
    spell_id_map: &HashMap<i64, i64>,
) -> Result<(usize, usize, usize), String> {
    let owned_keys = owned_spec_keys(session_cfg, class_id)?;
    if owned_keys.is_empty() {
        return Ok((0, 0, 0));
    }
    let keys_csv = owned_keys.iter().map(|k| format!("'{}'", sql_quote(k))).collect::<Vec<_>>().join(", ");
    let line_rows = run_remote_sql_json(session_cfg, &format!("SELECT KeyName FROM SpellLine WHERE Spec IN ({keys_csv});"))?;
    let line_keys: Vec<String> = line_rows.iter().map(|r| str_field(r, "KeyName")).collect();
    if line_keys.is_empty() {
        return Ok((0, 0, 0));
    }
    let line_keys_csv = line_keys.iter().map(|k| format!("'{}'", sql_quote(k))).collect::<Vec<_>>().join(", ");
    let rows = run_remote_sql_json(session_cfg, &format!("SELECT * FROM LineXSpell WHERE LineName IN ({line_keys_csv});"))?;
    let session_ids: HashSet<String> = rows.iter().map(|r| str_field(r, "LineXSpell_ID")).collect();

    let (mut created, mut updated) = (0, 0);
    for row in &rows {
        let line_xspell_id = str_field(row, "LineXSpell_ID");
        let id_q = sql_quote(&line_xspell_id);
        let session_spell_id = int_field(row, "SpellID");
        let live_spell_id = spell_id_map.get(&session_spell_id).copied().unwrap_or(session_spell_id);
        let level = int_field(row, "Level");

        let existing = run_remote_sql_json(live_cfg, &format!("SELECT SpellID, Level FROM LineXSpell WHERE LineXSpell_ID = '{id_q}';"))?;
        if let Some(live_row) = existing.first() {
            if int_field(live_row, "SpellID") != live_spell_id || int_field(live_row, "Level") != level {
                run_remote_sql(
                    live_cfg,
                    &format!("UPDATE LineXSpell SET SpellID = {live_spell_id}, Level = {level} WHERE LineXSpell_ID = '{id_q}';"),
                )?;
                updated += 1;
            }
        } else {
            let line_name_q = sql_quote(&str_field(row, "LineName"));
            run_remote_sql(
                live_cfg,
                &format!(
                    "INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) \
                     VALUES ('{line_name_q}', {live_spell_id}, {level}, '{id_q}');"
                ),
            )?;
            created += 1;
        }
    }

    let live_rows = run_remote_sql_json(live_cfg, &format!("SELECT LineXSpell_ID, SpellID FROM LineXSpell WHERE LineName IN ({line_keys_csv});"))?;
    let mut removed = 0;
    for row in &live_rows {
        let line_xspell_id = str_field(row, "LineXSpell_ID");
        if session_ids.contains(&line_xspell_id) {
            continue;
        }
        let id_q = sql_quote(&line_xspell_id);
        let spell_id = int_field(row, "SpellID");
        run_remote_sql(live_cfg, &format!("DELETE FROM LineXSpell WHERE LineXSpell_ID = '{id_q}';"))?;
        delete_owned_spells(live_cfg, &[spell_id])?;
        removed += 1;
    }

    Ok((created, updated, removed))
}

/// Syncs this class's own full `ClassXSpecialization` assignment list from session onto live -
/// the one step that must run last of all (see this module's own doc comment), so a spec is
/// never visible as assigned before everything it actually grants already is. A full declarative
/// replace (delete this class's whole assignment list on live, then reinsert session's own
/// current one verbatim), the same "always resubmit the whole thing, never try to diff it
/// row-by-row" convention `apply_class_specs` already uses, for the identical reason: a still-
/// shared spec promoted to this class's own independent clone mid-session (`ensure_class_owns_spec`)
/// doesn't add a row, it *renames* the class's existing assignment from the old shared key to the
/// new owned one in place - a plain "insert whatever's missing" pass would leave the stale old
/// key assigned on live *alongside* the new one, assigning the same conceptual spec twice over.
/// A no-op (and reported as such) if session's own assignment list already matches live's
/// exactly.
fn sync_class_spec_assignment(session_cfg: &SshConfig, live_cfg: &SshConfig, class_id: i64) -> Result<bool, String> {
    let rows = run_remote_sql_json(session_cfg, &format!("SELECT SpecKeyName, LevelAcquired FROM ClassXSpecialization WHERE ClassID = {class_id};"))?;
    let live_rows = run_remote_sql_json(live_cfg, &format!("SELECT SpecKeyName, LevelAcquired FROM ClassXSpecialization WHERE ClassID = {class_id};"))?;

    let as_sorted_pairs = |rs: &[Value]| -> Vec<(String, i64)> {
        let mut pairs: Vec<(String, i64)> = rs.iter().map(|r| (str_field(r, "SpecKeyName"), int_field(r, "LevelAcquired"))).collect();
        pairs.sort();
        pairs
    };
    if as_sorted_pairs(&rows) == as_sorted_pairs(&live_rows) {
        return Ok(false);
    }

    run_remote_sql(live_cfg, &format!("DELETE FROM ClassXSpecialization WHERE ClassID = {class_id};"))?;
    for row in &rows {
        let key_q = sql_quote(&str_field(row, "SpecKeyName"));
        let level = int_field(row, "LevelAcquired");
        run_remote_sql(
            live_cfg,
            &format!("INSERT INTO ClassXSpecialization (ClassID, SpecKeyName, LevelAcquired) VALUES ({class_id}, '{key_q}', {level});"),
        )?;
    }
    Ok(true)
}

/// Migrates everything `class_id` owns (specs, spell lines, spells, styles, procs, and its own
/// spec assignment list) from `session_cfg`'s own local design-session database onto
/// `live_cfg` - the one step a design session's own disposable scratch copy was always meant to
/// lead up to. Callers must call `deploy::ensure_safe_to_write(live_cfg)` first (this function
/// itself deliberately doesn't - see every other write path in this tool for the same
/// "server stopped + a sufficiently fresh backup" convention, enforced once at the Tauri command
/// boundary, never duplicated inside the function it gates).
///
/// Returns a short human-readable summary of what actually changed - "nothing to migrate" if this
/// class's own live state already matches its session exactly (safe to call again after an
/// earlier migration with no further edits in between; every step here is itself idempotent).
pub fn migrate_class_design_to_live(session_cfg: &SshConfig, live_cfg: &SshConfig, class_id: i64) -> Result<String, String> {
    let specs_created = sync_specializations_and_lines(session_cfg, live_cfg, class_id)?;
    let (spell_id_map, spells_created, spells_updated) = sync_spells(session_cfg, live_cfg, class_id)?;
    let (style_id_map, styles_created, styles_updated) = sync_styles(session_cfg, live_cfg, class_id)?;
    let (procs_created, procs_updated, procs_removed) =
        sync_style_procs(session_cfg, live_cfg, class_id, &spell_id_map, &style_id_map)?;
    let (lines_created, lines_updated, lines_removed) = sync_line_memberships(session_cfg, live_cfg, class_id, &spell_id_map)?;
    let assignment_synced = sync_class_spec_assignment(session_cfg, live_cfg, class_id)?;

    let mut summary = Vec::new();
    if specs_created > 0 {
        summary.push(format!("{specs_created} new spec(s)/spell line(s)"));
    }
    if spells_created > 0 || spells_updated > 0 {
        summary.push(format!("{spells_created} new spell(s), {spells_updated} updated"));
    }
    if styles_created > 0 || styles_updated > 0 {
        summary.push(format!("{styles_created} new style(s), {styles_updated} updated"));
    }
    if procs_created > 0 || procs_updated > 0 || procs_removed > 0 {
        summary.push(format!(
            "{procs_created} new style effect(s), {procs_updated} updated, {procs_removed} deleted"
        ));
    }
    if lines_created > 0 || lines_updated > 0 || lines_removed > 0 {
        summary.push(format!(
            "{lines_created} new spell-line entries, {lines_updated} updated, {lines_removed} deleted"
        ));
    }
    if assignment_synced {
        summary.push("spec assignment synced".to_string());
    }
    if summary.is_empty() {
        Ok("nothing to migrate - live already matches this design session".to_string())
    } else {
        Ok(format!("migrated to live: {}", summary.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::specs::{
        add_new_spell_series_to_line, add_style_proc, add_style_to_spec, create_new_combat_spec, create_new_spell_spec,
        delete_spell_from_line, delete_style_proc, update_style_proc_chance,
    };
    use std::process::Command;

    fn test_cfg(dir: &std::path::Path) -> SshConfig {
        SshConfig {
            host: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            remote_server_dir: dir.to_string_lossy().to_string(),
            start_script_name: String::new(),
        }
    }

    /// Both DBs start out byte-for-byte identical (a real design session's own snapshot
    /// guarantee - see this module's own doc comment) - one `name` seeds both a "-live" and a
    /// "-session" directory from the exact same schema/seed SQL.
    fn setup_pair(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let sql = "
            CREATE TABLE Specialization (KeyName TEXT, Name TEXT, Description TEXT, Icon INTEGER, Implementation TEXT, ClientStyleGroup INTEGER DEFAULT 0);
            CREATE TABLE ClassXSpecialization (ClassID INTEGER, SpecKeyName TEXT, LevelAcquired INTEGER);
            CREATE TABLE SpellLine (KeyName TEXT, Name TEXT, Spec TEXT, IsBaseLine INTEGER, ClassIDHint INTEGER);
            CREATE TABLE LineXSpell (LineName TEXT, SpellID INTEGER, Level INTEGER, LineXSpell_ID TEXT PRIMARY KEY DEFAULT '');
            CREATE TABLE Style (StyleID INTEGER PRIMARY KEY AUTOINCREMENT, ClassId INTEGER, Name TEXT, SpecKeyName TEXT,
                SpecLevelRequirement INTEGER, Icon INTEGER, EnduranceCost INTEGER, StealthRequirement INTEGER,
                OpeningRequirementType INTEGER, OpeningRequirementValue INTEGER, AttackResultRequirement INTEGER,
                WeaponTypeRequirement INTEGER, GrowthOffset REAL, GrowthRate REAL, BonusToHit INTEGER,
                BonusToDefense INTEGER, TwoHandAnimation INTEGER, RandomProc INTEGER, ArmorHitLocation INTEGER,
                ID INTEGER DEFAULT 0);
            CREATE TABLE SpecXAbility (SpecXabilityID INTEGER PRIMARY KEY AUTOINCREMENT, Spec TEXT, SpecLevel INTEGER,
                AbilityKey TEXT, AbilityLevel INTEGER, ClassId INTEGER);
            CREATE TABLE DOLCharacters (Name TEXT, Class INTEGER, RespecAmountAllSkill INTEGER DEFAULT 0, SerializedSpecs TEXT DEFAULT '');
            CREATE TABLE Spell (SpellID INTEGER NOT NULL DEFAULT 0, Name TEXT, SpellGroup INTEGER DEFAULT 0, Spell_ID TEXT NOT NULL DEFAULT '', PRIMARY KEY (Spell_ID));
            CREATE UNIQUE INDEX U_Spell_SpellID ON Spell (SpellID);
            CREATE TABLE StyleXSpell (StyleXSpell_ID TEXT PRIMARY KEY DEFAULT '', SpellID INTEGER, ClassID INTEGER, StyleID INTEGER, Chance INTEGER);

            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) VALUES ('slash', 'Slash', 'Slashing weapons', 100, NULL, 0);
            INSERT INTO ClassXSpecialization VALUES (1, 'slash', 0);
            INSERT INTO Style VALUES (NULL, 1, 'Pulverize', 'slash', 5, 300, 5, 0, 1, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);
            INSERT INTO Spell (SpellID, Name, SpellGroup, Spell_ID) VALUES (9001, 'Smite', 500, 'seed-spell-9001');
        ";
        let mut dirs = Vec::new();
        for suffix in ["live", "session"] {
            let dir = std::env::temp_dir().join(format!("daoc-class-editor-migrate-test-{name}-{suffix}"));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(dir.join("data")).unwrap();
            let db = dir.join("data/opendaoc.sqlite3.db");
            let status = Command::new("sqlite3").arg(&db).arg(sql).status().expect("sqlite3 must be on PATH for this test");
            assert!(status.success());
            dirs.push(dir);
        }
        (dirs[0].clone(), dirs[1].clone())
    }

    #[test]
    fn migrates_a_brand_new_spec_spell_and_style_created_entirely_in_session() {
        let (live_dir, session_dir) = setup_pair("brand-new");
        let live = test_cfg(&live_dir);
        let session = test_cfg(&session_dir);

        // Everything below happens only against the session - live never sees any of it until
        // migrate_class_design_to_live runs.
        create_new_spell_spec(&session, 2, "Earthshatter", 5).unwrap();
        add_new_spell_series_to_line(&session, 2, "earthshatter_2", "earthshatter_2_line", Some(2)).unwrap();
        create_new_combat_spec(&session, 2, "Ironwill", 1).unwrap();
        add_style_to_spec(&session, 2, "ironwill_2", None).unwrap();

        let summary = migrate_class_design_to_live(&session, &live, 2).unwrap();
        assert!(summary.contains("migrated to live"), "{summary}");

        let live_spec = run_remote_sql_json(&live, "SELECT Name FROM Specialization WHERE KeyName = 'earthshatter_2';").unwrap();
        assert_eq!(str_field(&live_spec[0], "Name"), "Earthshatter");

        let live_spells = run_remote_sql_json(
            &live,
            "SELECT ls.Level, s.Name FROM LineXSpell ls JOIN Spell s ON s.SpellID = ls.SpellID \
             WHERE ls.LineName = 'earthshatter_2_line' ORDER BY ls.Level;",
        )
        .unwrap();
        assert_eq!(live_spells.len(), 2, "{live_spells:?}");
        assert_eq!(int_field(&live_spells[0], "Level"), 1);
        assert_eq!(int_field(&live_spells[1], "Level"), 2);

        let live_styles = run_remote_sql_json(&live, "SELECT Name, ClassId FROM Style WHERE SpecKeyName = 'ironwill_2';").unwrap();
        assert_eq!(live_styles.len(), 2, "{live_styles:?}");

        let live_assignments = run_remote_sql_json(
            &live,
            "SELECT SpecKeyName FROM ClassXSpecialization WHERE ClassID = 2 ORDER BY SpecKeyName;",
        )
        .unwrap();
        assert_eq!(live_assignments.len(), 2, "{live_assignments:?}");

        // The newly migrated spell must be tracked on *live*, not just in the session, so a
        // later direct live edit recognizes it as already owned.
        let new_live_spell_id = int_field(&live_spells[0], "Level"); // placeholder, re-fetch below
        let _ = new_live_spell_id;
        let tracked = run_remote_sql_json(&live, "SELECT COUNT(*) AS c FROM ClassEditorSpellClone WHERE OwningClassId = 2;").unwrap();
        assert_eq!(int_field(&tracked[0], "c"), 2, "one per spell created in-session");

        std::fs::remove_dir_all(&live_dir).ok();
        std::fs::remove_dir_all(&session_dir).ok();
    }

    #[test]
    fn migrate_is_idempotent_when_run_twice_with_no_further_changes() {
        let (live_dir, session_dir) = setup_pair("idempotent");
        let live = test_cfg(&live_dir);
        let session = test_cfg(&session_dir);

        create_new_spell_spec(&session, 2, "Earthshatter", 5).unwrap();
        migrate_class_design_to_live(&session, &live, 2).unwrap();
        let second = migrate_class_design_to_live(&session, &live, 2).unwrap();
        assert_eq!(second, "nothing to migrate - live already matches this design session");

        std::fs::remove_dir_all(&live_dir).ok();
        std::fs::remove_dir_all(&session_dir).ok();
    }

    #[test]
    fn migrate_removes_a_spell_deleted_from_a_line_after_an_earlier_migration() {
        // A real, reported bug: deleting a spell mid-session (delete_spell_from_line) only ever
        // touched the session's own database - every sync in this module was pure insert-or-
        // update, so re-migrating afterward left the deleted spell still fully granted (and still
        // working) for every character already on the class, no matter how many times it was
        // re-migrated.
        let (live_dir, session_dir) = setup_pair("delete-then-remigrate");
        let live = test_cfg(&live_dir);
        let session = test_cfg(&session_dir);

        create_new_spell_spec(&session, 2, "Earthshatter", 5).unwrap();
        let second_spell = add_new_spell_series_to_line(&session, 2, "earthshatter_2", "earthshatter_2_line", Some(10)).unwrap();
        migrate_class_design_to_live(&session, &live, 2).unwrap();

        let live_before = run_remote_sql_json(&live, "SELECT 1 FROM LineXSpell WHERE LineName = 'earthshatter_2_line';").unwrap();
        assert_eq!(live_before.len(), 2, "both spells must be live after the first migration: {live_before:?}");

        // Only the session's own copy is touched from here on - delete the second spell, then
        // re-migrate, exactly like the owner's own reported "delete, then re-migrate" workflow.
        delete_spell_from_line(&session, 2, &second_spell.line_xspell_id).unwrap();
        let summary = migrate_class_design_to_live(&session, &live, 2).unwrap();
        assert!(summary.contains("1 deleted"), "{summary}");

        let live_after = run_remote_sql_json(&live, "SELECT Level FROM LineXSpell WHERE LineName = 'earthshatter_2_line';").unwrap();
        assert_eq!(live_after.len(), 1, "the deleted spell's own membership row must be gone on live too: {live_after:?}");
        assert_eq!(int_field(&live_after[0], "Level"), 1, "only the first (still-owned) spell must remain");

        let live_spell_gone = run_remote_sql_json(&live, &format!("SELECT 1 FROM Spell WHERE SpellID = {};", second_spell.spell_id)).unwrap();
        assert!(live_spell_gone.is_empty(), "the deleted spell's own underlying Spell row must be gone on live too");

        std::fs::remove_dir_all(&live_dir).ok();
        std::fs::remove_dir_all(&session_dir).ok();
    }

    #[test]
    fn migrate_syncs_a_style_effects_own_chance_change_and_removal_after_an_earlier_migration() {
        // The exact same class of bug the spell-deletion test above covers, just for a style's
        // own secondary effect (StyleXSpell) instead of a spell-line entry: add_style_proc/
        // update_style_proc_chance/delete_style_proc only ever touch the session's own database,
        // so a Chance edit or an outright removal made mid-session must still correctly reach
        // live once (re-)migrated, not just a brand-new effect's own first creation.
        let (live_dir, session_dir) = setup_pair("style-proc-sync");
        let live = test_cfg(&live_dir);
        let session = test_cfg(&session_dir);

        create_new_combat_spec(&session, 2, "Ironwill", 1).unwrap();
        let style_id = int_field(
            &run_remote_sql_json(&session, "SELECT StyleID FROM Style WHERE SpecKeyName = 'ironwill_2';").unwrap()[0],
            "StyleID",
        );
        let kept = add_style_proc(&session, 2, "ironwill_2", style_id).unwrap();
        let removed = add_style_proc(&session, 2, "ironwill_2", style_id).unwrap();
        migrate_class_design_to_live(&session, &live, 2).unwrap();

        let live_before = run_remote_sql_json(&live, "SELECT Chance FROM StyleXSpell WHERE ClassID = 2;").unwrap();
        assert_eq!(live_before.len(), 2, "both effects must be live after the first migration: {live_before:?}");

        // Only the session's own copy is touched from here on.
        update_style_proc_chance(&session, 2, &kept.style_x_spell_id, 42).unwrap();
        delete_style_proc(&session, 2, &removed.style_x_spell_id).unwrap();
        let summary = migrate_class_design_to_live(&session, &live, 2).unwrap();
        assert!(summary.contains("1 updated") && summary.contains("1 deleted"), "{summary}");

        let live_after = run_remote_sql_json(&live, "SELECT Chance FROM StyleXSpell WHERE ClassID = 2;").unwrap();
        assert_eq!(live_after.len(), 1, "the deleted effect's own row must be gone on live too: {live_after:?}");
        assert_eq!(int_field(&live_after[0], "Chance"), 42, "the kept effect's own Chance edit must have reached live too");

        let live_spell_gone =
            run_remote_sql_json(&live, &format!("SELECT 1 FROM Spell WHERE SpellID = {};", removed.spell_id)).unwrap();
        assert!(live_spell_gone.is_empty(), "the deleted effect's own underlying Spell row must be gone on live too");

        std::fs::remove_dir_all(&live_dir).ok();
        std::fs::remove_dir_all(&session_dir).ok();
    }

    #[test]
    fn migrate_clones_a_still_shared_spec_promoted_only_during_the_session() {
        // The exact scenario this whole feature exists for: class 1 has "slash" assigned under
        // its plain, shared KeyName on *both* sides to start with (never cloned through this
        // tool at all) - touching one of its styles (add_style_to_spec, triggering
        // ensure_class_owns_spec's own first-touch promotion) only in the session must migrate
        // onto live as a brand-new "slash_1" independent clone, leaving the real, original
        // "slash" spec on live completely untouched - not silently edited in place.
        let (live_dir, session_dir) = setup_pair("promote-shared");
        let live = test_cfg(&live_dir);
        let session = test_cfg(&session_dir);

        add_style_to_spec(&session, 1, "slash", None).unwrap();

        migrate_class_design_to_live(&session, &live, 1).unwrap();

        let live_original = run_remote_sql_json(&live, "SELECT ClassId FROM Style WHERE SpecKeyName = 'slash';").unwrap();
        assert_eq!(live_original.len(), 1, "the original shared spec's own style must be untouched, not cloned away");

        let live_owned_spec = run_remote_sql_json(&live, "SELECT Name FROM Specialization WHERE KeyName = 'slash_1';").unwrap();
        assert_eq!(str_field(&live_owned_spec[0], "Name"), "Slash");

        let live_owned_styles = run_remote_sql_json(&live, "SELECT Name, ClassId FROM Style WHERE SpecKeyName = 'slash_1' ORDER BY Name;").unwrap();
        assert_eq!(live_owned_styles.len(), 2, "{live_owned_styles:?}", );

        let live_assignment = run_remote_sql_json(&live, "SELECT SpecKeyName FROM ClassXSpecialization WHERE ClassID = 1;").unwrap();
        assert_eq!(str_field(&live_assignment[0], "SpecKeyName"), "slash_1", "class 1 must now be assigned its own clone, not the shared original");

        std::fs::remove_dir_all(&live_dir).ok();
        std::fs::remove_dir_all(&session_dir).ok();
    }

    #[test]
    fn migrate_syncs_a_field_edit_made_in_session_onto_an_already_owned_live_style() {
        let (live_dir, session_dir) = setup_pair("field-edit");
        let live = test_cfg(&live_dir);
        let session = test_cfg(&session_dir);

        // Promote "slash" to class 1's own clone identically on *both* sides first (simulating a
        // spec that was already independently owned before this design session ever started) by
        // creating the exact same style row (StyleID 1, already seeded by setup_pair) directly on
        // each DB - standing in for what ensure_class_owns_spec would already have produced.
        for cfg in [&live, &session] {
            run_remote_sql(
                cfg,
                "UPDATE ClassXSpecialization SET SpecKeyName = 'slash_1' WHERE ClassID = 1;\n\
                 INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                 VALUES ('slash_1', 'Slash', 'Slashing weapons', 100, NULL, 0);\n\
                 UPDATE Style SET SpecKeyName = 'slash_1' WHERE StyleID = 1;",
            )
            .unwrap();
        }

        // Only the session's own copy is edited from here on - live must still show the old name
        // until migrate runs.
        run_remote_sql(&session, "UPDATE Style SET Name = 'Greater Pulverize' WHERE StyleID = 1;").unwrap();

        migrate_class_design_to_live(&session, &live, 1).unwrap();

        let live_style = run_remote_sql_json(&live, "SELECT Name FROM Style WHERE StyleID = 1;").unwrap();
        assert_eq!(str_field(&live_style[0], "Name"), "Greater Pulverize");

        std::fs::remove_dir_all(&live_dir).ok();
        std::fs::remove_dir_all(&session_dir).ok();
    }

    #[test]
    fn migrate_remaps_a_chain_styles_prerequisite_created_in_the_same_session() {
        let (live_dir, session_dir) = setup_pair("chain-style");
        let live = test_cfg(&live_dir);
        let session = test_cfg(&session_dir);

        create_new_combat_spec(&session, 2, "Ironwill", 1).unwrap();
        // The spec's own first seeded style is never a chain style (see
        // create_new_combat_spec's own doc comment) - add a second one and hand-set it up as a
        // chain style requiring the first.
        let first = run_remote_sql_json(&session, "SELECT StyleID FROM Style WHERE SpecKeyName = 'ironwill_2';").unwrap();
        let first_id = int_field(&first[0], "StyleID");
        let second = add_style_to_spec(&session, 2, "ironwill_2", None).unwrap();
        run_remote_sql(
            &session,
            &format!(
                "UPDATE Style SET AttackResultRequirement = 7, OpeningRequirementValue = {first_id} WHERE StyleID = {};",
                second.style_id
            ),
        )
        .unwrap();

        migrate_class_design_to_live(&session, &live, 2).unwrap();

        let live_styles = run_remote_sql_json(&live, "SELECT StyleID, ID, OpeningRequirementValue, AttackResultRequirement FROM Style WHERE SpecKeyName = 'ironwill_2' ORDER BY StyleID;").unwrap();
        assert_eq!(live_styles.len(), 2, "{live_styles:?}");
        let chain_style = live_styles.iter().find(|s| int_field(s, "AttackResultRequirement") == 7).unwrap();
        let prereq_style = live_styles.iter().find(|s| int_field(s, "AttackResultRequirement") != 7).unwrap();
        assert_eq!(
            int_field(chain_style, "OpeningRequirementValue"),
            int_field(prereq_style, "ID"),
            "the chain style's prerequisite must resolve to its own prerequisite's real live ID, \
             not whatever stale session-only StyleID it pointed at before migrating"
        );

        std::fs::remove_dir_all(&live_dir).ok();
        std::fs::remove_dir_all(&session_dir).ok();
    }
}
