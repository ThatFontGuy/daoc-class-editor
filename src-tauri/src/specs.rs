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
//! SpellLine(s), their LineXSpell entries, its Style (melee skill) rows, and its SpecXAbility
//! (level-granted abilities, e.g. Stealth's "Safe Fall"/"Detect Hidden", or a "Career" spec's
//! auto-trained bonuses) are all duplicated under a new KeyName/display Name the caller chooses.
//! This used to also offer a "reuse" mode (sharing the exact same spec/styles with the source
//! class, no duplication) - removed after a real, reproduced bug: `SkillBase.GetStyleList(specID,
//! classId)` looks up a class-locked style's availability by an *exact* class-ID match, so a
//! reused spec whose styles are locked to the *source* class's ID (a very common case for any
//! real combat spec) would train and show a level, but its styles would never actually appear
//! for the new class at all - silently, nothing in the UI or server logs flags it. Cloning always
//! repoints a class-locked style's (and ability's - the exact same bug class applies to
//! SpecXAbility's own ClassId) ClassId to the new class, so this can't happen. Procs
//! (StyleXSpell) are not cloned yet - a known gap, noted in the UI.
//!
//! Renaming a spec that's already assigned to the class being edited does *not* remove the old
//! ClassXSpecialization assignment under its original key - a real, reproduced bug found live:
//! editing an existing class and re-cloning one of its own already-assigned specs under a new
//! name (e.g. "Stealth" -> "stealth_64") leaves *both* assigned, so the same ability shows twice
//! (and any points already trained stay split across both keys). The caller must explicitly
//! remove the old assignment itself if a rename is meant to replace it, not just add to it.
//!
//! Deleting a spec's row from the Edit flow's list (not renaming it, just dropping it) did
//! nothing at all until `previous_keys` was added - a real, reproduced bug found live: removing
//! "Nosferatu's Bane"/"Nosferatu's Claw" from the list and redeploying left both fully assigned
//! and untouched, since `apply_class_specs` only ever acted on the keys it was told about *this*
//! call, with no notion of "used to be assigned, now isn't." `apply_class_specs` now diffs
//! `previous_keys` (the exact assignment snapshot the Edit flow read before the user started
//! editing) against this deploy's own final keys and fully removes anything genuinely dropped -
//! but only for keys carrying this class's own clone suffix, never a plain/shared key, since
//! those may still be in active use by other classes.

use crate::deploy::{SshConfig, run_remote_sql, run_remote_sql_json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Mirrors `GlobalConstants.GetSpecToInternalIndex`'s hardcoded spec-name -> client-recognized
/// style-category index table from the real server source, so a clone can automatically inherit
/// its source spec's own category instead of needing a server-side code change for every new
/// custom spec. Keep in sync with that switch statement if the server ever adds a real DAoC spec
/// this tool doesn't know about yet (it only affects which "bucket" a style's icon/label is
/// grouped under - harmless, cosmetic-only, if a name here goes stale).
const CLIENT_STYLE_GROUPS: &[(&str, u8)] = &[
    ("Slash", 0x01),
    ("Thrust", 0x02),
    ("Parry", 0x08),
    ("Sword", 0x0E),
    ("Hammer", 0x10),
    ("Axe", 0x11),
    ("Left Axe", 0x12),
    ("Stealth", 0x13),
    ("Spear", 0x1A),
    ("Mending", 0x1D),
    ("Augmentation", 0x1E),
    ("Crush", 0x21),
    ("Pacification", 0x22),
    ("Darkness", 0x26),
    ("Suppression", 0x27),
    ("Runecarving", 0x2A),
    ("Shields", 0x2B),
    ("Flexible", 0x2E),
    ("Staff", 0x2F),
    ("Summoning", 0x30),
    ("Stormcalling", 0x32),
    ("Beastcraft", 0x3E),
    ("Polearm", 0x40),
    ("Two Handed", 0x41),
    ("Fire Magic", 0x42),
    ("Wind Magic", 0x43),
    ("Cold Magic", 0x44),
    ("Earth Magic", 0x45),
    ("Light", 0x46),
    ("Matter Magic", 0x47),
    ("Body Magic", 0x48),
    ("Spirit Magic", 0x49),
    ("Mind Magic", 0x4A),
    ("Void", 0x4B),
    ("Mana", 0x4C),
    ("Dual Wield", 0x4D),
    ("Composite Bow", 0x4E),
    ("Battlesongs", 0x52),
    ("Enhancement", 0x53),
    ("Enchantments", 0x54),
    ("Rejuvenation", 0x58),
    ("Smite", 0x59),
    ("Longbows", 0x5A),
    ("Crossbows", 0x5B),
    ("Chants", 0x61),
    ("Instruments", 0x62),
    ("Blades", 0x65),
    ("Blunt", 0x66),
    ("Piercing", 0x67),
    ("Large Weapons", 0x68),
    ("Mentalism", 0x69),
    ("Regrowth", 0x6A),
    ("Nurture", 0x6B),
    ("Nature", 0x6C),
    ("Music", 0x6D),
    ("Celtic Dual", 0x6E),
    ("Celtic Spear", 0x70),
    ("Recurve Bow", 0x71),
    ("Valor", 0x72),
    ("Pathfinding", 0x74),
    ("Envenom", 0x75),
    ("Critical Strike", 0x76),
    ("Deathsight", 0x78),
    ("Painworking", 0x79),
    ("Death Servant", 0x7A),
    ("Soulrending", 0x7B),
    ("Hand to Hand", 0x7C),
    ("Scythe", 0x7D),
    ("Arboreal Path", 0x7F),
    ("Creeping Path", 0x81),
    ("Verdant Path", 0x82),
    ("Odin's Will", 0x85),
    ("Spectral Force", 0x86),
    ("Phantasmal Wail", 0x87),
    ("Ethereal Shriek", 0x88),
    ("Shadow Mastery", 0x89),
    ("Vampiiric Embrace", 0x8A),
    ("Dementia", 0x8B),
    ("Witchcraft", 0x8C),
    ("Cursing", 0x8D),
    ("Hexing", 0x8E),
    ("Fist Wraps", 0x93),
    ("Mauler Staff", 0x94),
    ("Spectral Guard", 0x95),
    ("Archery", 0x9B),
];

/// What `ClientStyleGroup` a clone should get, given its source spec's own KeyName and whatever
/// override (if any) the source itself already carries. A non-zero `source_override` means the
/// source is itself a previous clone that was already resolved - propagate it forward unchanged
/// (keeps the chain correct through "clone of a clone"). Otherwise, if the source's own KeyName
/// is one the client natively recognizes, copy that index explicitly, since the clone's own
/// brand-new KeyName never will be. Falls back to 0 (no override - the same "generic Abilities
/// bucket" behavior as any other unrecognized spec) only if neither applies.
fn client_style_group_for(source_key: &str, source_override: u8) -> u8 {
    if source_override != 0 {
        return source_override;
    }
    CLIENT_STYLE_GROUPS
        .iter()
        .find(|(name, _)| *name == source_key)
        .map(|(_, idx)| *idx)
        .unwrap_or(0)
}

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
///
/// `previous_keys` is the exact set of SpecKeyName values this class had assigned when the Edit
/// flow last read its specs (empty for a brand-new clone, which has nothing to diff against) -
/// see the module doc comment for why a key the caller dropped from `assignments` needs this to
/// actually be removed.
pub fn apply_class_specs(
    cfg: &SshConfig,
    new_class_id: i64,
    assignments: &[SpecAssignment],
    previous_keys: &[String],
) -> Result<String, String> {
    let mut log = String::new();
    for a in assignments {
        // Embedding the new class's ID directly in the stored KeyName (not just the spell lines
        // under it) gives every row this clone creates a reliable, greppable suffix -
        // `delete_deployed_class` relies on exactly this to find and remove only what was cloned
        // for a given class, never a spec shared by other classes.
        let final_key = format!("{}_{new_class_id}", a.new_key);

        // Make this idempotent/re-deployable - a real bug found live: editing an already-deployed
        // class resubmits *every* currently-assigned spec (not just newly changed ones, since the
        // Edit flow has no way to know which ones are unchanged), and clone_spec always did a
        // fresh INSERT assuming its target key is brand new - so simply redeploying a class whose
        // specs hadn't actually changed (or changing one spec while leaving others as-is) hit a
        // UNIQUE constraint violation on every unchanged spec's own already-existing KeyName.
        // Deleting whatever this exact key already fully owns first (if anything) before
        // re-cloning makes every deploy produce the same correct result regardless of how many
        // times it runs, the same way redeploying the class's own .cs file already does.
        //
        // Critical exception, a real data-loss bug found live immediately after adding the above:
        // when an unchanged spec is resubmitted, its own source_key *is* this same final_key (the
        // Edit flow reads a spec's current own KeyName back as its "source" when nothing about it
        // changed) - deleting it first, then trying to clone from it, deletes its only copy of
        // the data before clone_spec can ever read it, leaving an empty, orphaned spec that then
        // crashes any character's login (SkillBase.GetSpecialization null-derefs on a key with no
        // matching row). There is nothing to safely redo in this case anyway - it already has
        // everything a previous deploy gave it - so skip both steps entirely and leave it alone.
        if a.source_key == final_key {
            log.push_str(&format!(
                "left '{}' untouched (already its own source - nothing to reclone)\n",
                final_key
            ));
            continue;
        }
        delete_existing_clone(cfg, &final_key)?;
        clone_spec(cfg, &a.source_key, &final_key, &a.new_name, new_class_id)?;

        // If this class already had the *source* spec assigned directly (this rename is meant to
        // replace it, not just add alongside it), remove that old assignment - a real bug found
        // live: leaving it meant the same ability showed up twice (e.g. "Stealth" appearing
        // twice on the character sheet), with any already-trained points left split across both
        // keys instead of following the rename. This only removes the ClassXSpecialization row,
        // never the source spec's own Specialization/Style/etc rows (which may still be used by
        // other classes) - same "only remove what's safe" caution as delete_deployed_class.
        let source_key_q = sql_quote(&a.source_key);
        let existing = run_remote_sql_json(
            cfg,
            &format!(
                "SELECT 1 FROM ClassXSpecialization WHERE ClassID = {new_class_id} AND SpecKeyName = '{source_key_q}';"
            ),
        )?;
        if !existing.is_empty() {
            run_remote_sql(
                cfg,
                &format!(
                    "DELETE FROM ClassXSpecialization WHERE ClassID = {new_class_id} AND SpecKeyName = '{source_key_q}';"
                ),
            )?;
            log.push_str(&format!(
                "removed the old '{}' assignment this class already had - note: any points a \
                 character already trained under that key are NOT migrated automatically, check \
                 DOLCharacters.SerializedSpecs for characters on this class\n",
                a.source_key
            ));
        }

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

    // Remove whatever this class had previously but no longer does - a real bug found live:
    // removing a spec's row from the Edit flow's list and redeploying left its
    // ClassXSpecialization assignment (and all its cloned data) completely untouched, so it kept
    // showing on every character's sheet forever. `previous_keys` is the exact assignment
    // snapshot the Edit flow read before the user started editing, so anything in it that isn't
    // one of this deploy's own final keys is something the user genuinely removed.
    //
    // Only ever auto-deletes a key carrying this exact class's own clone suffix (`_<class_id>`,
    // the same convention `delete_deployed_class`'s cleanup already scopes to) - a plain/shared
    // key (e.g. "Stealth", assigned directly with no suffix) is never touched here, since the
    // same KeyName may still be in active use by other classes; only this class's own
    // ClassXSpecialization row for it would ever be safe to drop automatically, and the editor
    // has no current way to assign a shared key like that in the first place, so there's nothing
    // meaningful to clean up for it.
    let keep_keys: std::collections::HashSet<String> = assignments
        .iter()
        .map(|a| format!("{}_{new_class_id}", a.new_key))
        .collect();
    let suffix = format!("_{new_class_id}");
    for key in previous_keys {
        if keep_keys.contains(key) || !key.ends_with(&suffix) {
            continue;
        }
        delete_existing_clone(cfg, key)?;
        log.push_str(&format!(
            "removed '{}' - no longer in this class's spec assignments\n",
            key
        ));
    }

    // Real classic-server/freeshard practice: a class's spec lineup changing (exactly what this
    // function just did) can invalidate how an already-playing character already spent their
    // points - what was a legal build a moment ago might reference a spec the class no longer
    // has, or no longer matches how points were meant to be distributed. Rather than leave that
    // silently stale, grant every character currently on this class a free full respec token
    // (the same RespecAmountAllSkill the real `/respec all` command consumes) - they keep playing
    // with their current build uninterrupted, and can freely reallocate every point next time
    // they choose to, with nothing lost. Adds to (never overwrites) any respec tokens a character
    // already had saved up.
    log.push_str(&grant_respec_to_characters(cfg, new_class_id)?);

    Ok(log)
}

/// Fully removes whatever an exact spec KeyName already owns (Specialization, its SpellLine(s)
/// and their LineXSpell entries, Style, SpecXAbility, and its ClassXSpecialization assignment) -
/// everything clone_spec creates - if it exists at all. A clean no-op otherwise. Lets
/// apply_class_specs call this unconditionally right before re-cloning so a redeploy is always
/// idempotent instead of colliding with whatever the same key already owns from a previous one.
fn delete_existing_clone(cfg: &SshConfig, key: &str) -> Result<(), String> {
    let key_q = sql_quote(key);

    let exists = run_remote_sql_json(
        cfg,
        &format!("SELECT 1 FROM Specialization WHERE KeyName = '{key_q}';"),
    )?;
    if exists.is_empty() {
        return Ok(());
    }

    let lines = run_remote_sql_json(
        cfg,
        &format!("SELECT KeyName FROM SpellLine WHERE Spec = '{key_q}';"),
    )?;
    for line in &lines {
        let line_key_q = sql_quote(&str_field(line, "KeyName"));
        run_remote_sql(
            cfg,
            &format!("DELETE FROM LineXSpell WHERE LineName = '{line_key_q}';"),
        )?;
    }
    run_remote_sql(
        cfg,
        &format!("DELETE FROM SpellLine WHERE Spec = '{key_q}';"),
    )?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM Style WHERE SpecKeyName = '{key_q}';"),
    )?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM SpecXAbility WHERE Spec = '{key_q}';"),
    )?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM ClassXSpecialization WHERE SpecKeyName = '{key_q}';"),
    )?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM Specialization WHERE KeyName = '{key_q}';"),
    )?;
    Ok(())
}

/// Grants every character currently playing `class_id` one free full-respec token
/// (RespecAmountAllSkill, consumed by the real `/respec all` command) - called after any class
/// spec change, since that can invalidate a character's existing build. Harmless/no-op for a
/// brand-new class with no characters on it yet.
fn grant_respec_to_characters(cfg: &SshConfig, class_id: i64) -> Result<String, String> {
    let affected = run_remote_sql_json(
        cfg,
        &format!("SELECT Name FROM DOLCharacters WHERE Class = {class_id};"),
    )?;
    if affected.is_empty() {
        return Ok(String::new());
    }
    run_remote_sql(
        cfg,
        &format!(
            "UPDATE DOLCharacters SET RespecAmountAllSkill = RespecAmountAllSkill + 1 WHERE Class = {class_id};"
        ),
    )?;
    let names: Vec<String> = affected.iter().map(|r| str_field(r, "Name")).collect();
    Ok(format!(
        "granted a free full respec to {} character(s) already on this class: {}\n",
        names.len(),
        names.join(", ")
    ))
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

    // The source spec's own ClientStyleGroup (if it's itself a previous clone) decides whether
    // this new clone inherits an explicit override or needs one computed fresh - see
    // client_style_group_for's doc comment.
    let source_rows = run_remote_sql_json(
        cfg,
        &format!("SELECT ClientStyleGroup FROM Specialization WHERE KeyName = '{source_key_q}';"),
    )?;
    let source_override = source_rows
        .first()
        .and_then(|r| r.get("ClientStyleGroup"))
        .and_then(Value::as_i64)
        .unwrap_or(0) as u8;
    let client_style_group = client_style_group_for(source_key, source_override);

    let mut batch = format!(
        "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
         SELECT '{new_key_q}', '{new_name_q}', Description, Icon, Implementation, {client_style_group} \
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
        let is_base = line.get("IsBaseLine").and_then(Value::as_i64).unwrap_or(0);
        let new_line_key = format!("{new_key}_{}", old_line_key);
        let old_line_key_q = sql_quote(&old_line_key);
        let new_line_key_q = sql_quote(&new_line_key);
        // The line's own display Name (shown wherever spell lines are listed, e.g. the
        // spellbook) must follow the caller's chosen rename too, not the source's own - a real
        // bug found live: a spec cloned and renamed (e.g. "Dementia" -> "Fervor") still showed
        // its spell line as "Dementia" everywhere, since this copied the source's Name verbatim
        // instead of using new_name like the Specialization row itself already correctly does.
        batch.push_str(&format!(
            "INSERT INTO SpellLine (KeyName, Name, Spec, IsBaseLine, ClassIDHint) \
             VALUES ('{new_line_key_q}', '{new_name_q}', '{new_key_q}', {is_base}, {new_class_id});\n"
        ));

        // LineXSpell_ID is a GUID-style primary key the real schema requires explicitly (not an
        // autoincrement DB rowid like Style's or SpecXAbility's) - a real bug found live: cloning
        // any spec whose spell line has more than one spell (the common case; it only went
        // unnoticed this long because every spec cloned so far happened to have at most one)
        // failed with "UNIQUE constraint failed: LineXSpell.LineXSpell_ID", since every row an
        // INSERT...SELECT produced shared the column's bare default ('') and collided with each
        // other from the second row on. Fetch the rows to clone first, then insert each with its
        // own freshly generated GUID.
        let spells = run_remote_sql_json(
            cfg,
            &format!("SELECT SpellID, Level FROM LineXSpell WHERE LineName = '{old_line_key_q}';"),
        )?;
        for spell in &spells {
            let spell_id = int_field(spell, "SpellID");
            let level = int_field(spell, "Level");
            let row_guid = Uuid::new_v4();
            batch.push_str(&format!(
                "INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) \
                 VALUES ('{new_line_key_q}', {spell_id}, {level}, '{row_guid}');\n"
            ));
        }
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

    // Style.ID is a *separate* field from the DB primary key StyleID - it's what a chain style's
    // own OpeningRequirementValue references (AttackResultRequirement=7, "a style is required")
    // to name its prerequisite, confirmed in the real server source and packet code
    // (`it.Item1.ID == style.OpeningRequirementValue`). The INSERT above never set it (so every
    // cloned style got the column's bare default, 0 - no two clones distinguishable at all), and
    // even if it had just copied the source's own ID verbatim, that value belongs to the
    // *un-cloned* prerequisite style, not whichever row this clone got - a real, reproduced bug
    // found live: a cloned "Gale" style still pointed at the source class's "Snow Shower" (by its
    // old ID), which doesn't exist in the new spec's own ID space, so the client showed no
    // prerequisite for it at all.
    //
    // Fixed in two passes, both scoped to exactly the rows this clone just inserted - first, give
    // every cloned style its own StyleID (the DB primary key, already guaranteed unique) as its
    // ID too, which is all a plain, non-chain style needs. Then, for every chain style among
    // them, look up its prerequisite by matching *names* between the original spec's styles and
    // this clone's own (now-ID-correct) styles, and repoint OpeningRequirementValue at the
    // clone's own prerequisite's new ID - never the original's.
    batch.push_str(&format!(
        "UPDATE Style SET ID = StyleID WHERE SpecKeyName = '{new_key_q}';\n"
    ));
    batch.push_str(&format!(
        "UPDATE Style SET OpeningRequirementValue = COALESCE((\
             SELECT new_style.StyleID FROM Style AS old_style \
             JOIN Style AS new_style ON new_style.Name = old_style.Name AND new_style.SpecKeyName = '{new_key_q}' \
             WHERE old_style.SpecKeyName = '{source_key_q}' AND old_style.ID = Style.OpeningRequirementValue \
             LIMIT 1\
         ), OpeningRequirementValue) \
         WHERE SpecKeyName = '{new_key_q}' AND AttackResultRequirement = 7;\n"
    ));

    // SpecXAbility: level-granted abilities (e.g. Stealth's "Safe Fall"/"Detect Hidden"/etc.,
    // or a "Career" spec's auto-trained bonuses like Evade) - a real, confirmed gap found live:
    // cloning a spec with these (any Implementation, not just plain combat specs) silently
    // dropped every one of them, since this table was never touched by cloning at all. Carries
    // the exact same two real bugs Style already had to guard against, confirmed with the same
    // live data shape: a non-zero ClassId locks an ability to that *exact* class only (so any
    // non-zero lock must repoint to the new class, unconditionally), and the same
    // (AbilityKey, SpecLevel, AbilityLevel) is frequently duplicated once per originally-eligible
    // class (Stealth's 35 real rows are only 9 distinct abilities, repeated across 3-4 classes
    // each) - GROUP BY collapses that before cloning, same as Style.
    let ability_class_expr = format!("CASE WHEN ClassId != 0 THEN {new_class_id} ELSE 0 END");
    batch.push_str(&format!(
        "INSERT INTO SpecXAbility (Spec, SpecLevel, AbilityKey, AbilityLevel, ClassId) \
         SELECT '{new_key_q}', SpecLevel, AbilityKey, AbilityLevel, {ability_class_expr} \
         FROM SpecXAbility WHERE Spec = '{source_key_q}' GROUP BY AbilityKey, SpecLevel, AbilityLevel;\n"
    ));

    run_remote_sql(cfg, &batch)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::SshConfig;
    use std::process::Command;

    #[test]
    fn client_style_group_for_propagates_a_chained_override_unchanged() {
        // "Celtic Dual" has its own recognized value (0x6E), but a non-zero source_override
        // (meaning the source is itself already a resolved clone) must win regardless - a
        // "clone of a clone" must keep whatever its immediate parent actually resolved to, not
        // recompute a fresh lookup that might disagree with it.
        assert_eq!(client_style_group_for("Celtic Dual", 0x50), 0x50);
    }

    #[test]
    fn client_style_group_for_resolves_a_recognized_real_spec_name() {
        assert_eq!(client_style_group_for("Celtic Dual", 0), 0x6E);
        assert_eq!(client_style_group_for("Slash", 0), 0x01);
    }

    #[test]
    fn client_style_group_for_falls_back_to_zero_for_an_unrecognized_name() {
        // A brand-new custom KeyName the client was never taught - same "no override, generic
        // bucket" behavior as the real server's own default case.
        assert_eq!(client_style_group_for("nosferatusclaw_64", 0), 0);
    }

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
            CREATE TABLE DOLCharacters (Name TEXT, Class INTEGER, RespecAmountAllSkill INTEGER DEFAULT 0);

            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) VALUES ('slash', 'Slash', 'Slashing weapons', 100, NULL, 0);
            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) VALUES ('smite', 'Smite', 'Holy damage spells', 200, NULL, 0);
            INSERT INTO ClassXSpecialization VALUES (1, 'slash', 0);
            INSERT INTO ClassXSpecialization VALUES (1, 'smite', 0);
            INSERT INTO Style VALUES (NULL, 1, 'Pulverize', 'slash', 5, 300, 5, 0, 1, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);
            INSERT INTO Style VALUES (NULL, 0, 'Backlash', 'slash', 10, 301, 5, 1, 2, 1, 1, 1, 1.0, 1.0, 5, 0, 0, 0, 0, 0);
            INSERT INTO SpellLine VALUES ('smite_base', 'Smiting Spells', 'smite', 0, 1);
            INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('smite_base', 9001, 1, 'seed-smite-1');
            INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('smite_base', 9002, 10, 'seed-smite-2');
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
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

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
    fn clone_of_a_recognized_spec_auto_populates_client_style_group() {
        // The real-world Nosferatu's Claw scenario end-to-end: cloning "Celtic Dual" (a real,
        // client-recognized spec name) under a brand-new KeyName must not leave the clone's
        // styles grouped under the client's generic "Abilities" bucket - the clone should
        // automatically inherit Celtic Dual's own recognized category (0x6E), with no server
        // code change and no manual step needed.
        let dir = setup_test_db("clone-recognized-name");
        let cfg = test_cfg(&dir);
        let sql = "
            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                VALUES ('Celtic Dual', 'Celtic Dual', 'Dual wield', 300, NULL, 0);
        ";
        let status = Command::new("sqlite3")
            .arg(dir.join("data/opendaoc.sqlite3.db"))
            .arg(sql)
            .status()
            .unwrap();
        assert!(status.success());

        let assignments = vec![SpecAssignment {
            source_key: "Celtic Dual".to_string(),
            new_key: "nosferatusclaw".to_string(),
            new_name: "Nosferatu's Claw".to_string(),
            level_acquired: 0,
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT ClientStyleGroup FROM Specialization WHERE KeyName='nosferatusclaw_64';",
        )
        .unwrap();
        assert_eq!(int_field(&rows[0], "ClientStyleGroup"), 0x6E);

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
            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) VALUES ('dual', 'Celtic Dual', 'Dual wield', 300, NULL, 0);
            INSERT INTO Style VALUES (NULL, 999, 'Misty Gloom', 'dual', 2, 400, 0, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 400);
            INSERT INTO Style VALUES (NULL, 888, 'Misty Gloom', 'dual', 2, 400, 0, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 400);
            INSERT INTO Style VALUES (NULL, 777, 'Misty Gloom', 'dual', 2, 400, 0, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 400);
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
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

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
    fn clone_remaps_a_chain_style_prerequisite_to_its_own_cloned_prerequisite() {
        // Reproduces a real bug found live: "Gale" (a chain style requiring "Snow Shower" to
        // have been used first) still pointed at Snow Shower's *original* ID after cloning -
        // meaningless in the new spec's own ID space (every cloned style's ID was left at its
        // bare default, 0, and nothing remapped OpeningRequirementValue in the first place), so
        // the client showed no prerequisite for it at all. Both styles must be cloned together
        // (same spec) for the remap to have something to resolve to - which it also confirms by
        // asserting the two new IDs actually differ from both each other and the originals.
        let dir = setup_test_db("clone-chain-style");
        let cfg = test_cfg(&dir);
        let sql = "
            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) VALUES ('dual', 'Celtic Dual', 'Dual wield', 300, NULL, 0);
            INSERT INTO Style (StyleID, ClassId, Name, SpecKeyName, SpecLevelRequirement, Icon, EnduranceCost, StealthRequirement, OpeningRequirementType, OpeningRequirementValue, AttackResultRequirement, WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, BonusToDefense, TwoHandAnimation, RandomProc, ArmorHitLocation, ID)
                VALUES (NULL, 0, 'Snow Shower', 'dual', 2, 400, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 323);
            INSERT INTO Style (StyleID, ClassId, Name, SpecKeyName, SpecLevelRequirement, Icon, EnduranceCost, StealthRequirement, OpeningRequirementType, OpeningRequirementValue, AttackResultRequirement, WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, BonusToDefense, TwoHandAnimation, RandomProc, ArmorHitLocation, ID)
                VALUES (NULL, 0, 'Gale', 'dual', 4, 401, 10, 0, 0, 323, 7, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 324);
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
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT StyleID, ID, Name, OpeningRequirementValue FROM Style WHERE SpecKeyName='nosferatusclaw_64' ORDER BY Name;",
        )
        .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let gale = rows
            .iter()
            .find(|r| str_field(r, "Name") == "Gale")
            .unwrap();
        let snow_shower = rows
            .iter()
            .find(|r| str_field(r, "Name") == "Snow Shower")
            .unwrap();

        assert_ne!(
            int_field(snow_shower, "ID"),
            0,
            "a cloned style must get its own real ID, not the column's bare default"
        );
        assert_ne!(
            int_field(snow_shower, "ID"),
            323,
            "a cloned style's ID must not collide with the source's own ID either"
        );
        assert_eq!(
            int_field(gale, "OpeningRequirementValue"),
            int_field(snow_shower, "ID"),
            "Gale's prerequisite must repoint to its own cloned Snow Shower, not the original"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_deep_clones_and_repoints_spec_x_ability() {
        // Mirrors the Style repoint/dedup test, but for SpecXAbility - a real, confirmed gap
        // found live: a Stealth-type clone silently dropped every one of its level-granted
        // abilities (35 real rows, Safe Fall/Detect Hidden/etc.) since SpecXAbility was never
        // cloned at all. The real data also had the exact same (AbilityKey, SpecLevel) once per
        // originally-eligible class (e.g. "Safe Fall" at level 10 repeated across 4 classes), so
        // this also confirms the clone collapses those into one row per ability, repointed to
        // the new class.
        let dir = setup_test_db("clone-spec-ability");
        let cfg = test_cfg(&dir);
        let sql = "
            INSERT INTO SpecXAbility (Spec, SpecLevel, AbilityKey, AbilityLevel, ClassId) VALUES ('slash', 10, 'SafeFall', 1, 9);
            INSERT INTO SpecXAbility (Spec, SpecLevel, AbilityKey, AbilityLevel, ClassId) VALUES ('slash', 10, 'SafeFall', 1, 23);
            INSERT INTO SpecXAbility (Spec, SpecLevel, AbilityKey, AbilityLevel, ClassId) VALUES ('slash', 10, 'SafeFall', 1, 49);
            INSERT INTO SpecXAbility (Spec, SpecLevel, AbilityKey, AbilityLevel, ClassId) VALUES ('slash', 5, 'Evade', 2, 0);
        ";
        let status = Command::new("sqlite3")
            .arg(dir.join("data/opendaoc.sqlite3.db"))
            .arg(sql)
            .status()
            .unwrap();
        assert!(status.success());

        let assignments = vec![SpecAssignment {
            source_key: "slash".to_string(),
            new_key: "earthshatter".to_string(),
            new_name: "Earthshatter".to_string(),
            level_acquired: 0,
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT AbilityKey, ClassId FROM SpecXAbility WHERE Spec='earthshatter_64' ORDER BY AbilityKey;",
        )
        .unwrap();
        assert_eq!(
            rows.len(),
            2,
            "three duplicate rows for the same ability must collapse to one, not {rows:?}"
        );
        let safe_fall = rows
            .iter()
            .find(|r| str_field(r, "AbilityKey") == "SafeFall")
            .unwrap();
        assert_eq!(
            int_field(safe_fall, "ClassId"),
            64,
            "a class-locked ability must repoint to the new class"
        );
        let evade = rows
            .iter()
            .find(|r| str_field(r, "AbilityKey") == "Evade")
            .unwrap();
        assert_eq!(
            int_field(evade, "ClassId"),
            0,
            "an unrestricted (ClassId=0) ability must stay unrestricted"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renaming_an_already_assigned_spec_removes_the_old_assignment() {
        // Reproduces a real bug found live: editing an existing class and re-cloning one of its
        // own already-assigned specs under a new name (e.g. "Stealth" -> "stealth_64") left both
        // the old and new ClassXSpecialization rows in place, so the same ability showed up
        // twice on the character sheet.
        let dir = setup_test_db("rename-already-assigned");
        let cfg = test_cfg(&dir);
        let sql = "INSERT INTO ClassXSpecialization VALUES (64, 'slash', -9);";
        let status = Command::new("sqlite3")
            .arg(dir.join("data/opendaoc.sqlite3.db"))
            .arg(sql)
            .status()
            .unwrap();
        assert!(status.success());

        let assignments = vec![SpecAssignment {
            source_key: "slash".to_string(),
            new_key: "earthshatter".to_string(),
            new_name: "Earthshatter".to_string(),
            level_acquired: -9,
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let rows = list_class_specs(&cfg, 64).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "the old 'slash' assignment must be removed, not left alongside the rename: {rows:?}"
        );
        assert_eq!(rows[0].spec_key_name, "earthshatter_64");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn applying_class_specs_grants_a_free_respec_to_existing_characters() {
        // Real classic-server/freeshard practice: a class's spec lineup changing can invalidate
        // how an already-playing character already spent their points, so every character on
        // that class should get a free full respec token instead of being silently left with a
        // stale/invalid build. A character on a *different* class must be untouched, and any
        // respec tokens a character already had saved up must be added to, not overwritten.
        let dir = setup_test_db("grant-respec");
        let cfg = test_cfg(&dir);
        let sql = "
            INSERT INTO DOLCharacters (Name, Class, RespecAmountAllSkill) VALUES ('Aeligord', 64, 0);
            INSERT INTO DOLCharacters (Name, Class, RespecAmountAllSkill) VALUES ('Savaen', 64, 2);
            INSERT INTO DOLCharacters (Name, Class, RespecAmountAllSkill) VALUES ('SomeoneElse', 23, 0);
        ";
        let status = Command::new("sqlite3")
            .arg(dir.join("data/opendaoc.sqlite3.db"))
            .arg(sql)
            .status()
            .unwrap();
        assert!(status.success());

        let assignments = vec![SpecAssignment {
            source_key: "slash".to_string(),
            new_key: "earthshatter".to_string(),
            new_name: "Earthshatter".to_string(),
            level_acquired: 0,
        }];
        let log = apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();
        assert!(log.contains("Aeligord"), "{log}");
        assert!(log.contains("Savaen"), "{log}");
        assert!(!log.contains("SomeoneElse"), "{log}");

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT Name, RespecAmountAllSkill FROM DOLCharacters ORDER BY Name;",
        )
        .unwrap();
        let aeligord = rows
            .iter()
            .find(|r| str_field(r, "Name") == "Aeligord")
            .unwrap();
        let savaen = rows
            .iter()
            .find(|r| str_field(r, "Name") == "Savaen")
            .unwrap();
        let other = rows
            .iter()
            .find(|r| str_field(r, "Name") == "SomeoneElse")
            .unwrap();
        assert_eq!(int_field(aeligord, "RespecAmountAllSkill"), 1);
        assert_eq!(
            int_field(savaen, "RespecAmountAllSkill"),
            3,
            "an existing respec token must be added to, not overwritten"
        );
        assert_eq!(
            int_field(other, "RespecAmountAllSkill"),
            0,
            "a character on a different class must be untouched"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_class_specs_is_idempotent_across_redeploys() {
        // Reproduces a real bug found live: editing an already-deployed class resubmits *every*
        // currently-assigned spec, not just ones that actually changed - the Edit flow has no way
        // to know which are unchanged. Redeploying (even to drop one unrelated spec, or with
        // nothing changed at all) therefore hit a UNIQUE constraint violation on every spec's own
        // already-existing KeyName, since clone_spec always did a fresh INSERT assuming it was
        // brand new. A second call with the exact same assignments must succeed and produce the
        // same correct result, not error.
        let dir = setup_test_db("idempotent-redeploy");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "slash".to_string(),
            new_key: "earthshatter".to_string(),
            new_name: "Earthshatter".to_string(),
            level_acquired: 0,
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();
        // Redeploy with the exact same assignment, simulating an Edit-mode resubmit.
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let specs = list_specializations(&cfg).unwrap();
        let matching: Vec<_> = specs
            .iter()
            .filter(|s| s.key_name == "earthshatter_64")
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "redeploying must not leave duplicate Specialization rows: {matching:?}"
        );

        let class_specs = list_class_specs(&cfg, 64).unwrap();
        let assigned: Vec<_> = class_specs
            .iter()
            .filter(|s| s.spec_key_name == "earthshatter_64")
            .collect();
        assert_eq!(
            assigned.len(),
            1,
            "redeploying must not leave duplicate ClassXSpecialization rows: {assigned:?}"
        );

        // The clone's own data (styles in this case) must still be intact after the redeploy,
        // not silently dropped by the cleanup-then-reclone pass.
        let rows = run_remote_sql_json(
            &cfg,
            "SELECT COUNT(*) as c FROM Style WHERE SpecKeyName='earthshatter_64';",
        )
        .unwrap();
        assert_eq!(int_field(&rows[0], "c"), 2, "{rows:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resubmitting_a_spec_with_itself_as_source_does_not_destroy_it() {
        // A critical data-loss bug found live immediately after the fix above: the Edit flow's
        // "load this class's current specs" reads an already-cloned spec's own current KeyName
        // back as its "source" when nothing about it was changed, so source_key equals the exact
        // same final_key being redeployed. The previous fix's "delete whatever this key already
        // owns, then clone from it" sequence deleted the spec's only copy of its own data before
        // clone_spec could ever read it - cloning "from itself" after deleting itself finds
        // nothing to copy, leaving an empty, orphaned Specialization that then crashes any
        // character's login entirely (confirmed live: SkillBase.GetSpecialization null-derefs on
        // a key with no matching row). This must leave the spec's real data completely untouched.
        let dir = setup_test_db("resubmit-self-source");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        // Resubmit it exactly as the Edit flow would for an unchanged spec: source_key is now its
        // own already-cloned key, not the original "smite".
        let resubmit = vec![SpecAssignment {
            source_key: "frostbolt_tree_64".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
        }];
        apply_class_specs(&cfg, 64, &resubmit, &[]).unwrap();

        let specs = list_specializations(&cfg).unwrap();
        assert!(
            specs.iter().any(|s| s.key_name == "frostbolt_tree_64"),
            "the spec itself must still exist: {specs:?}"
        );
        let lines = run_remote_sql_json(
            &cfg,
            "SELECT KeyName FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert_eq!(
            lines.len(),
            1,
            "its spell line must not have been wiped out: {lines:?}"
        );
        let spells = run_remote_sql_json(
            &cfg,
            &format!(
                "SELECT SpellID FROM LineXSpell WHERE LineName='{}';",
                str_field(&lines[0], "KeyName")
            ),
        )
        .unwrap();
        assert_eq!(
            spells.len(),
            2,
            "its spells must not have been wiped out: {spells:?}"
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
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

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
        assert_eq!(
            str_field(&lines[0], "Name"),
            "Frostbolt Tree",
            "the cloned line's own display name must follow the rename, not the source's name"
        );

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

    #[test]
    fn a_spec_removed_from_the_assignment_list_is_fully_deleted() {
        // Reproduces a real bug found live: removing a spec's row from the Edit flow's list (not
        // renaming it, just dropping it) and redeploying left it fully assigned and untouched -
        // apply_class_specs had no way to know a previously-assigned key wasn't being resubmitted
        // on purpose versus simply forgotten about. Deploy two specs, then redeploy with only one
        // of them and the other's key in `previous_keys` (exactly what the Edit flow's own load
        // would have captured) - the dropped one must be completely gone: its
        // ClassXSpecialization assignment, its Specialization row, and its cloned Style.
        let dir = setup_test_db("remove-from-list");
        let cfg = test_cfg(&dir);

        let assignments = vec![
            SpecAssignment {
                source_key: "slash".to_string(),
                new_key: "earthshatter".to_string(),
                new_name: "Earthshatter".to_string(),
                level_acquired: 0,
            },
            SpecAssignment {
                source_key: "smite".to_string(),
                new_key: "frostbolt_tree".to_string(),
                new_name: "Frostbolt Tree".to_string(),
                level_acquired: 0,
            },
        ];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        // Redeploy with only "Earthshatter" kept; "Frostbolt Tree" was removed from the list in
        // the UI, and its previously-assigned key is passed back as the Edit flow would.
        let kept = vec![assignments[0].clone()];
        let previous_keys = vec!["earthshatter_64".to_string(), "frostbolt_tree_64".to_string()];
        let log = apply_class_specs(&cfg, 64, &kept, &previous_keys).unwrap();
        assert!(
            log.contains("removed 'frostbolt_tree_64'"),
            "log should mention the removal: {log}"
        );

        let class_specs = list_class_specs(&cfg, 64).unwrap();
        assert!(
            class_specs.iter().any(|c| c.spec_key_name == "earthshatter_64"),
            "the kept spec must still be assigned: {class_specs:?}"
        );
        assert!(
            !class_specs.iter().any(|c| c.spec_key_name == "frostbolt_tree_64"),
            "the removed spec must no longer be assigned: {class_specs:?}"
        );

        let specs = list_specializations(&cfg).unwrap();
        assert!(
            !specs.iter().any(|s| s.key_name == "frostbolt_tree_64"),
            "the removed spec's own row must be fully deleted, not just unassigned: {specs:?}"
        );
        let lines = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert!(lines.is_empty(), "its spell line must be cleaned up too");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_plain_shared_key_in_previous_keys_is_never_auto_removed() {
        // A plain/shared key (no class-suffix, e.g. a spec assigned by its original real KeyName
        // rather than one of this tool's clones) must never be auto-deleted by the removal pass,
        // even if it's absent from the current assignment list - it may still be in active use by
        // other classes, and the editor has no path that produces such a key as a "clone" to
        // safely reverse in the first place.
        let dir = setup_test_db("plain-key-untouched");
        let cfg = test_cfg(&dir);

        apply_class_specs(&cfg, 64, &[], &["slash".to_string()]).unwrap();

        // "slash" (the real, unsuffixed shared spec) must be completely unaffected.
        let specs = list_specializations(&cfg).unwrap();
        assert!(
            specs.iter().any(|s| s.key_name == "slash"),
            "a plain shared key must never be deleted by this pass: {specs:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
