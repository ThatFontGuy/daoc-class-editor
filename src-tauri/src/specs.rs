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
//! SpellLine(s), their LineXSpell entries, its Style (melee skill) rows, its SpecXAbility
//! (level-granted abilities, e.g. Stealth's "Safe Fall"/"Detect Hidden", or a "Career" spec's
//! auto-trained bonuses), and its Style procs (StyleXSpell) are all duplicated under a new
//! KeyName/display Name the caller chooses. This used to also offer a "reuse" mode (sharing the
//! exact same spec/styles with the source class, no duplication) - removed after a real,
//! reproduced bug: `SkillBase.GetStyleList(specID, classId)` looks up a class-locked style's
//! availability by an *exact* class-ID match, so a reused spec whose styles are locked to the
//! *source* class's ID (a very common case for any real combat spec) would train and show a
//! level, but its styles would never actually appear for the new class at all - silently, nothing
//! in the UI or server logs flags it. Cloning always repoints a class-locked style's (and
//! ability's - the exact same bug class applies to SpecXAbility's own ClassId) ClassId to the new
//! class, so this can't happen.
//!
//! A second, deeper gap in the same spirit, found later and fixed in `spell_ownership.rs`:
//! LineXSpell/StyleXSpell entries used to just point at the *source*'s own literal SpellID
//! verbatim - so a clone's spells were never actually independent data, just new pointers at
//! whatever the source (or any other class entirely, if that spell happens to be reused
//! elsewhere) already had. Editing a spell's own values would have silently edited it everywhere
//! it's referenced. Every spell (cast or proc) this function clones now gets its own fully
//! independent `Spell` row via `SpellCloner`, tracked in `ClassEditorSpellClone` so it can later
//! be safely deleted or edited without ever touching a row this class doesn't itself own.
//! `migrate_class_to_owned_spells` (below) does the same conversion for a class that was cloned
//! before this fix existed.
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
use crate::spell_ownership::{
    SpellCloner, clone_style_with_fresh_id, columns_excluding_pk, delete_owned_spells,
    delete_style_procs_for_styles, ensure_tracking_table, sql_literal, style_x_spell_guid_column,
};
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

/// Spec KeyNames the real server hardcodes a mechanic to by their exact, literal name - cloning
/// any of these under a new key doesn't just lose cosmetic flexibility, it silently breaks the
/// actual gameplay mechanic while still looking correct on the character sheet, since the server
/// looks the real name up directly instead of reading it generically from whatever key a
/// character actually trained under. Confirmed live and by exact citation in the real server
/// source (not a guess) - a real, reproduced bug: cloning "Stealth"/"Sword" for Nosferatu left
/// both showing fully trained, but the character could not actually stealth and had lost her
/// weapon's entire spec-driven damage bonus, because:
///
/// - Every weapon-type spec (every value `SkillBase.InitializeObjectTypeToSpec` ever assigns) is
///   resolved generically from a weapon's own `Object_Type` via `SkillBase.ObjectTypeToSpec`, then
///   `GamePlayer.GetWeaponSkill`/`WeaponBaseSpecLevel` and
///   `AbstractServerRules.GetObject(Base)SpecLevel` call `GetModifiedSpecLevel`/`GetBaseSpecLevel`
///   on that exact resolved name - never on the class's own cloned key, which this lookup has no
///   way to know about. A class-specific clone is therefore invisible to the damage formula
///   entirely: Staff, Shortbow, Fist Wraps, Mauler Staff, Crush, Slash, Thrust, Two Handed,
///   Polearm, Flexible, Crossbows, Longbows, Archery, Hammer, Sword, Left Axe, Axe, Hand to Hand,
///   Spear, Thrown Weapons, Composite Bow, Blunt, Blades, Piercing, Large Weapons, Celtic Spear,
///   Scythe, Shields, Envenom, Recurve Bow.
/// - A handful of other mechanics have their own direct, hardcoded `GetModifiedSpecLevel(Specs.X)`
///   call sites that bypass a player's own trained key the exact same way (`StealthSpecHandler.cs`,
///   `MaxSpeedCalculator.cs`, `AttackComponent.cs`, `ParryChanceCalculator.cs`): Stealth, Parry,
///   Dual Wield, Celtic Dual, Savagery.
///
/// This is why Vampiir's caster specs (Dementia/Shadow Mastery/Vampiiric Embrace) were safe to
/// clone and rename for Nosferatu but Stealth/Sword were not: a caster spec's spell power comes
/// from `GamePlayer.GetModifiedSpecLevel` reading the player's *own* trained key generically (see
/// its doc comment) - cloning only loses minor item/RA bonus stacking (`SkillBase.SpecToSkill`'s
/// own hardcoded map), never the core mechanic itself.
const NEVER_CLONE_SPECS: &[&str] = &[
    "Staff",
    "Shortbow",
    "Fist Wraps",
    "Mauler Staff",
    "Crush",
    "Slash",
    "Thrust",
    "Two Handed",
    "Polearm",
    "Flexible",
    "Crossbows",
    "Longbows",
    "Archery",
    "Hammer",
    "Sword",
    "Left Axe",
    "Axe",
    "Hand to Hand",
    "Spear",
    "Thrown Weapons",
    "Composite Bow",
    "Blunt",
    "Blades",
    "Piercing",
    "Large Weapons",
    "Celtic Spear",
    "Scythe",
    "Shields",
    "Envenom",
    "Recurve Bow",
    "Stealth",
    "Parry",
    "Dual Wield",
    "Celtic Dual",
    "Savagery",
];

/// Same normalization `deriveSpecKey` on the frontend applies to a chosen name before appending
/// the class suffix - needed here to recognize a key like "sword_64" as a previous (mistaken)
/// clone of "Sword" once the suffix is stripped off.
fn normalize_for_match(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// If `key` is - or is a previous clone of - one of `NEVER_CLONE_SPECS`, returns its one real
/// canonical name, the only KeyName it can ever actually function under. Checks a direct match
/// (the common case: assigning straight from the real spec) and a suffix-stripped match (healing
/// a key like "sword_64" this tool itself mistakenly cloned before this fix existed).
fn resolve_uncloneable_spec(key: &str, class_id: i64) -> Option<&'static str> {
    if let Some(name) = NEVER_CLONE_SPECS.iter().find(|n| **n == key) {
        return Some(name);
    }
    let suffix = format!("_{class_id}");
    let stripped = key.strip_suffix(&suffix)?;
    let stripped_norm = normalize_for_match(stripped);
    NEVER_CLONE_SPECS
        .iter()
        .find(|n| normalize_for_match(n) == stripped_norm)
        .copied()
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

/// One SpellLine the user chose to include or exclude when cloning a spec, plus any individual
/// spells to leave out of an included line. Lets a spec spanning several real SpellLines (e.g.
/// "Mind Magic" has both "Mind Twisting" and "Domination"; "Enhancement" has five, one per real
/// class that shares it) be cloned down to just the one line actually wanted, rather than always
/// pulling in every line the source spec has - a real, reproduced problem: cloning "Mind Magic"
/// for just its "Mind Twisting" spells also silently brought "Domination" along for the ride.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpellLineSelection {
    pub source_line_key: String,
    pub included: bool,
    #[serde(default)]
    pub excluded_spell_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecAssignment {
    pub source_key: String,
    pub new_key: String,
    pub new_name: String,
    pub level_acquired: i64,
    /// Empty means "no customization was made - clone every SpellLine and every spell in it, in
    /// full" (the original, still-default behavior, and what every pre-existing saved project or
    /// test keeps getting via `#[serde(default)]`). Non-empty is the authoritative, explicit list
    /// of what to do with the source spec's own lines - any line of the source's that isn't
    /// mentioned in it at all is excluded.
    #[serde(default)]
    pub spell_line_selection: Vec<SpellLineSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecSpellLineRow {
    pub key_name: String,
    pub name: String,
    pub is_base_line: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineSpellRow {
    pub spell_id: i64,
    pub name: String,
    pub level: i64,
    /// The real server's own `Spell.SpellGroup` - nonzero links together every level of what's
    /// really one spell's progression (e.g. a Sorcerer's "Inflict Minor Forgetfulness" ->
    /// "Inflict Forgetfulness" -> "Inflict Minor Amnesia" -> "Inflict Amnesia" ->
    /// "Inflict Oblivion" all share one SpellGroup, confirmed live) even though each level's own
    /// display Name changes along the way - 0 means "not part of any progression, stands alone."
    /// Lets the picker group a progression onto one row the same way real character planners
    /// (e.g. eden-daoc.net's) do, instead of a flat list ordered only by level.
    pub spell_group: i64,
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

/// One spec a class currently has assigned, enriched for the "Classes" tab's own two summary
/// lists (combat specs vs. spell specs) - just enough to decide which group(s) to show it in and
/// what status badge to give it, with no spell/style row data of its own (that's fetched
/// separately, only for whichever one the user actually expands - see `list_spell_lines_for_spec`/
/// `list_styles_for_spec`). A spec with both styles and spell lines (rare, but possible) shows up
/// in both groups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassSpecSummary {
    pub key_name: String,
    pub name: String,
    pub level_acquired: i64,
    /// Already safe to field-edit without this class triggering a clone first - either this
    /// class's own `_<class_id>`-suffixed spec, or (for a mechanic-locked spec like "Staff" - see
    /// `NEVER_CLONE_SPECS` - whose `Specialization`/`SpellLine` can never be renamed) one whose
    /// combat styles are already locked specifically to this class's own `ClassId`, the same way
    /// the real base game's own data already is for most classes that have always trained it.
    pub is_owned: bool,
    pub has_styles: bool,
    pub has_spell_lines: bool,
}

/// Every spec `class_id` has assigned, each flagged with whether it grants combat styles, spell
/// lines, both, or (rare) neither, and whether it's already safe to edit without this class
/// triggering a clone first - the source list for the "Classes" tab's own "Combat specs"/"Spells"
/// summaries, so the user sees a plain list of names before choosing what to actually expand and
/// edit, rather than every spec's full data dumped on screen at once regardless of what they came
/// here to change.
pub fn list_class_spec_summaries(cfg: &SshConfig, class_id: i64) -> Result<Vec<ClassSpecSummary>, String> {
    let specs = list_class_specs(cfg, class_id)?;
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let keys_csv = specs
        .iter()
        .map(|s| format!("'{}'", sql_quote(&s.spec_key_name)))
        .collect::<Vec<_>>()
        .join(", ");

    let line_keys: std::collections::HashSet<String> =
        run_remote_sql_json(cfg, &format!("SELECT DISTINCT Spec FROM SpellLine WHERE Spec IN ({keys_csv});"))?
            .iter()
            .map(|r| str_field(r, "Spec"))
            .collect();

    // One query covers both "does this spec have any styles at all" and "does this class already
    // have its own locked copies" - no need for a second round trip just to re-check ClassId.
    let style_rows = run_remote_sql_json(
        cfg,
        &format!("SELECT DISTINCT SpecKeyName, ClassId FROM Style WHERE SpecKeyName IN ({keys_csv});"),
    )?;
    let mut style_keys = std::collections::HashSet::new();
    let mut own_style_keys = std::collections::HashSet::new();
    for row in &style_rows {
        let key = str_field(row, "SpecKeyName");
        if int_field(row, "ClassId") == class_id {
            own_style_keys.insert(key.clone());
        }
        style_keys.insert(key);
    }

    let suffix = format!("_{class_id}");
    Ok(specs
        .into_iter()
        .map(|s| {
            let has_styles = style_keys.contains(&s.spec_key_name);
            let has_spell_lines = line_keys.contains(&s.spec_key_name);
            let is_owned = s.spec_key_name.ends_with(&suffix) || own_style_keys.contains(&s.spec_key_name);
            ClassSpecSummary {
                key_name: s.spec_key_name,
                name: s.spec_name,
                level_acquired: s.level_acquired,
                is_owned,
                has_styles,
                has_spell_lines,
            }
        })
        .collect())
}

/// A reasonable, unique-enough DB key from whatever the user named a brand-new spec - mirrors
/// `ClonePanel.tsx`'s own `deriveSpecKey` exactly (strip to plain alphanumerics, lowercase), so a
/// from-scratch spec's key reads the same way a cloned one's does.
fn derive_spec_key(name: &str) -> String {
    let slug: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    if slug.is_empty() { "spec".to_string() } else { slug }
}

/// `derive_spec_key(name)` plus this class's own `_<class_id>` suffix - the same ownership
/// convention every cloned spec's own key already follows (see `ClassSpecSummary::is_owned`'s own
/// doc comment), so a from-scratch spec is recognized as already independent the instant it's
/// created, with no separate "is this mine" bookkeeping needed. Falls back to appending a small
/// numeric suffix of its own if that exact key is somehow already taken (e.g. two new specs
/// created back-to-back with the same name) - checked against the real table, not guessed.
fn unique_spec_key(cfg: &SshConfig, name: &str, class_id: i64) -> Result<String, String> {
    let base = derive_spec_key(name);
    let mut attempt = 1;
    loop {
        let candidate = if attempt == 1 {
            format!("{base}_{class_id}")
        } else {
            format!("{base}{attempt}_{class_id}")
        };
        let candidate_q = sql_quote(&candidate);
        let existing = run_remote_sql_json(
            cfg,
            &format!("SELECT 1 FROM Specialization WHERE KeyName = '{candidate_q}';"),
        )?;
        if existing.is_empty() {
            return Ok(candidate);
        }
        attempt += 1;
    }
}

/// Creates a brand-new `Specialization` (not cloned from anything) for `class_id`, with one
/// placeholder `Style` already under it so it's immediately visible in the "Combat specs"
/// summary and immediately editable via the existing style field editor - there is no separate
/// "empty spec" UI state to build, just a spec that already has exactly the one thing the Combat
/// specs list actually keys its classification on (`ClassSpecSummary::has_styles`).
///
/// The placeholder style is cloned from a real existing row (via `clone_style_with_fresh_id`,
/// the same primitive `clone_spec` itself uses) rather than hand-built from a literal column
/// list - `Style` has almost twenty columns this tool otherwise never needs to know the exact
/// constraints of, and cloning a real row guarantees every one of them starts out populated with
/// some value SQLite already accepted once before. The template is deliberately chosen with
/// `AttackResultRequirement = 0` ("Any attack result" - see `eAttackResultRequirement.Any` in the
/// real server source) specifically to avoid seeding a chain style (`AttackResultRequirement = 7`,
/// "a style is required") - `clone_spec`'s own chain-repoint pass only works because it clones a
/// whole spec's styles at once and can search among them for the matching new prerequisite; a
/// single placeholder style here has no sibling to repoint at, so a cloned chain requirement
/// would dangle, pointing at a `Style.ID` that doesn't exist anywhere in this new spec.
pub fn create_new_combat_spec(
    cfg: &SshConfig,
    class_id: i64,
    name: &str,
    level_acquired: i64,
) -> Result<ClassSpecSummary, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a new combat spec needs a name".to_string());
    }
    let final_key = unique_spec_key(cfg, trimmed, class_id)?;
    let final_key_q = sql_quote(&final_key);
    let name_q = sql_quote(trimmed);

    run_remote_sql(
        cfg,
        &format!(
            "INSERT INTO Specialization (KeyName, Name, Icon, Description, Implementation) \
             VALUES ('{final_key_q}', '{name_q}', 0, '', NULL);\n\
             INSERT INTO ClassXSpecialization (ClassID, SpecKeyName, LevelAcquired) \
             VALUES ({class_id}, '{final_key_q}', {level_acquired});"
        ),
    )?;

    let template_rows = run_remote_sql_json(
        cfg,
        "SELECT StyleID FROM Style WHERE AttackResultRequirement = 0 ORDER BY StyleID LIMIT 1;",
    )?;
    let template_style_id = template_rows.first().map(|r| int_field(r, "StyleID")).ok_or_else(|| {
        "could not find any existing style to seed this new spec's first style from".to_string()
    })?;
    let new_style_id =
        clone_style_with_fresh_id(cfg, template_style_id, &final_key_q, &class_id.to_string())?;
    run_remote_sql(
        cfg,
        &format!(
            "UPDATE Style SET Name = 'New Style', SpecLevelRequirement = 1 WHERE StyleID = {new_style_id};"
        ),
    )?;

    Ok(ClassSpecSummary {
        key_name: final_key,
        name: trimmed.to_string(),
        level_acquired,
        is_owned: true,
        has_styles: true,
        has_spell_lines: false,
    })
}

/// Adds one more combat style to an existing combat spec `spec_key` already grants - the "Add
/// style" counterpart to `CombatSpecEditor`'s per-style field/level editors, which (like
/// `SpellSpecEditor`'s own, see `add_spell_to_line`'s doc comment for the full reported gap) had
/// no way at all to grow a spec past whatever styles it already had.
///
/// Ensures this class actually owns (or, for a `NEVER_CLONE_SPECS` spec, has been granted its own
/// locked copies of) this spec's styles first, exactly like `ensure_style_owned` does before any
/// edit - see its own doc comment for why a shared/stock/universal spec can't just be appended to
/// directly. The new style is cloned (via `clone_style_with_fresh_id`, the same primitive
/// `create_new_combat_spec` and `clone_spec` both already use) from whichever of this class's own
/// styles under this spec currently sits at the *highest* `SpecLevelRequirement` - a new style is
/// overwhelmingly more likely to want broadly similar fields (WeaponTypeRequirement,
/// AttackResultRequirement, EnduranceCost shape) to the one before it than to some unrelated
/// style - falling back to whatever style exists under this spec at all (regardless of owner) if
/// this class doesn't have one of its own yet to copy from.
///
/// `level` defaults to one above this class's own current highest `SpecLevelRequirement` under
/// this spec if not given explicitly.
pub fn add_style_to_spec(
    cfg: &SshConfig,
    class_id: i64,
    spec_key: &str,
    level: Option<i64>,
) -> Result<StyleInfo, String> {
    let owning_key = if let Some(real_name) = resolve_uncloneable_spec(spec_key, class_id) {
        grant_styles_for_uncloneable_spec(cfg, real_name, class_id)?;
        real_name.to_string()
    } else {
        ensure_class_owns_spec(cfg, class_id, spec_key)?
    };
    let owning_key_q = sql_quote(&owning_key);

    let own_rows = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT StyleID, SpecLevelRequirement FROM Style WHERE SpecKeyName = '{owning_key_q}' \
             AND ClassId = {class_id} ORDER BY SpecLevelRequirement DESC LIMIT 1;"
        ),
    )?;
    let (template_style_id, new_level) = if let Some(top) = own_rows.first() {
        (int_field(top, "StyleID"), level.unwrap_or(int_field(top, "SpecLevelRequirement") + 1))
    } else {
        let any_rows = run_remote_sql_json(
            cfg,
            &format!(
                "SELECT StyleID FROM Style WHERE SpecKeyName = '{owning_key_q}' \
                 ORDER BY SpecLevelRequirement DESC LIMIT 1;"
            ),
        )?;
        let id = any_rows.first().map(|r| int_field(r, "StyleID")).ok_or_else(|| {
            format!("no existing style found under '{owning_key}' to seed a new one from")
        })?;
        (id, level.unwrap_or(1))
    };

    let new_style_id =
        clone_style_with_fresh_id(cfg, template_style_id, &owning_key_q, &class_id.to_string())?;
    run_remote_sql(
        cfg,
        &format!("UPDATE Style SET Name = 'New Style', SpecLevelRequirement = {new_level} WHERE StyleID = {new_style_id};"),
    )?;

    let rows = run_remote_sql_json(cfg, &format!("SELECT * FROM Style WHERE StyleID = {new_style_id};"))?;
    let fields = rows.into_iter().next().ok_or_else(|| {
        format!("could not re-read newly cloned style {new_style_id} - it may no longer exist")
    })?;
    Ok(StyleInfo {
        style_id: new_style_id,
        is_owned: true,
        fields,
    })
}

/// Creates a brand-new `Specialization` plus one `SpellLine` under it (not cloned from anything)
/// for `class_id`, with one placeholder `Spell` already on that line so it's immediately visible
/// in the "Spells" summary and immediately editable via the existing spell field editor - the
/// spell-line equivalent of `create_new_combat_spec`, same reasoning throughout.
///
/// The placeholder spell is cloned via `SpellCloner` (the same primitive `clone_spec` itself
/// uses for every spell it clones) from the very first real `Spell` row on the server, picked
/// purely because it's guaranteed to exist and already satisfies the schema's real constraints
/// whatever they are - its own Name/Type/Damage/etc. are expected to be immediately rewritten via
/// the existing spell field editor into whatever the new spell is actually meant to be.
pub fn create_new_spell_spec(
    cfg: &SshConfig,
    class_id: i64,
    name: &str,
    level_acquired: i64,
) -> Result<ClassSpecSummary, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("a new spell line needs a name".to_string());
    }
    let final_key = unique_spec_key(cfg, trimmed, class_id)?;
    let final_key_q = sql_quote(&final_key);
    let name_q = sql_quote(trimmed);
    let line_key = format!("{final_key}_line");
    let line_key_q = sql_quote(&line_key);

    run_remote_sql(
        cfg,
        &format!(
            "INSERT INTO Specialization (KeyName, Name, Icon, Description, Implementation) \
             VALUES ('{final_key_q}', '{name_q}', 0, '', NULL);\n\
             INSERT INTO ClassXSpecialization (ClassID, SpecKeyName, LevelAcquired) \
             VALUES ({class_id}, '{final_key_q}', {level_acquired});\n\
             INSERT INTO SpellLine (KeyName, Name, Spec, IsBaseLine, ClassIDHint) \
             VALUES ('{line_key_q}', '{name_q}', '{final_key_q}', 0, {class_id});"
        ),
    )?;

    let template_rows =
        run_remote_sql_json(cfg, "SELECT SpellID FROM Spell ORDER BY SpellID LIMIT 1;")?;
    let template_spell_id = template_rows.first().map(|r| int_field(r, "SpellID")).ok_or_else(|| {
        "could not find any existing spell to seed this new spell line's first spell from".to_string()
    })?;
    let mut cloner = SpellCloner::new(cfg, class_id)?;
    let new_spell_id = cloner.clone_for_class(cfg, template_spell_id)?;
    run_remote_sql(
        cfg,
        &format!(
            "UPDATE Spell SET Name = 'New Spell' WHERE SpellID = {new_spell_id};\n\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) \
             VALUES ('{line_key_q}', {new_spell_id}, 1, '{}');",
            Uuid::new_v4()
        ),
    )?;

    Ok(ClassSpecSummary {
        key_name: final_key,
        name: trimmed.to_string(),
        level_acquired,
        is_owned: true,
        has_styles: false,
        has_spell_lines: true,
    })
}

/// Adds one more spell to an existing spell line `line_key` under `spec_key` - the "Add spell"
/// counterpart to `SpellSpecEditor`'s per-spell field/level editors, which, until now, had no way
/// at all to grow a spell line past whatever spells `create_new_spell_spec` (or an earlier clone)
/// already seeded it with: a real, reported gap - building out a multi-rank progression (e.g. a
/// "Brisk Movement" / "Greater Brisk Movement" / ... SpeedEnhancement line) meant editing the
/// line's one existing spell over and over, with nowhere to actually add a second, third, ...
/// rank at all.
///
/// Ensures the owning spec (and therefore this line) is independently owned by `class_id` first,
/// exactly like `ensure_line_owned` does before any edit to an existing spell - see its own doc
/// comment for why a shared/stock line can't just be appended to directly, and for the
/// `"{new_spec_key}_{old_line_key}"` naming its own clone of this line always ends up with.
///
/// The new spell is cloned (via `SpellCloner`, the same primitive every other spell clone in this
/// tool already uses) from whichever spell on this line currently sits at the *highest* `Level` -
/// a new rank in a progression is overwhelmingly more likely to want broadly similar fields
/// (Type, Target, flavor-text shape) to the rank right before it than to some unrelated spell,
/// leaving exactly the fields that actually matter for a new rank (Value/Damage/Duration/Level)
/// to tune from there. Its Name is reset to "New Spell" (same placeholder
/// `create_new_spell_spec` already uses), the one field deliberately *not* carried over from the
/// template - otherwise two different ranks of the same progression would default to showing an
/// identical name in the spellbook until manually renamed. Falls back to the same "very first
/// real `Spell` row" template `create_new_spell_spec` uses if the line is (unexpectedly)
/// completely empty.
///
/// `level` defaults to one above the line's own current highest level if not given explicitly.
pub fn add_spell_to_line(
    cfg: &SshConfig,
    class_id: i64,
    spec_key: &str,
    line_key: &str,
    level: Option<i64>,
) -> Result<OwnedSpellInLine, String> {
    let owned_spec_key = ensure_class_owns_spec(cfg, class_id, spec_key)?;
    let owned_line_key = if owned_spec_key == spec_key {
        line_key.to_string()
    } else {
        format!("{owned_spec_key}_{line_key}")
    };
    let owned_line_key_q = sql_quote(&owned_line_key);

    let existing = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT SpellID, Level FROM LineXSpell WHERE LineName = '{owned_line_key_q}' \
             ORDER BY Level DESC LIMIT 1;"
        ),
    )?;
    let (template_spell_id, new_level) = if let Some(top) = existing.first() {
        (int_field(top, "SpellID"), level.unwrap_or(int_field(top, "Level") + 1))
    } else {
        let template_rows =
            run_remote_sql_json(cfg, "SELECT SpellID FROM Spell ORDER BY SpellID LIMIT 1;")?;
        let template_spell_id = template_rows.first().map(|r| int_field(r, "SpellID")).ok_or_else(|| {
            "could not find any existing spell to seed this new spell from".to_string()
        })?;
        (template_spell_id, level.unwrap_or(1))
    };

    let mut cloner = SpellCloner::new(cfg, class_id)?;
    let new_spell_id = cloner.clone_for_class(cfg, template_spell_id)?;
    let new_line_xspell_id = Uuid::new_v4().to_string();
    run_remote_sql(
        cfg,
        &format!(
            "UPDATE Spell SET Name = 'New Spell' WHERE SpellID = {new_spell_id};\n\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) \
             VALUES ('{owned_line_key_q}', {new_spell_id}, {new_level}, '{new_line_xspell_id}');"
        ),
    )?;

    let rows = run_remote_sql_json(cfg, &format!("SELECT * FROM Spell WHERE SpellID = {new_spell_id};"))?;
    let fields = rows.into_iter().next().ok_or_else(|| {
        format!("could not re-read newly cloned spell {new_spell_id} - it may no longer exist")
    })?;
    Ok(OwnedSpellInLine {
        line_xspell_id: new_line_xspell_id,
        level: new_level,
        spell_id: new_spell_id,
        is_owned: true,
        fields,
    })
}

/// Renames `line_key`'s own display Name (`SpellLine.Name` - what shows wherever a spell line's
/// own name appears: this editor's own line header, and the Character Planner) - never its
/// `KeyName`, the real, stable identity its own `LineXSpell` rows and `SpellLine.Spec` reference,
/// which must never change. A real, reported gap this fixes: there was no way at all to rename a
/// spell line once created (short of recreating the whole thing from scratch under a new name) -
/// confirmed live with Nosferatu's own "Movement" line, the common one-line-per-spec case, where
/// the line's own name wasn't even shown anywhere in the editor at all, only its owning spec's.
///
/// Ensures the owning spec (and therefore this line) is independently owned by `class_id` first,
/// exactly like `add_spell_to_line` does before adding to it - see `ensure_class_owns_spec`'s own
/// doc comment for why a shared/stock line can't just be renamed directly, and for why this
/// refuses outright (the same way any other edit to one already does) for a `NEVER_CLONE_SPECS`
/// spec's own line, which can never be renamed at all.
pub fn rename_spell_line(
    cfg: &SshConfig,
    class_id: i64,
    spec_key: &str,
    line_key: &str,
    new_name: &str,
) -> Result<String, String> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err("a spell line needs a name".to_string());
    }
    let owned_spec_key = ensure_class_owns_spec(cfg, class_id, spec_key)?;
    let owned_line_key = if owned_spec_key == spec_key {
        line_key.to_string()
    } else {
        format!("{owned_spec_key}_{line_key}")
    };
    let owned_line_key_q = sql_quote(&owned_line_key);
    let name_q = sql_quote(trimmed);
    run_remote_sql(
        cfg,
        &format!("UPDATE SpellLine SET Name = '{name_q}' WHERE KeyName = '{owned_line_key_q}';"),
    )?;

    // When this spec has exactly one spell line - the common case, and the only shape
    // create_new_spell_spec itself ever produces - its own display Name (shown in the "Spells"
    // summary list before a spec is even expanded) and this, its one line's own Name, are meant
    // to stay in sync: confirmed by create_new_spell_spec seeding both identically to start with.
    // A real, reported gap this fixes: renaming only the line left the owning spec's own Name
    // silently stuck at whatever it started as, with no way to tell from the summary list alone
    // that anything had even been renamed at all. A spec with several lines under it (e.g.
    // Smite's own baseline plus spec-gated lines) has no such unambiguous "same name" assumption
    // - only the one line actually being renamed is ever touched there, never silently renaming
    // the whole spec's own name along with just one of several lines under it.
    let owned_spec_key_q = sql_quote(&owned_spec_key);
    let sibling_lines =
        run_remote_sql_json(cfg, &format!("SELECT 1 FROM SpellLine WHERE Spec = '{owned_spec_key_q}';"))?;
    if sibling_lines.len() == 1 {
        run_remote_sql(
            cfg,
            &format!("UPDATE Specialization SET Name = '{name_q}' WHERE KeyName = '{owned_spec_key_q}';"),
        )?;
    }

    Ok(format!("renamed to \"{trimmed}\""))
}


/// Fields worth suggesting type-appropriate defaults for - everything in
/// `ClassSpecsSection.tsx`'s own `EDITABLE_SPELL_FIELD_HINTS` except Name/Type/Description/
/// Message1/Message2, which are spell-specific flavor text with no meaningful "typical value" to
/// suggest at all. A real, reported problem this exists to fix: a brand-new spell is seeded by
/// cloning an unrelated template spell (`create_new_spell_spec`'s own lowest-SpellID template,
/// chosen purely for being safe to clone, not for being a sensible starting point) - its Value,
/// Duration, Range, etc. showed up verbatim as whatever that unrelated template spell happened
/// to have, with nothing suggesting what's actually typical once the user picks a real Type.
const SPELL_TYPE_STAT_FIELDS: &[&str] = &[
    "Damage",
    "DamageType",
    "Value",
    "Range",
    "Radius",
    "Duration",
    "Pulse",
    "Frequency",
    "PulsePower",
    "Power",
    "CastTime",
    "RecastDelay",
    "Icon",
    "ClientEffect",
    "Target",
];

/// Fields where a low value represents a weaker/earlier-rank version of the same effect (a speed
/// buff's own Value, a DoT's own Damage, how long a buff's own Duration lasts, ...). The owner's
/// own explicit preference: a brand-new spell should default to the *lowest* real value in use
/// for its Type - "let the user ramp them up as they add to it" themselves - not an average or
/// most-common value that might already be a mid-to-late-rank number. Every other stat field
/// (DamageType/Pulse/Icon/ClientEffect/Target) is categorical, not a magnitude - "low" has no
/// real meaning there, so the most *common* real value is suggested instead. `PulsePower`
/// behaves like `Power` itself (a cost that scales up with rank) wherever it's actually used at
/// all - meaningless on a spell with `Pulse = 0` (never pulses, so nothing to charge per pulse),
/// but still worth ramping like `Power` on one that does.
const RAMP_FIELDS: &[&str] = &[
    "Damage",
    "Value",
    "Range",
    "Radius",
    "Duration",
    "Frequency",
    "PulsePower",
    "Power",
    "CastTime",
    "RecastDelay",
];

/// One field's own type-appropriate suggestion: `default` is what a brand-new spell of this Type
/// should start with (see `RAMP_FIELDS`'s own doc comment for which rule picks it); `options` is
/// every distinct real value already in use by some spell of this Type, in the same order
/// (ascending for a ramp field, most-common-first otherwise) - meant for a `<datalist>` alongside
/// a plain text input, so picking one of these is a one-click shortcut to staying consistent with
/// the rest of the game, while direct entry still works for a deliberately novel value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpellTypeFieldStat {
    pub default: String,
    pub options: Vec<String>,
}

/// Every stat-worthy field's own typical-value suggestions for `spell_type` - queries every real
/// spell already using this Type, live, rather than hand-maintaining a lookup table that would
/// drift out of sync with the server's own actual content the moment anyone adds a new spell. A
/// field absent entirely from the result (not just an empty `options` list) means no spell of
/// this Type has ever set it to anything at all - nothing to suggest either way.
pub fn spell_type_field_stats(
    cfg: &SshConfig,
    spell_type: &str,
) -> Result<std::collections::HashMap<String, SpellTypeFieldStat>, String> {
    let type_q = sql_quote(spell_type);
    // Only queries whichever of SPELL_TYPE_STAT_FIELDS actually exist as real columns on this
    // server fork's own Spell table, rather than assuming every one of them always does - a
    // real, found gap: adding a field here that any fork's schema happens to be missing (Pulse/
    // PulsePower, confirmed live) broke this whole feature outright with a raw SQL error
    // ("no such column"), for every Type, not just the one a spell with that gap belonged to.
    // Resolved once per call (not hardcoded) the same "rediscover the real schema, don't assume
    // it" way `columns_excluding_pk` already does for cloning.
    let real_columns: std::collections::HashSet<String> =
        run_remote_sql_json(cfg, "PRAGMA table_info(Spell);")?
            .iter()
            .map(|r| str_field(r, "name"))
            .collect();
    let fields: Vec<&str> = SPELL_TYPE_STAT_FIELDS
        .iter()
        .copied()
        .filter(|f| real_columns.contains(*f))
        .collect();
    if fields.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let cols = fields.join(", ");
    let rows = run_remote_sql_json(cfg, &format!("SELECT {cols} FROM Spell WHERE Type = '{type_q}';"))?;

    let mut stats = std::collections::HashMap::new();
    for &field in &fields {
        // Every real value this field actually has across every spell of this Type - null (the
        // column genuinely unset) is the only thing skipped; zero is a real, meaningful value in
        // its own right for several of these fields (e.g. CastTime=0 is "instant cast", a common
        // and significant case, not a placeholder to ignore).
        let raw: Vec<String> = rows
            .iter()
            .filter_map(|r| r.get(field))
            .filter(|v| !v.is_null())
            .map(|v| match v {
                Value::String(s) => s.clone(),
                // sqlite3's own JSON output can render an integer-valued column as a float
                // (e.g. "115.0") - rendered here as a human would expect to see it in a text
                // field ("115"), not with a misleading trailing ".0" that was never actually
                // typed anywhere.
                Value::Number(n) => match n.as_i64() {
                    Some(i) => i.to_string(),
                    None => match n.as_f64() {
                        Some(f) if f.fract() == 0.0 => (f as i64).to_string(),
                        _ => n.to_string(),
                    },
                },
                other => other.to_string(),
            })
            .filter(|s| !s.is_empty())
            .collect();
        if raw.is_empty() {
            continue;
        }

        if RAMP_FIELDS.contains(&field) {
            let mut numeric: Vec<(f64, String)> =
                raw.iter().filter_map(|s| s.parse::<f64>().ok().map(|n| (n, s.clone()))).collect();
            numeric.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            numeric.dedup_by(|a, b| a.0 == b.0);
            if let Some((_, default)) = numeric.first().cloned() {
                stats.insert(
                    field.to_string(),
                    SpellTypeFieldStat { default, options: numeric.into_iter().map(|(_, s)| s).collect() },
                );
            }
        } else {
            let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for v in &raw {
                *counts.entry(v.clone()).or_insert(0) += 1;
            }
            let mut ordered: Vec<(String, usize)> = counts.into_iter().collect();
            // Most frequent first; ties broken by the value itself purely for deterministic
            // output (tests, and a stable dropdown order run to run).
            ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            if let Some((default, _)) = ordered.first().cloned() {
                stats.insert(
                    field.to_string(),
                    SpellTypeFieldStat { default, options: ordered.into_iter().map(|(s, _)| s).collect() },
                );
            }
        }
    }
    Ok(stats)
}

/// Every SpellLine attached to `spec_key` - the source list for the Edit flow's "customize which
/// lines to include" picker (see `SpellLineSelection`'s own doc comment for why this exists).
pub fn list_spec_spell_lines(
    cfg: &SshConfig,
    spec_key: &str,
) -> Result<Vec<SpecSpellLineRow>, String> {
    let spec_key_q = sql_quote(spec_key);
    let rows = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT KeyName, Name, IsBaseLine FROM SpellLine WHERE Spec = '{spec_key_q}' ORDER BY Name;"
        ),
    )?;
    Ok(rows
        .iter()
        .map(|r| SpecSpellLineRow {
            key_name: str_field(r, "KeyName"),
            name: str_field(r, "Name"),
            is_base_line: int_field(r, "IsBaseLine") != 0,
        })
        .collect())
}

/// Every spell in `line_key` (joining `LineXSpell` to `Spell` for a real display name) - the
/// source list for the Edit flow's "remove specific spells from the line we copy over" picker.
pub fn list_line_spells(cfg: &SshConfig, line_key: &str) -> Result<Vec<LineSpellRow>, String> {
    let line_key_q = sql_quote(line_key);
    let sql = format!(
        "SELECT lxs.SpellID AS SpellID, lxs.Level AS Level, s.Name AS Name, s.SpellGroup AS SpellGroup \
         FROM LineXSpell lxs JOIN Spell s ON s.SpellID = lxs.SpellID \
         WHERE lxs.LineName = '{line_key_q}' ORDER BY lxs.Level;"
    );
    let rows = run_remote_sql_json(cfg, &sql)?;
    Ok(rows
        .iter()
        .map(|r| LineSpellRow {
            spell_id: int_field(r, "SpellID"),
            name: str_field(r, "Name"),
            level: int_field(r, "Level"),
            spell_group: int_field(r, "SpellGroup"),
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
        // Some spec keys can never safely be cloned/renamed at all (NEVER_CLONE_SPECS's own doc
        // comment has the full citations) - the server hardcodes their actual mechanic to their
        // exact literal name, so a clone only ever looks right on the character sheet while
        // silently not working in play. Handle this before any of the normal clone machinery
        // below even runs, and move on to the next assignment.
        if let Some(real_name) = resolve_uncloneable_spec(&a.source_key, new_class_id) {
            log.push_str(&ensure_uncloneable_spec_assigned(
                cfg,
                new_class_id,
                real_name,
                &a.source_key,
                &a.new_name,
                a.level_acquired,
            )?);
            continue;
        }

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
        // matching row). There is nothing to safely redo via the normal delete-then-reclone path
        // in this case - it already has everything a previous deploy gave it.
        //
        // A real, reported bug found live after *that* fix shipped: this unconditionally skipped
        // everything, including a spell_line_selection customization the Edit flow's own picker
        // produces - opening an already-deployed spec's "customize spell lines" picker and
        // unchecking a line/spell does not change source_key, new_key, new_name, or
        // level_acquired at all (only spell_line_selection does), so a self-source redeploy with
        // real exclusions to apply was being silently discarded entirely - generating, deploying,
        // even restarting the server did nothing, since this whole branch never even looked at
        // spell_line_selection before returning. The destructive delete-then-reclone path is
        // still never safe here (same reason as before - there's no separate source left to
        // clone from), but a *targeted, in-place* prune of this exact key's own already-existing
        // SpellLine/LineXSpell rows is - see prune_spell_lines_in_place's own doc comment.
        if a.source_key == final_key {
            if a.spell_line_selection.is_empty() {
                log.push_str(&format!(
                    "left '{}' untouched (already its own source - nothing to reclone)\n",
                    final_key
                ));
            } else {
                log.push_str(&prune_spell_lines_in_place(
                    cfg,
                    &final_key,
                    &a.spell_line_selection,
                )?);
            }
            continue;
        }
        delete_existing_clone(cfg, &final_key)?;
        clone_spec(
            cfg,
            &a.source_key,
            &final_key,
            &a.new_name,
            new_class_id,
            &a.spell_line_selection,
        )?;

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
    // A key carrying this exact class's own clone suffix (`_<class_id>`, the same convention
    // `delete_deployed_class`'s cleanup already scopes to) is fully deleted - nothing else ever
    // owns it. A plain/shared key (e.g. "Sword", assigned directly via `NEVER_CLONE_SPECS` with
    // no suffix at all) is handled differently - a real, reproduced bug found live: removing
    // "Sword"/"Axe"/"Hammer"/"Thrown Weapons" from a class's list and redeploying left every one
    // of them still fully offered by the class (still on the character sheet, still in the
    // in-game trainer window) - this loop previously skipped *any* unsuffixed key entirely,
    // reasoning (correctly) that the same KeyName might still be shared by other classes, but
    // wrongly concluding from that there was nothing safe to clean up at all. There is: this
    // class's own `ClassXSpecialization` row for it, and any `Style` rows
    // `grant_styles_for_uncloneable_spec` duplicated specifically for this class (`ClassId =
    // new_class_id`, which by construction can only ever belong to this one class) - never the
    // spec's own shared `Specialization`/`SpellLine`/`SpecXAbility` rows, nor any other class's
    // own locked styles.
    let keep_keys: std::collections::HashSet<String> = assignments
        .iter()
        .map(|a| {
            resolve_uncloneable_spec(&a.source_key, new_class_id)
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("{}_{new_class_id}", a.new_key))
        })
        .collect();
    let suffix = format!("_{new_class_id}");
    for key in previous_keys {
        if keep_keys.contains(key) {
            continue;
        }
        // Must run before delete_existing_clone/unassign_shared_spec_from_class, not after - a
        // real, reproduced crash found live immediately after this removal feature shipped:
        // deleting a spec correctly removes it from Specialization, but a character who already
        // had points in it still references that now-gone KeyName in their own saved
        // SerializedSpecs - SkillBase.GetSpecialization null-derefs on a key with no matching
        // row, crashing login entirely before the character ever reaches the free respec token
        // granted below. A plain/shared key's own Specialization row is never actually deleted
        // here, so that specific crash can't happen for it - but it still needs scrubbing for an
        // entirely separate reason: GamePlayer.HandleCharacterSkills blindly re-adds *every*
        // key in a character's own saved SerializedSpecs on every login, with no regard at all
        // for whether the class's own current ClassXSpecialization still offers it - so a
        // character who already trained "Sword" keeps seeing and using it forever otherwise,
        // regardless of what this class's data says now.
        log.push_str(&scrub_spec_from_characters(cfg, new_class_id, key)?);
        if key.ends_with(&suffix) {
            delete_existing_clone(cfg, key)?;
            log.push_str(&format!(
                "removed '{}' - no longer in this class's spec assignments\n",
                key
            ));
        } else {
            unassign_shared_spec_from_class(cfg, new_class_id, key)?;
            log.push_str(&format!(
                "unassigned '{}' from this class only - its real Specialization/SpellLine/Style \
                 data (shared by other classes) was left untouched\n",
                key
            ));
        }
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

/// One-time conversion for a class that was cloned *before* `clone_spec` started independently
/// cloning its own spells (see this module's own doc comment, and `spell_ownership.rs`) - finds
/// every spell this class's own cloned specs reference that still isn't an independent clone,
/// and converts it to one in place (new `Spell` row, `LineXSpell`/`StyleXSpell` repointed at it),
/// without touching the Specialization/SpellLine/Style rows themselves at all. Idempotent: a
/// spell already independently owned by *any* class (this one or otherwise - once a `SpellID` is
/// a tracked clone at all, it's definitionally not shared with anything else) is left alone, so
/// running this again (or against a class that was never affected in the first place) is a safe
/// no-op.
///
/// Only ever touches specs this tool itself cloned specifically for `class_id` - identified the
/// same way `delete_deployed_class`'s own cleanup already does, via the `_<class_id>` suffix
/// convention `apply_class_specs` gives every clone's own KeyName. A plain/shared spec assigned
/// directly (e.g. "Sword", via `NEVER_CLONE_SPECS`) is never migrated - its SpellLines, if any,
/// are still genuinely shared with whatever else assigns it directly, and always will be.
///
/// `on_progress(processed, total)` is called once up front (processed=0, so a caller knows the
/// real total immediately, even before anything's actually done) and again after every spell/proc
/// actually cloned - this can genuinely take a while (dozens of slow round-trips for a class with
/// many spec lines), and a real, reported problem this exists for is a UI with no visible
/// progress feedback inviting an impatient double-click on the same button, which used to be
/// able to race (see `SpellCloner::clone_for_class`'s own doc comment - now fixed regardless, but
/// visible progress is what actually stops someone from clicking twice in the first place).
pub fn migrate_class_to_owned_spells(
    cfg: &SshConfig,
    class_id: i64,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<String, String> {
    // Must exist before the "already independently owned?" checks below ever query it - a fresh
    // server/first-ever migration on this DB won't have it yet.
    ensure_tracking_table(cfg)?;

    let suffix = format!("_{class_id}");
    let assigned = run_remote_sql_json(
        cfg,
        &format!("SELECT SpecKeyName FROM ClassXSpecialization WHERE ClassID = {class_id};"),
    )?;
    let owned_keys: Vec<String> = assigned
        .iter()
        .map(|r| str_field(r, "SpecKeyName"))
        .filter(|k| k.ends_with(&suffix))
        .collect();
    if owned_keys.is_empty() {
        return Ok("migrated 0 spell(s) and 0 proc(s), across 0 owned spec(s), to independent, \
                    editable copies\n"
            .to_string());
    }

    // Whatever StyleXSpell's own real GUID-style primary key column is called (see
    // `style_x_spell_guid_column`'s own doc comment), resolved once up front - falls back to
    // matching a row by its full (StyleID, SpellID) pair if this table's own PK turns out to be
    // a plain autoincrement rowid instead (no fork has been seen to actually do that, but this
    // stays correct either way rather than assuming).
    let style_x_spell_pk = style_x_spell_guid_column(cfg)?;

    // First pass: gather every row that actually still needs migrating (already-independent
    // ones are skipped here, not just silently no-op'd later), so the real total is known before
    // any work starts - what a progress indicator actually needs to be meaningful.
    //
    // A real, reproduced performance problem this used to have: every single spell's own
    // "already independently owned?" check (plus every spec's own SpellLine lookup, and every
    // line's own LineXSpell lookup) was its own individual round-trip - for a class with ~100
    // spells across a few lines, that's ~100+ separate SSH connections just to confirm there was
    // nothing left to do, each paying real connection-setup overhead on top of the query itself.
    // Re-running Migrate on an already-fully-migrated class (a safe, idempotent no-op by design)
    // could feel like a hang for a minute or more, with zero visible progress the whole time
    // (the progress bar only has something to report once this whole gathering pass finishes).
    // Every one of these lookups is now a single batched `IN (...)` query instead - one round
    // trip per *kind* of lookup, not one per row.
    struct LineWork {
        line_xspell_id: String,
        old_spell_id: i64,
    }
    struct ProcWork {
        row_key: String,
        old_spell_id: i64,
    }

    let keys_csv = owned_keys
        .iter()
        .map(|k| format!("'{}'", sql_quote(k)))
        .collect::<Vec<_>>()
        .join(", ");
    let spell_lines = run_remote_sql_json(
        cfg,
        &format!("SELECT KeyName FROM SpellLine WHERE Spec IN ({keys_csv});"),
    )?;
    let line_keys: Vec<String> = spell_lines.iter().map(|l| str_field(l, "KeyName")).collect();

    let mut line_rows: Vec<Value> = Vec::new();
    if !line_keys.is_empty() {
        let line_keys_csv = line_keys
            .iter()
            .map(|k| format!("'{}'", sql_quote(k)))
            .collect::<Vec<_>>()
            .join(", ");
        line_rows = run_remote_sql_json(
            cfg,
            &format!(
                "SELECT LineXSpell_ID, SpellID FROM LineXSpell WHERE LineName IN ({line_keys_csv});"
            ),
        )?;
    }

    let style_rows = run_remote_sql_json(
        cfg,
        &format!("SELECT StyleID FROM Style WHERE SpecKeyName IN ({keys_csv});"),
    )?;
    let style_ids: Vec<i64> = style_rows.iter().map(|r| int_field(r, "StyleID")).collect();

    let mut proc_rows: Vec<Value> = Vec::new();
    if !style_ids.is_empty() {
        let style_ids_csv = style_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let select_cols = match &style_x_spell_pk {
            Some(col) => format!("{col} AS RowKey, SpellID"),
            None => "StyleID AS RowKey, SpellID".to_string(),
        };
        proc_rows = run_remote_sql_json(
            cfg,
            &format!("SELECT {select_cols} FROM StyleXSpell WHERE StyleID IN ({style_ids_csv});"),
        )?;
    }

    // One batched ownership check covering every distinct spell either kind of row references,
    // instead of one check per row.
    let mut candidate_ids: Vec<i64> = line_rows.iter().map(|r| int_field(r, "SpellID")).collect();
    candidate_ids.extend(proc_rows.iter().map(|r| int_field(r, "SpellID")));
    candidate_ids.sort_unstable();
    candidate_ids.dedup();
    let already_owned: std::collections::HashSet<i64> = if candidate_ids.is_empty() {
        Default::default()
    } else {
        let ids_csv = candidate_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        run_remote_sql_json(
            cfg,
            &format!("SELECT NewSpellID FROM ClassEditorSpellClone WHERE NewSpellID IN ({ids_csv});"),
        )?
        .iter()
        .map(|r| int_field(r, "NewSpellID"))
        .collect()
    };

    let line_work: Vec<LineWork> = line_rows
        .iter()
        .filter(|r| !already_owned.contains(&int_field(r, "SpellID")))
        .map(|r| LineWork {
            line_xspell_id: str_field(r, "LineXSpell_ID"),
            old_spell_id: int_field(r, "SpellID"),
        })
        .collect();
    let proc_work: Vec<ProcWork> = proc_rows
        .iter()
        .filter(|r| !already_owned.contains(&int_field(r, "SpellID")))
        .map(|r| ProcWork {
            row_key: if style_x_spell_pk.is_some() {
                str_field(r, "RowKey")
            } else {
                int_field(r, "RowKey").to_string()
            },
            old_spell_id: int_field(r, "SpellID"),
        })
        .collect();

    let total = line_work.len() + proc_work.len();
    let mut processed = 0usize;
    on_progress(processed, total);

    let mut cloner = SpellCloner::new(cfg, class_id)?;
    let mut batch = String::new();

    for item in &line_work {
        let new_spell_id = cloner.clone_for_class(cfg, item.old_spell_id)?;
        let row_id_q = sql_quote(&item.line_xspell_id);
        batch.push_str(&format!(
            "UPDATE LineXSpell SET SpellID = {new_spell_id} WHERE LineXSpell_ID = '{row_id_q}';\n"
        ));
        processed += 1;
        on_progress(processed, total);
    }

    for item in &proc_work {
        let new_spell_id = cloner.clone_for_class(cfg, item.old_spell_id)?;
        let where_clause = match &style_x_spell_pk {
            Some(col) => format!("{col} = '{}'", sql_quote(&item.row_key)),
            None => format!(
                "StyleID = {} AND SpellID = {}",
                item.row_key, item.old_spell_id
            ),
        };
        batch.push_str(&format!(
            "UPDATE StyleXSpell SET SpellID = {new_spell_id} WHERE {where_clause};\n"
        ));
        processed += 1;
        on_progress(processed, total);
    }

    if !batch.is_empty() {
        run_remote_sql(cfg, &batch)?;
    }
    Ok(format!(
        "migrated {} spell(s) and {} proc(s), across {} owned spec(s), to independent, editable \
         copies\n",
        line_work.len(),
        proc_work.len(),
        owned_keys.len()
    ))
}

/// One spell's own row on a particular spell line, for the "Classes" tab's own spell-editing
/// section - the level it becomes available at (editable, see `update_line_spell_level`) plus its
/// full `Spell` row (dynamic fields - see `spell_ownership`'s own doc comment for why this is a
/// raw JSON object rather than a fixed struct). `is_owned` tells the UI whether this spell is
/// already an independent clone, or still shared with whatever class/spec it came from - editing
/// it (Save) clones it for this class on the spot either way, so this is purely informational.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedSpellInLine {
    pub line_xspell_id: String,
    pub level: i64,
    pub spell_id: i64,
    pub is_owned: bool,
    pub fields: Value,
}

/// One `SpellLine` belonging to a spec, with every spell on it (in level order) - the real "how
/// does this spec actually progress" shape a flat, line-blind spell list couldn't express at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpellLineGroup {
    pub spec_key: String,
    pub line_key: String,
    pub line_name: String,
    /// True for a line the class gets regardless of spec investment (scales with character
    /// level), false for one that only exists/progresses once points are actually spent on this
    /// spec - see `resolve_spec_lines_for_class`'s own doc comment for the real mechanic this
    /// mirrors. The "Classes" tab and the Character Planner both need this to show the two
    /// kinds separately instead of silently merging them into one list.
    pub is_base_line: bool,
    pub spells: Vec<OwnedSpellInLine>,
}

/// A real, confirmed-live bug found via Character Planner data: several classes share one `Spec`
/// KeyName (e.g. Healer/Shaman/Valkyrie all list "Mending"), and the server's own `SpellLine`
/// table carries *multiple* rows for that one Spec - tagged per class via `ClassIDHint` (0 for a
/// shared/neutral row, a real class ID for one that only applies to that class) and per
/// `IsBaseLine` (a baseline row is granted to every class with this spec regardless of points
/// spent, scaling off character level; a non-baseline row only exists once spec points are
/// actually invested, scaling off spec level). A naive `WHERE Spec = key` query returns every
/// class's rows mixed together - this was showing Valkyrie's own "Mending" spells under Healer,
/// and merging Smite's always-on baseline (Direct Damage/Stun) with its spec-gated lines
/// (Damage Add/Mesmerize/etc.) into one undifferentiated block.
///
/// Mirrors the real server's own resolution exactly (`Specialization.GetSpellLinesForLiving` /
/// `Specialization.SpecLinesForClass`, both in `gameutils/Specialization.cs`): baseline and
/// non-baseline are resolved independently; for each, a class's own `ClassIDHint`-tagged rows
/// are used if any exist, falling back to the neutral (`ClassIDHint = 0`) rows only when it has
/// none of its own. Maulers (class IDs 60-62) are the one documented exception for non-baseline
/// rows: their own lines are *added to* the neutral ones (their realm-gift lines extend the
/// shared Aura Manipulation line), not swapped in for them.
fn resolve_spec_lines_for_class(lines: &[Value], class_id: i64) -> Vec<Value> {
    let is_mauler = (60..=62).contains(&class_id);
    let bucket = |want_base_line: bool| -> Vec<Value> {
        let matches_bucket = |l: &&Value| (int_field(l, "IsBaseLine") != 0) == want_base_line;
        let own: Vec<Value> = lines
            .iter()
            .filter(matches_bucket)
            .filter(|l| int_field(l, "ClassIDHint") == class_id)
            .cloned()
            .collect();
        let neutral: Vec<Value> = lines
            .iter()
            .filter(matches_bucket)
            .filter(|l| int_field(l, "ClassIDHint") == 0)
            .cloned()
            .collect();
        if own.is_empty() {
            neutral
        } else if !want_base_line && is_mauler {
            neutral.into_iter().chain(own).collect()
        } else {
            own
        }
    };
    bucket(true).into_iter().chain(bucket(false)).collect()
}

/// Every spell line that actually applies to `class_id` for `spec_key` - a class's own cloned
/// spec, or a stock/shared one it's merely been assigned (any class with this spec in its spec
/// list can call this, not just one that's already cloned it) - each with its own spells in level
/// order, baseline lines first. Scoped to one spec at a time (rather than a whole class's worth
/// up front) so expanding just the one spec a user actually wants to look at in the "Classes" tab
/// doesn't also have to fetch every other spec's own data nobody asked to see yet. See
/// `resolve_spec_lines_for_class` for why `class_id` is required at all, not just `spec_key`.
pub fn list_spell_lines_for_spec(
    cfg: &SshConfig,
    class_id: i64,
    spec_key: &str,
) -> Result<Vec<SpellLineGroup>, String> {
    ensure_tracking_table(cfg)?;

    let spec_key_q = sql_quote(spec_key);
    let all_lines = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT KeyName, Name, Spec, IsBaseLine, ClassIDHint FROM SpellLine WHERE Spec = '{spec_key_q}';"
        ),
    )?;
    let lines = resolve_spec_lines_for_class(&all_lines, class_id);
    let mut groups: Vec<SpellLineGroup> = lines
        .iter()
        .map(|l| SpellLineGroup {
            spec_key: str_field(l, "Spec"),
            line_key: str_field(l, "KeyName"),
            line_name: str_field(l, "Name"),
            is_base_line: int_field(l, "IsBaseLine") != 0,
            spells: Vec::new(),
        })
        .collect();
    if groups.is_empty() {
        return Ok(groups);
    }


    let line_keys_csv = groups
        .iter()
        .map(|g| format!("'{}'", sql_quote(&g.line_key)))
        .collect::<Vec<_>>()
        .join(", ");
    // `Spell.*` is deliberately the generic, dynamic field set (see `OwnedSpellInLine`'s own doc
    // comment); `LineXSpell`'s own `LineName`/`LineXSpell_ID`/`Level` are aliased so they don't
    // collide with `Spell`'s own identically-named `SpellID` column in the combined JSON row.
    let rows = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT LineXSpell.LineName AS OwnerLineKey, LineXSpell.LineXSpell_ID AS OwnerLineXSpellId, \
             LineXSpell.Level AS OwnerLevel, Spell.* \
             FROM LineXSpell JOIN Spell ON Spell.SpellID = LineXSpell.SpellID \
             WHERE LineXSpell.LineName IN ({line_keys_csv}) ORDER BY LineXSpell.Level;"
        ),
    )?;

    let spell_ids: Vec<i64> = rows.iter().map(|r| int_field(r, "SpellID")).collect();
    let owned_ids: std::collections::HashSet<i64> = if spell_ids.is_empty() {
        Default::default()
    } else {
        let ids_csv = spell_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        run_remote_sql_json(
            cfg,
            &format!("SELECT NewSpellID FROM ClassEditorSpellClone WHERE NewSpellID IN ({ids_csv});"),
        )?
        .iter()
        .map(|r| int_field(r, "NewSpellID"))
        .collect()
    };

    for row in &rows {
        let line_key = str_field(row, "OwnerLineKey");
        if let Some(group) = groups.iter_mut().find(|g| g.line_key == line_key) {
            let spell_id = int_field(row, "SpellID");
            group.spells.push(OwnedSpellInLine {
                line_xspell_id: str_field(row, "OwnerLineXSpellId"),
                level: int_field(row, "OwnerLevel"),
                spell_id,
                is_owned: owned_ids.contains(&spell_id),
                fields: row.clone(),
            });
        }
    }
    Ok(groups)
}

/// Promotes a spec `class_id` currently has assigned under a plain/shared key (e.g. a stock
/// class's own real "Rejuvenation") into its own independent `_<class_id>`-suffixed clone -
/// `clone_spec` under the hood, so everything it owns (SpellLine(s)/LineXSpell, Style,
/// SpecXAbility, and every spell/proc) comes along fully independent in one step, exactly like a
/// spec assigned through the clone wizard already does. Only this one spec's own
/// `ClassXSpecialization` row is touched - unlike `apply_class_specs` (which always resubmits a
/// class's *entire* spec list), this never risks cloning or touching any of the class's other,
/// unrelated specs just because one of them was edited.
///
/// A no-op, returning `spec_key` unchanged, if it's already this class's own clone. Refuses
/// outright for a mechanic-locked spec (`NEVER_CLONE_SPECS`, e.g. "Staff") - its `Specialization`/
/// `SpellLine` can never safely be renamed at all (see that list's own doc comment); only its
/// combat styles can ever be made class-specific, via `ensure_style_owned` instead.
fn ensure_class_owns_spec(cfg: &SshConfig, class_id: i64, spec_key: &str) -> Result<String, String> {
    if resolve_uncloneable_spec(spec_key, class_id).is_some() {
        return Err(format!(
            "'{spec_key}' is one of this server's mechanic-locked specs - its own Specialization/\
             SpellLine can't be independently cloned or renamed (see NEVER_CLONE_SPECS); only its \
             combat styles can ever be made class-specific"
        ));
    }
    let suffix = format!("_{class_id}");
    if spec_key.ends_with(&suffix) {
        return Ok(spec_key.to_string());
    }

    let spec_key_q = sql_quote(spec_key);
    let rows = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT s.Name AS Name, cxs.LevelAcquired AS LevelAcquired FROM ClassXSpecialization cxs \
             JOIN Specialization s ON s.KeyName = cxs.SpecKeyName \
             WHERE cxs.ClassID = {class_id} AND cxs.SpecKeyName = '{spec_key_q}';"
        ),
    )?;
    let row = rows
        .first()
        .ok_or_else(|| format!("class {class_id} does not have '{spec_key}' assigned"))?;
    let name = str_field(row, "Name");

    let new_key = format!("{spec_key}_{class_id}");
    delete_existing_clone(cfg, &new_key)?;
    clone_spec(cfg, spec_key, &new_key, &name, class_id, &[])?;

    let new_key_q = sql_quote(&new_key);
    run_remote_sql(
        cfg,
        &format!(
            "UPDATE ClassXSpecialization SET SpecKeyName = '{new_key_q}' \
             WHERE ClassID = {class_id} AND SpecKeyName = '{spec_key_q}';"
        ),
    )?;
    Ok(new_key)
}

/// Returns (new `LineXSpell_ID`, new `SpellID`) for whatever `line_xspell_id` now maps to, after
/// ensuring both the spec it belongs to and this one spell within it are independently owned by
/// `class_id` - a no-op (echoes the same IDs straight back) if everything already was. This is
/// the shared "clone on first touch" building block both field-editing (Save) and level-editing
/// use, so neither needs a separate step run first, for a custom class's own spec *or* a stock
/// class's real, previously-shared one alike (e.g. a class never cloned through this tool at all,
/// like Friar's own real "Rejuvenation").
///
/// When the owning spec isn't owned yet, promoting it (`ensure_class_owns_spec`) deep-clones its
/// whole SpellLine/LineXSpell set under a deterministic new line key
/// (`"{new_spec_key}_{old_line_key}"` - exactly what `clone_spec` itself always names a cloned
/// line), so the spell's own new identity can be found again by its unchanged `Level` within that
/// new line, with no fuzzy matching needed.
fn ensure_line_owned(cfg: &SshConfig, class_id: i64, line_xspell_id: &str) -> Result<(String, i64), String> {
    let line_id_q = sql_quote(line_xspell_id);
    let owning = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT SpellLine.Spec AS OwnerSpec, SpellLine.KeyName AS OwnerLineKey, \
             LineXSpell.Level AS OwnerLevel, LineXSpell.SpellID AS CurrentSpellID \
             FROM LineXSpell JOIN SpellLine ON SpellLine.KeyName = LineXSpell.LineName \
             WHERE LineXSpell.LineXSpell_ID = '{line_id_q}';"
        ),
    )?;
    let row = owning.first().ok_or_else(|| {
        format!("no LineXSpell row found for '{line_xspell_id}' - it may have been removed")
    })?;
    let spec_key = str_field(row, "OwnerSpec");
    let old_line_key = str_field(row, "OwnerLineKey");
    let level = int_field(row, "OwnerLevel");
    let current_spell_id = int_field(row, "CurrentSpellID");

    let suffix = format!("_{class_id}");
    if spec_key.ends_with(&suffix) {
        // The spec itself is already owned - only the individual spell might still need its own
        // clone (e.g. a class cloned before independent spell cloning existed).
        ensure_tracking_table(cfg)?;
        let already_owned = run_remote_sql_json(
            cfg,
            &format!("SELECT 1 FROM ClassEditorSpellClone WHERE NewSpellID = {current_spell_id};"),
        )?;
        if !already_owned.is_empty() {
            return Ok((line_xspell_id.to_string(), current_spell_id));
        }
        let mut cloner = SpellCloner::new(cfg, class_id)?;
        let new_spell_id = cloner.clone_for_class(cfg, current_spell_id)?;
        run_remote_sql(
            cfg,
            &format!(
                "UPDATE LineXSpell SET SpellID = {new_spell_id} WHERE LineXSpell_ID = '{line_id_q}';"
            ),
        )?;
        return Ok((line_xspell_id.to_string(), new_spell_id));
    }

    let new_key = ensure_class_owns_spec(cfg, class_id, &spec_key)?;
    let new_line_key = format!("{new_key}_{old_line_key}");
    let new_line_key_q = sql_quote(&new_line_key);
    let new_rows = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT LineXSpell_ID, SpellID FROM LineXSpell WHERE LineName = '{new_line_key_q}' AND Level = {level};"
        ),
    )?;
    let new_row = new_rows.first().ok_or_else(|| {
        "could not find this class's own copy of this spell line after cloning its spec".to_string()
    })?;
    Ok((str_field(new_row, "LineXSpell_ID"), int_field(new_row, "SpellID")))
}

/// Changes the level a spell line's own spell becomes available at (`LineXSpell.Level`), cloning
/// the owning spec (and this spell) for this class first if either isn't independently owned yet
/// - see `ensure_line_owned`'s own doc comment.
pub fn update_line_spell_level(
    cfg: &SshConfig,
    class_id: i64,
    line_xspell_id: &str,
    new_level: i64,
) -> Result<String, String> {
    let (owned_line_id, _) = ensure_line_owned(cfg, class_id, line_xspell_id)?;
    let owned_line_id_q = sql_quote(&owned_line_id);
    run_remote_sql(
        cfg,
        &format!("UPDATE LineXSpell SET Level = {new_level} WHERE LineXSpell_ID = '{owned_line_id_q}';"),
    )?;
    Ok(format!("updated level to {new_level}"))
}

/// Returns `class_id`'s own independent clone of whatever `Spell` row `line_xspell_id` currently
/// points at, cloning it (and, if needed, its whole owning spec too) on the spot if it isn't an
/// independent copy already - a no-op, returning the same ID unchanged, if it already is one. See
/// `ensure_line_owned`'s own doc comment for exactly how.
///
/// This is what lets field-editing a spell "just work" straight from the editing UI's own Save
/// button, one spell at a time, the moment it's actually touched - instead of requiring a
/// separate, explicit, whole-class `migrate_class_to_owned_spells` pass (or, for a stock class
/// that was never cloned through this tool at all, no path to independent editing whatsoever) run
/// first.
pub fn ensure_line_spell_owned(cfg: &SshConfig, class_id: i64, line_xspell_id: &str) -> Result<i64, String> {
    let (_, spell_id) = ensure_line_owned(cfg, class_id, line_xspell_id)?;
    Ok(spell_id)
}

/// One `Style` (combat skill) row for the "Classes" tab's own combat-spec editing section -
/// mirrors `OwnedSpellInLine`'s shape/reasoning (dynamic fields, an informational `is_owned`
/// flag - editing always clones/grants on Save regardless).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleInfo {
    pub style_id: i64,
    pub is_owned: bool,
    pub fields: Value,
}

/// Every combat style `spec_key` currently grants that's actually visible to `class_id`, mirroring
/// the real server's own `Specialization.GetStylesForLiving` lookup priority (see
/// `grant_styles_for_uncloneable_spec`'s own doc comment for the citation): this class's own
/// `ClassId`-locked copies if it has any, else the spec's `ClassId = 0` universal bucket if one
/// exists, else - purely for display, so there's still something to look at and a basis to clone
/// from - whatever styles exist under this spec at all (collapsed to one row per logical style,
/// same `GROUP BY Name, SpecLevelRequirement` `grant_styles_for_uncloneable_spec` already uses,
/// since an uncloneable spec's styles otherwise show up once per other real class that already
/// has its own locked copy).
pub fn list_styles_for_spec(cfg: &SshConfig, class_id: i64, spec_key: &str) -> Result<Vec<StyleInfo>, String> {
    let spec_key_q = sql_quote(spec_key);
    let to_infos = |rows: Vec<Value>, is_owned: bool| -> Vec<StyleInfo> {
        rows.into_iter()
            .map(|r| StyleInfo {
                style_id: int_field(&r, "StyleID"),
                is_owned,
                fields: r,
            })
            .collect()
    };

    let own = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT * FROM Style WHERE SpecKeyName = '{spec_key_q}' AND ClassId = {class_id} \
             ORDER BY SpecLevelRequirement;"
        ),
    )?;
    if !own.is_empty() {
        return Ok(to_infos(own, true));
    }

    let universal = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT * FROM Style WHERE SpecKeyName = '{spec_key_q}' AND ClassId = 0 \
             ORDER BY SpecLevelRequirement;"
        ),
    )?;
    if !universal.is_empty() {
        return Ok(to_infos(universal, false));
    }

    let any = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT * FROM Style WHERE SpecKeyName = '{spec_key_q}' GROUP BY Name, SpecLevelRequirement \
             ORDER BY SpecLevelRequirement;"
        ),
    )?;
    Ok(to_infos(any, false))
}

/// Returns `class_id`'s own, exclusively-locked copy of `style_id` - granting/cloning it on the
/// spot if it isn't one already, by whichever mechanism actually applies:
/// - a mechanic-locked spec (`NEVER_CLONE_SPECS`, e.g. "Staff") grants this class its own
///   `ClassId`-locked copies of the spec's styles (`grant_styles_for_uncloneable_spec`) without
///   ever touching the spec/spell-line itself;
/// - any other spec instead promotes the *whole* spec to an independent clone
///   (`ensure_class_owns_spec`), which brings its own already-independent styles along with it.
///
/// Either way, `style_id` itself usually isn't reusable afterward (both paths can create a whole
/// new set of rows under a new identity) - the newly-owned row is re-found by matching this
/// class's own copy of the same `Name`/`SpecLevelRequirement`, the same matching `clone_spec`'s
/// own style-grouping already relies on.
pub fn ensure_style_owned(
    cfg: &SshConfig,
    class_id: i64,
    spec_key: &str,
    style_id: i64,
) -> Result<i64, String> {
    let rows = run_remote_sql_json(
        cfg,
        &format!("SELECT Name, SpecLevelRequirement, ClassId FROM Style WHERE StyleID = {style_id};"),
    )?;
    let row = rows
        .first()
        .ok_or_else(|| format!("style {style_id} not found"))?;
    if int_field(row, "ClassId") == class_id {
        return Ok(style_id);
    }
    let name = str_field(row, "Name");
    let level = int_field(row, "SpecLevelRequirement");

    let owning_key = if let Some(real_name) = resolve_uncloneable_spec(spec_key, class_id) {
        grant_styles_for_uncloneable_spec(cfg, real_name, class_id)?;
        real_name.to_string()
    } else {
        ensure_class_owns_spec(cfg, class_id, spec_key)?
    };

    let owning_key_q = sql_quote(&owning_key);
    let name_q = sql_quote(&name);
    let new_rows = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT StyleID FROM Style WHERE SpecKeyName = '{owning_key_q}' AND ClassId = {class_id} \
             AND Name = '{name_q}' AND SpecLevelRequirement = {level};"
        ),
    )?;
    new_rows
        .first()
        .map(|r| int_field(r, "StyleID"))
        .ok_or_else(|| format!("could not find this class's own copy of '{name}' after cloning/granting its spec"))
}

/// Updates one or more fields on a `Style` row this class already exclusively owns (`ClassId =
/// class_id`) - refuses outright otherwise, the same safety stance `spell_ownership::
/// update_owned_spell_fields` takes for `Spell`: never let the editing UI reach a row still
/// shared with another class, even by mistake. Call `ensure_style_owned` first to get an owned
/// `style_id` to pass in here.
pub fn update_style_fields(
    cfg: &SshConfig,
    class_id: i64,
    style_id: i64,
    fields: &std::collections::HashMap<String, Value>,
) -> Result<String, String> {
    if fields.is_empty() {
        return Ok("nothing to update".to_string());
    }
    let owned = run_remote_sql_json(
        cfg,
        &format!("SELECT 1 FROM Style WHERE StyleID = {style_id} AND ClassId = {class_id};"),
    )?;
    if owned.is_empty() {
        return Err(format!(
            "style {style_id} is not locked specifically to class {class_id} - refusing to edit it \
             directly, since it may still be shared with other classes"
        ));
    }
    let real_columns = columns_excluding_pk(cfg, "Style")?;
    let mut set_clauses = Vec::new();
    for (field, value) in fields {
        if !real_columns.iter().any(|c| c == field) {
            return Err(format!("'{field}' is not a real column on Style - refusing to update it"));
        }
        set_clauses.push(format!("{field} = {}", sql_literal(value)));
    }
    let sql = format!("UPDATE Style SET {} WHERE StyleID = {style_id};", set_clauses.join(", "));
    run_remote_sql(cfg, &sql)?;
    Ok(format!("updated {} field(s) on style {style_id}", fields.len()))
}

/// Changes the level a style becomes available at (`Style.SpecLevelRequirement`), granting/
/// cloning this class its own copy first if it doesn't have one yet - see `ensure_style_owned`'s
/// own doc comment.
pub fn update_style_level(
    cfg: &SshConfig,
    class_id: i64,
    spec_key: &str,
    style_id: i64,
    new_level: i64,
) -> Result<String, String> {
    let owned_style_id = ensure_style_owned(cfg, class_id, spec_key, style_id)?;
    run_remote_sql(
        cfg,
        &format!("UPDATE Style SET SpecLevelRequirement = {new_level} WHERE StyleID = {owned_style_id};"),
    )?;
    Ok(format!("updated level to {new_level}"))
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
    // Gather every spell this clone owns *before* deleting the rows that reference them - once
    // the LineXSpell row is gone there's no way back to which SpellID it used.
    let mut owned_spell_ids: Vec<i64> = Vec::new();
    for line in &lines {
        let line_key_q = sql_quote(&str_field(line, "KeyName"));
        let spells = run_remote_sql_json(
            cfg,
            &format!("SELECT SpellID FROM LineXSpell WHERE LineName = '{line_key_q}';"),
        )?;
        owned_spell_ids.extend(spells.iter().map(|s| int_field(s, "SpellID")));
        run_remote_sql(
            cfg,
            &format!("DELETE FROM LineXSpell WHERE LineName = '{line_key_q}';"),
        )?;
    }
    run_remote_sql(
        cfg,
        &format!("DELETE FROM SpellLine WHERE Spec = '{key_q}';"),
    )?;

    // Same for any styles' own procs - gathered before the styles themselves (and their
    // StyleXSpell rows) are deleted below.
    let style_ids: Vec<i64> = run_remote_sql_json(
        cfg,
        &format!("SELECT StyleID FROM Style WHERE SpecKeyName = '{key_q}';"),
    )?
    .iter()
    .map(|r| int_field(r, "StyleID"))
    .collect();
    delete_style_procs_for_styles(cfg, &style_ids)?;
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
    delete_owned_spells(cfg, &owned_spell_ids)?;
    Ok(())
}

/// Strips a now-deleted spec key from every character currently on `class_id`'s own saved
/// SerializedSpecs - a real, reproduced crash found live immediately after the removal feature
/// above shipped: deleting a spec correctly removes its Specialization row, but a character who
/// already had points in it still carries that now-gone KeyName in their own saved
/// SerializedSpecs string. SkillBase.GetSpecialization null-derefs on a key with no matching row,
/// crashing login entirely (the exact signature from the self-source-deletion bug found earlier
/// in this file) - before the character ever gets anywhere near the free respec token this same
/// deploy grants. SerializedSpecs is a `;`-joined list of `KeyName|points` tokens; this drops only
/// the exact matching token (never a partial/substring match) and writes back only characters
/// whose string actually changed, leaving every other spec's points untouched.
fn scrub_spec_from_characters(cfg: &SshConfig, class_id: i64, key: &str) -> Result<String, String> {
    let rows = run_remote_sql_json(
        cfg,
        &format!("SELECT Name, SerializedSpecs FROM DOLCharacters WHERE Class = {class_id};"),
    )?;
    let mut changed = Vec::new();
    for row in &rows {
        let name = str_field(row, "Name");
        let serialized_specs = str_field(row, "SerializedSpecs");
        if serialized_specs.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = serialized_specs.split(';').collect();
        let kept: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|t| t.split('|').next() != Some(key))
            .collect();
        if kept.len() == tokens.len() {
            continue;
        }
        run_remote_sql(
            cfg,
            &format!(
                "UPDATE DOLCharacters SET SerializedSpecs = '{}' WHERE Name = '{}' AND Class = {class_id};",
                sql_quote(&kept.join(";")),
                sql_quote(&name)
            ),
        )?;
        changed.push(name);
    }
    if changed.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "removed stale '{}' points from {} character(s) so they can still log in: {}\n",
        key,
        changed.len(),
        changed.join(", ")
    ))
}

/// Unassigns a plain/shared spec `key` (one with no class-suffix at all - the convention
/// `NEVER_CLONE_SPECS` assigns under) from exactly `class_id`, and nothing else: its own
/// `ClassXSpecialization` row, and any `Style` rows locked specifically to this class
/// (`ClassId = class_id` - by construction, from `grant_styles_for_uncloneable_spec`, that
/// `ClassId` can only ever be this one class's own duplicate copies, never another class's or the
/// spec's shared originals). Deliberately never touches the spec's own shared
/// `Specialization`/`SpellLine`/`LineXSpell`/`SpecXAbility` rows, nor any other class's own locked
/// `Style` rows - a real, reproduced real-world case this matters for: removing "Sword" from a
/// custom class must never affect the real Warrior/Hunter/etc. who also train it.
fn unassign_shared_spec_from_class(cfg: &SshConfig, class_id: i64, key: &str) -> Result<(), String> {
    let key_q = sql_quote(key);
    run_remote_sql(
        cfg,
        &format!(
            "DELETE FROM ClassXSpecialization WHERE ClassID = {class_id} AND SpecKeyName = '{key_q}';"
        ),
    )?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM Style WHERE SpecKeyName = '{key_q}' AND ClassId = {class_id};"),
    )?;
    Ok(())
}

/// Migrates a character's own trained points from one SpecKeyName to another in their saved
/// SerializedSpecs, for every character currently on `class_id` - renames the token's key but
/// keeps its exact value. Used to heal a spec this tool mistakenly cloned before
/// `NEVER_CLONE_SPECS` existed (e.g. "sword_64") back onto its real, mechanically-functional name
/// ("Sword") without losing whatever a character already invested under the old key - unlike
/// `scrub_spec_from_characters`, nothing is actually being removed from the class here, only its
/// real identity restored.
fn rename_spec_in_characters(
    cfg: &SshConfig,
    class_id: i64,
    from_key: &str,
    to_key: &str,
) -> Result<String, String> {
    let rows = run_remote_sql_json(
        cfg,
        &format!("SELECT Name, SerializedSpecs FROM DOLCharacters WHERE Class = {class_id};"),
    )?;
    let mut changed = Vec::new();
    for row in &rows {
        let name = str_field(row, "Name");
        let serialized_specs = str_field(row, "SerializedSpecs");
        if serialized_specs.is_empty() {
            continue;
        }
        let mut touched = false;
        let tokens: Vec<String> = serialized_specs
            .split(';')
            .map(|t| {
                let mut parts = t.splitn(2, '|');
                let key = parts.next().unwrap_or("");
                let rest = parts.next();
                if key == from_key {
                    touched = true;
                    match rest {
                        Some(r) => format!("{to_key}|{r}"),
                        None => to_key.to_string(),
                    }
                } else {
                    t.to_string()
                }
            })
            .collect();
        if !touched {
            continue;
        }
        run_remote_sql(
            cfg,
            &format!(
                "UPDATE DOLCharacters SET SerializedSpecs = '{}' WHERE Name = '{}' AND Class = {class_id};",
                sql_quote(&tokens.join(";")),
                sql_quote(&name)
            ),
        )?;
        changed.push(name);
    }
    if changed.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "migrated '{}' points to '{}' for {} character(s): {}\n",
        from_key,
        to_key,
        changed.len(),
        changed.join(", ")
    ))
}

/// Moves a stale clone's own Style and SpecXAbility rows onto `real_name` (UPDATE, never DELETE),
/// then removes only the clone's now-empty identity (its Specialization row, any SpellLine/
/// LineXSpell, and its own ClassXSpecialization row). A real, reproduced case this exists for:
/// "sword_64" (a previous clone of "Sword", before `NEVER_CLONE_SPECS` existed) still carried its
/// own genuinely class-exclusive combat styles, and "stealth_64" carried Nosferatu's own
/// SpecXAbility grants (Safe Fall, Climb Walls, Danger Sense, Detect Hidden, Distraction) - those
/// are real, class-specific content worth keeping, locked to the same ClassId either way, so they
/// move onto the real spec's own row (exactly how the real game already lets several classes
/// share one base spec with their own distinct, ClassId-locked style sets) rather than being
/// thrown away just because the spec's own *identity* has to be the real one.
fn reparent_onto_real_spec(
    cfg: &SshConfig,
    stale_key: &str,
    real_name: &str,
) -> Result<(), String> {
    let stale_key_q = sql_quote(stale_key);
    let real_name_q = sql_quote(real_name);

    run_remote_sql(
        cfg,
        &format!(
            "UPDATE Style SET SpecKeyName = '{real_name_q}' WHERE SpecKeyName = '{stale_key_q}';"
        ),
    )?;
    run_remote_sql(
        cfg,
        &format!("UPDATE SpecXAbility SET Spec = '{real_name_q}' WHERE Spec = '{stale_key_q}';"),
    )?;

    let lines = run_remote_sql_json(
        cfg,
        &format!("SELECT KeyName FROM SpellLine WHERE Spec = '{stale_key_q}';"),
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
        &format!("DELETE FROM SpellLine WHERE Spec = '{stale_key_q}';"),
    )?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM ClassXSpecialization WHERE SpecKeyName = '{stale_key_q}';"),
    )?;
    run_remote_sql(
        cfg,
        &format!("DELETE FROM Specialization WHERE KeyName = '{stale_key_q}';"),
    )?;
    Ok(())
}

/// Grants `new_class_id` its own copy of a `NEVER_CLONE_SPECS` spec's already-existing combat
/// styles - a real, reproduced gap found live: a class built by cloning a parent that directly
/// uses real weapon-type specs (e.g. a Viking-descended class inheriting Warrior's own direct
/// Sword/Axe/Hammer/Shields/Spear assignments) trains those specs correctly on the character
/// sheet (the weapon-skill math is resolved generically by exact KeyName, confirmed in
/// `NEVER_CLONE_SPECS`'s own doc comment) but saw *zero* combat styles for any of them in the
/// in-game trainer window. Confirmed in the real server source:
/// `Specialization.GetStylesForLiving` resolves styles by the player's own exact
/// `CharacterClass.ID` (`SkillBase.GetStyleList(KeyName, classId)`), falling back to a
/// `ClassId = 0` "universal" bucket only if the player's own class has zero styles at all - and a
/// brand-new custom class's ID was obviously never one of the handful of real classes a shared
/// weapon spec's styles happen to be locked to, nor does a universal bucket exist for any weapon
/// spec checked live (Spear's styles are locked to Hunter only; Sword/Axe/Hammer/Shields are each
/// locked to a handful of other specific real classes - no `ClassId = 0` fallback exists for any
/// of them).
///
/// Confirmed live, by direct comparison across several real classes that already share one of
/// these specs, that every class's own locked copy of conceptually the same style carries the
/// *exact same* `Style.ID` value (e.g. Sword's "Whirling Blade" is `ID` 146 for both Warrior and
/// Hunter alike) - this is how the real game's own chain-style `OpeningRequirementValue`
/// references already stay correct across every class that shares a spec, with no per-class
/// remapping at all. So this copies every field unchanged except `ClassId` (and the autoincrement
/// `StyleID` itself) - no chain-prerequisite remap is needed here, unlike `clone_spec`'s own
/// (which only needs one because it generates a *new*, not-yet-existing style set under a
/// brand-new `SpecKeyName`).
///
/// A clean no-op if the spec has no styles at all (e.g. Parry/Envenom/Thrown Weapons, which don't
/// grant styles), already has a `ClassId = 0` universal bucket (nothing to grant - already
/// visible to every class), or `new_class_id` already has its own copy (idempotent across
/// redeploys).
fn grant_styles_for_uncloneable_spec(
    cfg: &SshConfig,
    real_name: &str,
    new_class_id: i64,
) -> Result<String, String> {
    let real_name_q = sql_quote(real_name);

    let already_has_own = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT 1 FROM Style WHERE SpecKeyName = '{real_name_q}' AND ClassId = {new_class_id};"
        ),
    )?;
    if !already_has_own.is_empty() {
        return Ok(String::new());
    }

    let has_universal = run_remote_sql_json(
        cfg,
        &format!("SELECT 1 FROM Style WHERE SpecKeyName = '{real_name_q}' AND ClassId = 0;"),
    )?;
    if !has_universal.is_empty() {
        return Ok(String::new());
    }

    run_remote_sql(
        cfg,
        &format!(
            "INSERT INTO Style (ClassId, Name, SpecKeyName, SpecLevelRequirement, Icon, EnduranceCost, \
             StealthRequirement, OpeningRequirementType, OpeningRequirementValue, AttackResultRequirement, \
             WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, BonusToDefense, TwoHandAnimation, \
             RandomProc, ArmorHitLocation, ID) \
             SELECT {new_class_id}, Name, '{real_name_q}', SpecLevelRequirement, Icon, EnduranceCost, \
             StealthRequirement, OpeningRequirementType, OpeningRequirementValue, AttackResultRequirement, \
             WeaponTypeRequirement, GrowthOffset, GrowthRate, BonusToHit, BonusToDefense, TwoHandAnimation, \
             RandomProc, ArmorHitLocation, ID \
             FROM Style WHERE SpecKeyName = '{real_name_q}' AND ClassId != 0 \
             GROUP BY Name, SpecLevelRequirement;"
        ),
    )?;

    let granted = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT COUNT(*) as c FROM Style WHERE SpecKeyName = '{real_name_q}' AND ClassId = {new_class_id};"
        ),
    )?;
    let count = granted.first().map(|r| int_field(r, "c")).unwrap_or(0);
    if count == 0 {
        return Ok(String::new());
    }
    Ok(format!(
        "granted {count} of '{real_name}'s own combat style(s) to this class (it had none of its own locked copies before)\n"
    ))
}

/// Ensures `class_id` has `real_name` (one of `NEVER_CLONE_SPECS`) assigned directly, un-cloned,
/// at `level`. If `source_key` differs from `real_name` - a previous deploy, before this fix
/// existed, cloned it under its own suffixed key (e.g. "sword_64") - heals that: migrates every
/// affected character's trained points from the old key onto the real one, and moves the stale
/// clone's own Style/SpecXAbility content onto the real spec's row too, so nothing real is lost
/// and nothing lingers duplicated. Also grants this class its own copy of the spec's combat
/// styles if it has none yet - see `grant_styles_for_uncloneable_spec`'s own doc comment for the
/// real, reproduced bug this fixes.
fn ensure_uncloneable_spec_assigned(
    cfg: &SshConfig,
    class_id: i64,
    real_name: &str,
    source_key: &str,
    new_name: &str,
    level: i64,
) -> Result<String, String> {
    let mut log = String::new();
    if source_key != real_name {
        log.push_str(&rename_spec_in_characters(
            cfg, class_id, source_key, real_name,
        )?);
        reparent_onto_real_spec(cfg, source_key, real_name)?;
    }
    let real_name_q = sql_quote(real_name);
    run_remote_sql(
        cfg,
        &format!(
            "DELETE FROM ClassXSpecialization WHERE ClassID = {class_id} AND SpecKeyName = '{real_name_q}';"
        ),
    )?;
    run_remote_sql(
        cfg,
        &format!(
            "INSERT INTO ClassXSpecialization (ClassID, SpecKeyName, LevelAcquired) VALUES ({class_id}, '{real_name_q}', {level});"
        ),
    )?;
    log.push_str(&format!(
        "kept '{}' assigned under its own real name at level {} - the server hardcodes this \
         spec's actual mechanic to its exact literal KeyName, so cloning or renaming it (you \
         asked for '{}') would only ever look right on the character sheet while silently not \
         working in play; see NEVER_CLONE_SPECS\n",
        real_name, level, new_name
    ));
    log.push_str(&grant_styles_for_uncloneable_spec(cfg, real_name, class_id)?);
    Ok(log)
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

/// Prunes an already-deployed spec's own existing SpellLine/LineXSpell rows in place, according
/// to `spell_line_selection` - used specifically for the self-source case in `apply_class_specs`
/// (`source_key == final_key`, i.e. redeploying an unchanged, already-assigned spec), where the
/// normal delete-then-reclone path can never run safely (there is no separate source left to
/// clone from - see that call site's own doc comment for the real data-loss bug that caused). A
/// real, reported bug this fixes: opening an already-deployed spec's "customize spell lines"
/// picker and unchecking a line/spell doesn't change `source_key`, `new_key`, `new_name`, or
/// `level_acquired` at all - only `spell_line_selection` does - so the self-source skip was
/// silently discarding every such exclusion entirely; generating, deploying, even restarting the
/// server did nothing, since `spell_line_selection` was never even looked at in that branch.
///
/// Because this operates on the spec's own *current* `KeyName` (not a fresh clone target), a
/// `SpellLineSelection.source_line_key` here is naturally the exact `SpellLine.KeyName` this key
/// already owns (the Edit flow's own picker calls `list_spec_spell_lines` with this exact key as
/// `spec_key`, confirmed in `ClonePanel.tsx` - never the *original* source's own, differently
/// -prefixed line key, so no remapping is needed at all). A line absent from the selection
/// entirely is excluded, matching `clone_spec`'s own "non-empty selection is authoritative"
/// convention.
fn prune_spell_lines_in_place(
    cfg: &SshConfig,
    final_key: &str,
    spell_line_selection: &[SpellLineSelection],
) -> Result<String, String> {
    let final_key_q = sql_quote(final_key);
    let lines = run_remote_sql_json(
        cfg,
        &format!("SELECT KeyName FROM SpellLine WHERE Spec = '{final_key_q}';"),
    )?;
    let mut log = String::new();
    for line in &lines {
        let line_key = str_field(line, "KeyName");
        let line_key_q = sql_quote(&line_key);
        let selection = spell_line_selection
            .iter()
            .find(|s| s.source_line_key == line_key);

        if selection.is_none_or(|s| !s.included) {
            let spell_ids: Vec<i64> = run_remote_sql_json(
                cfg,
                &format!("SELECT SpellID FROM LineXSpell WHERE LineName = '{line_key_q}';"),
            )?
            .iter()
            .map(|r| int_field(r, "SpellID"))
            .collect();
            run_remote_sql(
                cfg,
                &format!("DELETE FROM LineXSpell WHERE LineName = '{line_key_q}';"),
            )?;
            run_remote_sql(
                cfg,
                &format!("DELETE FROM SpellLine WHERE KeyName = '{line_key_q}';"),
            )?;
            delete_owned_spells(cfg, &spell_ids)?;
            log.push_str(&format!(
                "removed the whole '{}' spell line (excluded from the customization)\n",
                line_key
            ));
            continue;
        }

        let s = selection.unwrap();
        if !s.excluded_spell_ids.is_empty() {
            let ids = s
                .excluded_spell_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            run_remote_sql(
                cfg,
                &format!(
                    "DELETE FROM LineXSpell WHERE LineName = '{line_key_q}' AND SpellID IN ({ids});"
                ),
            )?;
            delete_owned_spells(cfg, &s.excluded_spell_ids)?;
            log.push_str(&format!(
                "removed {} excluded spell(s) from '{}'\n",
                s.excluded_spell_ids.len(),
                line_key
            ));
        }
    }
    Ok(log)
}

/// Deep-clones one specialization: the Specialization row itself, every SpellLine attached to it
/// (plus their LineXSpell entries), and every Style attached to it - all under the new KeyName,
/// as a single SQL batch. `spell_line_selection` narrows which of the source's own SpellLines (and
/// which spells within an included one) actually get cloned - see `SpellLineSelection`'s own doc
/// comment; an empty slice clones every line and every spell in full, unchanged from before this
/// existed.
fn clone_spec(
    cfg: &SshConfig,
    source_key: &str,
    new_key: &str,
    new_name: &str,
    new_class_id: i64,
    spell_line_selection: &[SpellLineSelection],
) -> Result<(), String> {
    let source_key_q = sql_quote(source_key);
    let new_key_q = sql_quote(new_key);
    let new_name_q = sql_quote(new_name);

    // ClientStyleGroup isn't on every fork's own Specialization table at all - a real,
    // reproduced case: Divided Realms' actual live production server (unlike the stale "Othlyn"
    // deployment this project's own earlier testing happened against, before that mixup was
    // caught) simply doesn't have this column, so unconditionally assuming it existed made every
    // single spec clone against the real server fail outright ("table Specialization has no
    // column named ClientStyleGroup") - this had gone undetected because nothing had actually
    // exercised a fresh clone_spec call against that specific database until now. Checked fresh
    // via the real schema (the same "never assume, introspect" approach this project's GUID-PK
    // handling already uses), not assumed present.
    let has_client_style_group = columns_excluding_pk(cfg, "Specialization")?
        .iter()
        .any(|c| c == "ClientStyleGroup");

    let client_style_group = if has_client_style_group {
        // The source spec's own ClientStyleGroup (if it's itself a previous clone) decides
        // whether this new clone inherits an explicit override or needs one computed fresh - see
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
        Some(client_style_group_for(source_key, source_override))
    } else {
        None
    };

    let mut batch = match client_style_group {
        Some(group) => format!(
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
             SELECT '{new_key_q}', '{new_name_q}', Description, Icon, Implementation, {group} \
             FROM Specialization WHERE KeyName = '{source_key_q}';\n"
        ),
        None => format!(
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation) \
             SELECT '{new_key_q}', '{new_name_q}', Description, Icon, Implementation \
             FROM Specialization WHERE KeyName = '{source_key_q}';\n"
        ),
    };

    // Every spell (cast or proc) this clone needs gets its own fully independent `Spell` row -
    // see spell_ownership.rs's own module doc comment for the real bug this closes. One cloner
    // per `clone_spec` call so the same source spell referenced twice (e.g. on two different
    // spell lines) is only ever cloned once for this new class.
    let mut spell_cloner = SpellCloner::new(cfg, new_class_id)?;

    // Spell lines attached to this spec each need their own new unique KeyName - read them first
    // so each clone statement can be built with a name we already know is unique (derived from
    // the new spec key plus the new class ID).
    let lines = run_remote_sql_json(
        cfg,
        &format!("SELECT KeyName, Name FROM SpellLine WHERE Spec = '{source_key_q}';"),
    )?;
    for line in &lines {
        let old_line_key = str_field(line, "KeyName");

        // A real, reproduced problem this exists for: several real specs span multiple
        // SpellLines shared across different classes ("Mind Magic" has both "Mind Twisting" and
        // "Domination"; "Enhancement" has five, one per real class that shares it) - cloning the
        // whole spec previously pulled in every single one of the source's lines with no way to
        // pick just the one actually wanted. An empty selection preserves that original
        // behavior exactly (clone every line); a non-empty one is authoritative - a line of the
        // source's own that isn't mentioned in it at all, or is mentioned but marked excluded, is
        // skipped entirely.
        let selection = spell_line_selection
            .iter()
            .find(|s| s.source_line_key == old_line_key);
        if !spell_line_selection.is_empty() {
            match selection {
                None => continue,
                Some(s) if !s.included => continue,
                _ => {}
            }
        }

        // A cloned line is always a regular, spec-point-gated line - never a base line, no
        // matter what the source's own IsBaseLine was. A real, reproduced bug found live:
        // cloning "Mind Twisting" (a genuine base line for the real Sorcerer, whose own class
        // design grants it free based on character level alone, needing no spec investment)
        // carried that flag over verbatim - Specialization.cs's own real granting logic sets a
        // base line's effective level straight to the player's *character* level
        // (`ls.Item1.Level = player.Level`), completely ignoring however many points were
        // actually trained in the spec. The clone's spec points showed correctly on the
        // character sheet, but every spell up to the character's own level was silently granted
        // regardless, and the in-game trainer window had nothing to preview/train at all (there
        // was never anything point-gated to show). Every other spec this tool clones is meant to
        // unlock progressively as points are trained, so this is forced to 0 unconditionally.
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
             VALUES ('{new_line_key_q}', '{new_name_q}', '{new_key_q}', 0, {new_class_id});\n"
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
            let source_spell_id = int_field(spell, "SpellID");
            // "Remove specific spells from the line we copy over" - a selected line can still
            // exclude individual spells within it, independent of whether whole other lines were
            // excluded above. Checked against the *source*'s own SpellID (what the picker UI
            // actually shows and the caller selects against), before this spell is ever cloned.
            if let Some(s) = selection {
                if s.excluded_spell_ids.contains(&source_spell_id) {
                    continue;
                }
            }
            let level = int_field(spell, "Level");
            let new_spell_id = spell_cloner.clone_for_class(cfg, source_spell_id)?;
            let row_guid = Uuid::new_v4();
            batch.push_str(&format!(
                "INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) \
                 VALUES ('{new_line_key_q}', {new_spell_id}, {level}, '{row_guid}');\n"
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
    // Grouping by (Name, SpecLevelRequirement) collapses exactly that "same style, one row per
    // originally-eligible class" pattern into a single row before the clone - without it, cloning
    // a spec like that would produce several duplicate, identically-named styles all now locked
    // to the same new class (confirmed real: Celtic Dual has every style name three times, once
    // per class, with every other field - EnduranceCost, Icon, etc. - identical between them).
    //
    // Done as one explicit-ID INSERT per group (not a single bulk INSERT...SELECT, which this
    // used before) because a style's own procs (StyleXSpell, cloned below) need to know this
    // clone's real new StyleID up front, in Rust, to build the proc INSERT alongside it in the
    // same batch - the DB's own autoincrement assignment isn't visible until the whole batch
    // actually runs, in one shot, at the very end of this function. Each group's new StyleID is
    // instead chosen explicitly, from the same kind of "next free ID in this table" counter
    // `SpellCloner` uses for `Spell` - and conveniently also removes the need for the old
    // separate "UPDATE Style SET ID = StyleID" pass, since ID is now set correctly at insert time.
    let source_styles = run_remote_sql_json(
        cfg,
        &format!(
            "SELECT StyleID, ClassId, Name, SpecLevelRequirement FROM Style \
             WHERE SpecKeyName = '{source_key_q}';"
        ),
    )?;
    let mut style_groups: Vec<(String, i64, Vec<Value>)> = Vec::new();
    for row in &source_styles {
        let name = str_field(row, "Name");
        let level_req = int_field(row, "SpecLevelRequirement");
        match style_groups
            .iter_mut()
            .find(|(n, l, _)| *n == name && *l == level_req)
        {
            Some((_, _, rows)) => rows.push(row.clone()),
            None => style_groups.push((name, level_req, vec![row.clone()])),
        }
    }

    // StyleXSpell's own GUID-style primary key column (if it has one at all - see
    // `style_x_spell_guid_column`'s own doc comment), resolved once up front and reused for
    // every proc cloned below.
    let style_x_spell_pk = style_x_spell_guid_column(cfg)?;
    for (_, _, rows) in &style_groups {
        let representative = &rows[0];
        let representative_style_id = int_field(representative, "StyleID");
        let class_expr = if int_field(representative, "ClassId") != 0 {
            new_class_id.to_string()
        } else {
            "0".to_string()
        };
        // Clones this style's row right now (not deferred into `batch`), with its real new
        // StyleID computed atomically inside the database rather than guessed in Rust - see
        // `SpellCloner::clone_for_class`'s own doc comment for the exact real bug this same
        // pattern avoids (a value precomputed once and incremented locally goes stale the
        // instant anything else writes to the same table first).
        let new_style_id =
            clone_style_with_fresh_id(cfg, representative_style_id, &new_key_q, &class_expr)?;

        // Procs: gather every StyleXSpell row attached to *any* of this group's collapsed source
        // StyleIDs (not just the representative one - the real data's own duplicates are
        // expected to carry the same procs, but this stays correct even if that ever isn't
        // true), deduped by (SpellID, ClassID, Chance) so an identical proc recorded under
        // several of the duplicate rows doesn't get attached to the new style more than once.
        let old_style_ids_csv = rows
            .iter()
            .map(|r| int_field(r, "StyleID").to_string())
            .collect::<Vec<_>>()
            .join(",");
        let procs = run_remote_sql_json(
            cfg,
            &format!(
                "SELECT SpellID, ClassID, Chance FROM StyleXSpell WHERE StyleID IN ({old_style_ids_csv});"
            ),
        )?;
        let mut seen_procs: Vec<(i64, i64, i64)> = Vec::new();
        for proc in &procs {
            let key = (
                int_field(proc, "SpellID"),
                int_field(proc, "ClassID"),
                int_field(proc, "Chance"),
            );
            if seen_procs.contains(&key) {
                continue;
            }
            seen_procs.push(key);
            let (source_proc_spell_id, source_proc_class_id, chance) = key;
            let new_proc_spell_id = spell_cloner.clone_for_class(cfg, source_proc_spell_id)?;
            // Same class-lock repoint rule as the style itself: 0 means "not a substitute style"
            // (per the real schema's own ClassID doc comment) and must stay 0; any non-zero lock
            // must point at this new class, never the source's.
            let proc_class_expr = if source_proc_class_id != 0 {
                new_class_id
            } else {
                0
            };
            let (pk_col, pk_val) = match &style_x_spell_pk {
                Some(col) => (format!(", {col}"), format!(", '{}'", Uuid::new_v4())),
                None => (String::new(), String::new()),
            };
            batch.push_str(&format!(
                "INSERT INTO StyleXSpell (SpellID, ClassID, StyleID, Chance{pk_col}) \
                 VALUES ({new_proc_spell_id}, {proc_class_expr}, {new_style_id}, {chance}{pk_val});\n"
            ));
        }
    }

    // Style.ID is a *separate* field from the DB primary key StyleID - it's what a chain style's
    // own OpeningRequirementValue references (AttackResultRequirement=7, "a style is required")
    // to name its prerequisite, confirmed in the real server source and packet code
    // (`it.Item1.ID == style.OpeningRequirementValue`). Simply copying the source's own ID
    // verbatim would be wrong regardless - that value belongs to the *un-cloned* prerequisite
    // style, not whichever row this clone got - a real, reproduced bug found live: a cloned
    // "Gale" style still pointed at the source class's "Snow Shower" (by its old ID), which
    // doesn't exist in the new spec's own ID space, so the client showed no prerequisite for it
    // at all. The INSERT above already sets ID = StyleID for every row this clone just made (all
    // a plain, non-chain style needs); for every chain style among them, look up its prerequisite
    // by matching *names* between the original spec's styles and this clone's own (now-ID
    // -correct) styles, and repoint OpeningRequirementValue at the clone's own prerequisite's new
    // ID - never the original's.
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
            CREATE TABLE DOLCharacters (Name TEXT, Class INTEGER, RespecAmountAllSkill INTEGER DEFAULT 0, SerializedSpecs TEXT DEFAULT '');
            CREATE TABLE Spell (SpellID INTEGER NOT NULL DEFAULT 0, Name TEXT, SpellGroup INTEGER DEFAULT 0, Spell_ID TEXT NOT NULL DEFAULT '', PRIMARY KEY (Spell_ID));
            CREATE UNIQUE INDEX U_Spell_SpellID ON Spell (SpellID);
            CREATE TABLE StyleXSpell (StyleXSpell_ID TEXT PRIMARY KEY DEFAULT '', SpellID INTEGER, ClassID INTEGER, StyleID INTEGER, Chance INTEGER);

            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) VALUES ('slash', 'Slash', 'Slashing weapons', 100, NULL, 0);
            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) VALUES ('smite', 'Smite', 'Holy damage spells', 200, NULL, 0);
            INSERT INTO ClassXSpecialization VALUES (1, 'slash', 0);
            INSERT INTO ClassXSpecialization VALUES (1, 'smite', 0);
            INSERT INTO Style VALUES (NULL, 1, 'Pulverize', 'slash', 5, 300, 5, 0, 1, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);
            INSERT INTO Style VALUES (NULL, 0, 'Backlash', 'slash', 10, 301, 5, 1, 2, 1, 1, 1, 1.0, 1.0, 5, 0, 0, 0, 0, 0);
            INSERT INTO SpellLine VALUES ('smite_base', 'Smiting Spells', 'smite', 0, 1);
            INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('smite_base', 9001, 1, 'seed-smite-1');
            INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('smite_base', 9002, 10, 'seed-smite-2');
            INSERT INTO Spell (SpellID, Name, SpellGroup, Spell_ID) VALUES (9001, 'Smite', 500, 'seed-spell-9001');
            INSERT INTO Spell (SpellID, Name, SpellGroup, Spell_ID) VALUES (9002, 'Greater Smite', 500, 'seed-spell-9002');
            INSERT INTO Spell (SpellID, Name, SpellGroup, Spell_ID) VALUES (9099, 'Blinding Light', 0, 'seed-spell-9099');
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
            spell_line_selection: Vec::new(),
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
    fn clone_deep_clones_a_styles_proc_as_its_own_independent_spell() {
        // Closes the second half of the gap this module's own doc comment describes: Style
        // procs (StyleXSpell) used to not be cloned at all. "Pulverize" (ClassId=1, locked to
        // the source class) gets a proc here - the clone must get its own StyleXSpell row,
        // pointing at the new style, repointed to the new class, and referencing its own
        // independent Spell row (not the source's own SpellID 9099).
        let dir = setup_test_db("clone-style-proc");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO StyleXSpell (StyleXSpell_ID, SpellID, ClassID, StyleID, Chance) \
             VALUES ('seed-proc-1', 9099, 1, \
             (SELECT StyleID FROM Style WHERE Name='Pulverize' AND SpecKeyName='slash'), 25);",
        )
        .unwrap();

        let assignments = vec![SpecAssignment {
            source_key: "slash".to_string(),
            new_key: "earthshatter".to_string(),
            new_name: "Earthshatter".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let new_style_id = int_field(
            &run_remote_sql_json(
                &cfg,
                "SELECT StyleID FROM Style WHERE Name='Pulverize' AND SpecKeyName='earthshatter_64';",
            )
            .unwrap()[0],
            "StyleID",
        );
        let procs = run_remote_sql_json(
            &cfg,
            &format!("SELECT SpellID, ClassID, Chance FROM StyleXSpell WHERE StyleID = {new_style_id};"),
        )
        .unwrap();
        assert_eq!(procs.len(), 1, "{procs:?}");
        assert_eq!(int_field(&procs[0], "Chance"), 25);
        assert_eq!(
            int_field(&procs[0], "ClassID"),
            64,
            "a class-locked proc must repoint to the new class"
        );
        let new_proc_spell_id = int_field(&procs[0], "SpellID");
        assert_ne!(
            new_proc_spell_id, 9099,
            "the proc's own spell must be its own independent clone, not the source's SpellID"
        );
        let proc_spell = run_remote_sql_json(
            &cfg,
            &format!("SELECT Name FROM Spell WHERE SpellID = {new_proc_spell_id};"),
        )
        .unwrap();
        assert_eq!(str_field(&proc_spell[0], "Name"), "Blinding Light");

        // The source's own proc must be completely untouched.
        let original_procs = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM StyleXSpell WHERE StyleID = \
             (SELECT StyleID FROM Style WHERE Name='Pulverize' AND SpecKeyName='slash');",
        )
        .unwrap();
        assert_eq!(original_procs.len(), 1, "the source proc must be untouched");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn migrate_converts_an_old_classs_spells_and_procs_to_independent_clones() {
        // Simulates a class that was cloned *before* `clone_spec` started independently cloning
        // spells - its LineXSpell/StyleXSpell still point straight at the real stock spec's own
        // shared SpellIDs, exactly the shape every class created before this feature existed is
        // left in.
        let dir = setup_test_db("migrate-old-class");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                VALUES ('earthshatter_64', 'Earthshatter', 'Slashing weapons', 100, NULL, 0);
             INSERT INTO ClassXSpecialization VALUES (64, 'earthshatter_64', 0);
             INSERT INTO SpellLine VALUES ('earthshatter_64_smite_base', 'Earthshatter Spells', 'earthshatter_64', 0, 64);
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('earthshatter_64_smite_base', 9001, 1, 'old-line-1');
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('earthshatter_64_smite_base', 9002, 10, 'old-line-2');
             INSERT INTO Style VALUES (NULL, 64, 'Shatter', 'earthshatter_64', 5, 300, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);
             INSERT INTO StyleXSpell VALUES ('old-proc-1', 9099, 64, (SELECT StyleID FROM Style WHERE SpecKeyName='earthshatter_64'), 25);",
        )
        .unwrap();

        let mut progress_calls: Vec<(usize, usize)> = Vec::new();
        let log = migrate_class_to_owned_spells(&cfg, 64, |processed, total| {
            progress_calls.push((processed, total))
        })
        .unwrap();
        assert!(
            log.contains("migrated 2 spell(s) and 1 proc(s)"),
            "log: {log}"
        );
        assert_eq!(
            progress_calls.first(),
            Some(&(0, 3)),
            "the real total (2 spells + 1 proc) must be reported up front, before any work: \
             {progress_calls:?}"
        );
        assert_eq!(
            progress_calls.last(),
            Some(&(3, 3)),
            "progress must finish at the real total: {progress_calls:?}"
        );

        let line_spells = run_remote_sql_json(
            &cfg,
            "SELECT SpellID FROM LineXSpell WHERE LineName='earthshatter_64_smite_base' ORDER BY Level;",
        )
        .unwrap();
        assert_eq!(line_spells.len(), 2);
        let new_cast_a = int_field(&line_spells[0], "SpellID");
        let new_cast_b = int_field(&line_spells[1], "SpellID");
        assert_ne!(new_cast_a, 9001, "must no longer point at the shared source spell");
        assert_ne!(new_cast_b, 9002, "must no longer point at the shared source spell");

        let proc_rows = run_remote_sql_json(
            &cfg,
            "SELECT SpellID FROM StyleXSpell WHERE StyleXSpell_ID='old-proc-1';",
        )
        .unwrap();
        let new_proc_id = int_field(&proc_rows[0], "SpellID");
        assert_ne!(new_proc_id, 9099, "the proc must also now be its own independent clone");

        // The original, shared spells (9001/9002/9099) must be completely untouched.
        let originals = run_remote_sql_json(
            &cfg,
            "SELECT SpellID FROM Spell WHERE SpellID IN (9001, 9002, 9099);",
        )
        .unwrap();
        assert_eq!(originals.len(), 3, "the original shared spells must survive untouched");

        // Running it again must be a safe no-op - everything's already independently owned.
        let second_log = migrate_class_to_owned_spells(&cfg, 64, |_, _| {}).unwrap();
        assert!(
            second_log.contains("migrated 0 spell(s) and 0 proc(s)"),
            "second run should find nothing left to migrate: {second_log}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_spell_lines_for_spec_returns_a_specs_own_lines_in_level_order() {
        let dir = setup_test_db("list-lines-for-spec");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let groups = list_spell_lines_for_spec(&cfg, 64, "frostbolt_tree_64").unwrap();
        assert_eq!(groups.len(), 1, "{groups:?}");
        let group = &groups[0];
        assert_eq!(group.spec_key, "frostbolt_tree_64");
        assert_eq!(group.line_name, "Frostbolt Tree");
        assert_eq!(group.spells.len(), 2, "{:?}", group.spells);
        assert_eq!(group.spells[0].level, 1, "spells must come back in level order");
        assert_eq!(group.spells[1].level, 10);
        assert!(
            group.spells.iter().all(|s| s.is_owned),
            "every spell cloned by apply_class_specs is already independent: {:?}",
            group.spells
        );
        assert_ne!(
            group.spells[0].spell_id, 9001,
            "must reference the independent clone, not the shared source spell"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_spell_lines_for_spec_never_leaks_another_classs_own_lines() {
        // The real, confirmed-live bug this guards against: Healer, Shaman and Valkyrie all
        // share the "Mending" spec, but the real server's own SpellLine table carries a
        // *separate* ClassIDHint-tagged row per class - a naive `WHERE Spec = key` query (what
        // this function used to run) returned all three classes' own rows mixed together,
        // showing spells meant only for Valkyrie under Healer's own Mending display.
        let dir = setup_test_db("no-cross-class-leak");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO SpellLine VALUES ('mending_neutral_base', 'Mending', 'mending', 1, 0);\
             INSERT INTO SpellLine VALUES ('healer_mending_spec', 'Mending', 'mending', 0, 26);\
             INSERT INTO SpellLine VALUES ('valkyrie_mending_spec', 'Mending', 'mending', 0, 34);\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('healer_mending_spec', 9001, 1, 'healer-line-1');\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('valkyrie_mending_spec', 9002, 1, 'valkyrie-line-1');",
        )
        .unwrap();

        let groups = list_spell_lines_for_spec(&cfg, 26, "mending").unwrap();
        let keys: Vec<&str> = groups.iter().map(|g| g.line_key.as_str()).collect();
        assert!(keys.contains(&"mending_neutral_base"), "{keys:?}");
        assert!(keys.contains(&"healer_mending_spec"), "{keys:?}");
        assert!(
            !keys.contains(&"valkyrie_mending_spec"),
            "Healer must never see Valkyrie's own Mending line: {keys:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_spell_lines_for_spec_prefers_a_classs_own_baseline_over_the_shared_neutral_one() {
        // Valkyrie has its own baseline Mending line (distinct spells from the shared one every
        // other Mending class falls back to) - real, confirmed live, via SpellLine rows
        // 'Mending' (ClassIDHint=0) and 'Valkyrie Mending' (ClassIDHint=34), both IsBaseLine=1.
        let dir = setup_test_db("baseline-own-over-neutral");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO SpellLine VALUES ('mending_neutral_base', 'Mending', 'mending', 1, 0);\
             INSERT INTO SpellLine VALUES ('valkyrie_mending_base', 'Valkyrie Mending', 'mending', 1, 34);",
        )
        .unwrap();

        let healer_groups = list_spell_lines_for_spec(&cfg, 26, "mending").unwrap();
        assert_eq!(
            healer_groups.iter().map(|g| g.line_key.as_str()).collect::<Vec<_>>(),
            vec!["mending_neutral_base"],
            "a class with no baseline of its own falls back to the shared neutral one"
        );

        let valkyrie_groups = list_spell_lines_for_spec(&cfg, 34, "mending").unwrap();
        assert_eq!(
            valkyrie_groups.iter().map(|g| g.line_key.as_str()).collect::<Vec<_>>(),
            vec!["valkyrie_mending_base"],
            "a class with its own baseline uses only its own, not the shared neutral one too"
        );
        assert!(valkyrie_groups[0].is_base_line);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_spell_lines_for_spec_separates_base_lines_from_spec_gated_lines() {
        // Cleric Smite, real and confirmed live: every Cleric gets a Direct Damage and a Stun
        // line for free regardless of spec investment (IsBaseLine=1, scaling off character
        // level), but only gains a Damage Add / Mesmerize / etc. line once points are actually
        // spent in Smite (IsBaseLine=0, scaling off spec level) - these must never be merged
        // into one undifferentiated list, which was this tool's own second real bug found live.
        let dir = setup_test_db("base-vs-spec-gated");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO SpellLine VALUES ('smite_dd_base', 'Direct Damage', 'smite', 1, 0);\
             INSERT INTO SpellLine VALUES ('smite_stun_base', 'Stun', 'smite', 1, 0);\
             INSERT INTO SpellLine VALUES ('smite_dmgadd_spec', 'Damage Add', 'smite', 0, 2);\
             INSERT INTO SpellLine VALUES ('smite_mez_spec', 'Mesmerise', 'smite', 0, 2);",
        )
        .unwrap();

        let groups = list_spell_lines_for_spec(&cfg, 2, "smite").unwrap();
        let base: Vec<&str> = groups
            .iter()
            .filter(|g| g.is_base_line)
            .map(|g| g.line_key.as_str())
            .collect();
        let gated: Vec<&str> = groups
            .iter()
            .filter(|g| !g.is_base_line)
            .map(|g| g.line_key.as_str())
            .collect();
        assert_eq!(base, vec!["smite_dd_base", "smite_stun_base"], "{groups:?}");
        assert_eq!(gated, vec!["smite_dmgadd_spec", "smite_mez_spec"], "{groups:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_spell_lines_for_spec_a_maulers_own_lines_extend_the_neutral_ones_instead_of_replacing_them() {
        // Maulers (class IDs 60-62) are the one documented exception: their own realm-gift lines
        // extend the shared Aura Manipulation line (every other class's own lines replace the
        // neutral fallback entirely) - see Specialization.SpecLinesForClass's own doc comment.
        let dir = setup_test_db("mauler-extends-neutral");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO SpellLine VALUES ('aura_neutral', 'Aura Manipulation', 'aura', 0, 0);\
             INSERT INTO SpellLine VALUES ('aura_mauler_gift', 'Discipline of Laresh', 'aura', 0, 61);\
             INSERT INTO SpellLine VALUES ('aura_other_own', 'Other Aura', 'aura', 0, 5);",
        )
        .unwrap();

        assert_eq!(
            list_spell_lines_for_spec(&cfg, 61, "aura")
                .unwrap()
                .iter()
                .map(|g| g.line_key.clone())
                .collect::<Vec<_>>(),
            vec!["aura_neutral", "aura_mauler_gift"],
            "a Mauler keeps the shared line and gains its own on top of it"
        );

        // Any other class with its own non-baseline row still replaces the neutral one outright.
        assert_eq!(
            list_spell_lines_for_spec(&cfg, 5, "aura")
                .unwrap()
                .iter()
                .map(|g| g.line_key.clone())
                .collect::<Vec<_>>(),
            vec!["aura_other_own"],
            "a non-Mauler's own line replaces the neutral one entirely"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_line_spell_level_changes_the_level_on_this_classs_own_line() {
        let dir = setup_test_db("update-level-owned");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let groups = list_spell_lines_for_spec(&cfg, 64, "frostbolt_tree_64").unwrap();
        let first_spell = &groups[0].spells[0];
        assert_eq!(first_spell.level, 1);

        update_line_spell_level(&cfg, 64, &first_spell.line_xspell_id, 5).unwrap();

        let groups_after = list_spell_lines_for_spec(&cfg, 64, "frostbolt_tree_64").unwrap();
        assert_eq!(groups_after[0].spells[0].level, 5);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_line_spell_level_clones_a_previously_shared_specs_line_on_first_touch() {
        // A real stock class's own spec (never cloned through this tool at all, e.g. a class
        // like Friar that's simply always had "Staff" or a caster spec assigned directly) must
        // still be editable - this promotes it to this class's own independent clone on the
        // spot, the same "clone on first touch" model already established for individual
        // spells, instead of refusing outright the way this used to.
        let dir = setup_test_db("update-level-promotes-shared");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO ClassXSpecialization VALUES (64, 'slash', 0);\
             INSERT INTO SpellLine VALUES ('slash_base', 'Slash Styles', 'slash', 0, 1);\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('slash_base', 9001, 1, 'shared-line-1');",
        )
        .unwrap();

        let result = update_line_spell_level(&cfg, 64, "shared-line-1", 5).unwrap();
        assert!(result.contains("updated level to 5"), "{result}");

        let assigned = run_remote_sql_json(
            &cfg,
            "SELECT SpecKeyName FROM ClassXSpecialization WHERE ClassID = 64;",
        )
        .unwrap();
        assert_eq!(
            str_field(&assigned[0], "SpecKeyName"),
            "slash_64",
            "the class's own assignment must now point at its own independent clone"
        );

        let groups = list_spell_lines_for_spec(&cfg, 64, "slash_64").unwrap();
        assert_eq!(groups[0].spells[0].level, 5);
        assert!(groups[0].spells[0].is_owned);

        let original_level = run_remote_sql_json(
            &cfg,
            "SELECT Level FROM LineXSpell WHERE LineXSpell_ID = 'shared-line-1';",
        )
        .unwrap();
        assert_eq!(
            int_field(&original_level[0], "Level"),
            1,
            "the original, still-shared line must be untouched"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_line_spell_owned_clones_on_first_call_and_is_idempotent_after() {
        // This is the "Save just works" building block - no separate whole-class Migrate pass
        // needed before a single spell can be field-edited. Simulates a class cloned *before*
        // `clone_spec` started independently cloning spells itself (same legacy shape used by
        // `migrate_converts_an_old_classs_spells_and_procs_to_independent_clones`), since
        // `apply_class_specs` today always clones spells up front already - this needs a line
        // still genuinely un-owned to exercise the on-demand clone path at all.
        let dir = setup_test_db("ensure-owned-first-call");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                VALUES ('frostbolt_tree_64', 'Frostbolt Tree', 'Cold damage spells', 100, NULL, 0);
             INSERT INTO ClassXSpecialization VALUES (64, 'frostbolt_tree_64', 0);
             INSERT INTO SpellLine VALUES ('frostbolt_tree_64_smite_base', 'Frostbolt Spells', 'frostbolt_tree_64', 0, 64);
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('frostbolt_tree_64_smite_base', 9001, 1, 'old-line-1');",
        )
        .unwrap();

        let groups = list_spell_lines_for_spec(&cfg, 64, "frostbolt_tree_64").unwrap();
        let line = &groups[0].spells[0];
        assert!(!line.is_owned, "a legacy line's spell starts out un-owned");
        let original_spell_id = line.spell_id;
        assert_eq!(original_spell_id, 9001);

        let first_call_id = ensure_line_spell_owned(&cfg, 64, &line.line_xspell_id).unwrap();
        assert_ne!(
            first_call_id, original_spell_id,
            "cloning must produce a brand-new SpellID, not reuse the shared one"
        );

        let groups_after = list_spell_lines_for_spec(&cfg, 64, "frostbolt_tree_64").unwrap();
        let line_after = &groups_after[0].spells[0];
        assert!(line_after.is_owned);
        assert_eq!(line_after.spell_id, first_call_id);

        // The original, shared spell must be completely untouched.
        let original = run_remote_sql_json(&cfg, "SELECT SpellID FROM Spell WHERE SpellID = 9001;").unwrap();
        assert_eq!(original.len(), 1, "the original shared spell must survive untouched");

        // Calling it again once already owned must be a pure no-op - same ID, no second clone.
        let second_call_id = ensure_line_spell_owned(&cfg, 64, &line.line_xspell_id).unwrap();
        assert_eq!(second_call_id, first_call_id);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_line_spell_owned_promotes_a_previously_shared_specs_line_on_first_touch() {
        // The stock-class counterpart to the legacy-custom-class test above: a spec this class
        // was simply assigned directly (never cloned through this tool at all) must still become
        // independently editable the moment it's actually touched.
        let dir = setup_test_db("ensure-owned-promotes-shared");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO ClassXSpecialization VALUES (64, 'slash', 0);\
             INSERT INTO SpellLine VALUES ('slash_base', 'Slash Styles', 'slash', 0, 1);\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('slash_base', 9001, 1, 'shared-line-1');",
        )
        .unwrap();

        let new_spell_id = ensure_line_spell_owned(&cfg, 64, "shared-line-1").unwrap();
        assert_ne!(new_spell_id, 9001, "must be an independent clone, not the shared source spell");

        let groups = list_spell_lines_for_spec(&cfg, 64, "slash_64").unwrap();
        assert_eq!(groups[0].spells[0].spell_id, new_spell_id);
        assert!(groups[0].spells[0].is_owned);

        let original = run_remote_sql_json(&cfg, "SELECT SpellID FROM Spell WHERE SpellID = 9001;").unwrap();
        assert_eq!(original.len(), 1, "the original shared spell must survive untouched");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_line_spell_owned_refuses_a_mechanic_locked_specs_line() {
        // Defensive guard: a NEVER_CLONE_SPECS spec's own SpellLine (an unrealistic shape for a
        // real weapon spec, which doesn't cast spells, but still worth guarding against) must
        // never be cloned/renamed outright - only its combat styles can ever become
        // class-specific, via `ensure_style_owned` instead.
        let dir = setup_test_db("ensure-owned-refuses-uncloneable");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                VALUES ('Staff', 'Staff', 'Staff weapons', 100, NULL, 0);
             INSERT INTO ClassXSpecialization VALUES (64, 'Staff', 0);
             INSERT INTO SpellLine VALUES ('staff_base', 'Staff Spells', 'Staff', 0, 1);
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('staff_base', 9001, 1, 'staff-line-1');",
        )
        .unwrap();

        let result = ensure_line_spell_owned(&cfg, 64, "staff-line-1");
        assert!(result.is_err(), "a mechanic-locked spec's spell line must never be cloned");

        let unchanged = run_remote_sql_json(
            &cfg,
            "SELECT SpellID FROM LineXSpell WHERE LineXSpell_ID = 'staff-line-1';",
        )
        .unwrap();
        assert_eq!(int_field(&unchanged[0], "SpellID"), 9001, "must be untouched");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_of_a_recognized_spec_auto_populates_client_style_group() {
        // A real, client-recognized spec name (one of CLIENT_STYLE_GROUPS's own entries) must not
        // leave its clone's styles grouped under the client's generic "Abilities" bucket - the
        // clone should automatically inherit the source's own recognized category, with no server
        // code change and no manual step needed. Uses "Mentalism" (a pure caster spec, not in
        // NEVER_CLONE_SPECS) rather than "Celtic Dual" - a real bug found live used that for
        // Nosferatu's Claw originally, which silently broke her actual dual-wield damage bonus
        // (see NEVER_CLONE_SPECS's own doc comment); this test only needs a recognized name that
        // is actually safe to clone.
        let dir = setup_test_db("clone-recognized-name");
        let cfg = test_cfg(&dir);
        let sql = "
            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                VALUES ('Mentalism', 'Mentalism', 'Mental spells', 300, NULL, 0);
        ";
        let status = Command::new("sqlite3")
            .arg(dir.join("data/opendaoc.sqlite3.db"))
            .arg(sql)
            .status()
            .unwrap();
        assert!(status.success());

        let assignments = vec![SpecAssignment {
            source_key: "Mentalism".to_string(),
            new_key: "nosferatusclaw".to_string(),
            new_name: "Nosferatu's Claw".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT ClientStyleGroup FROM Specialization WHERE KeyName='nosferatusclaw_64';",
        )
        .unwrap();
        assert_eq!(int_field(&rows[0], "ClientStyleGroup"), 0x69);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clone_spec_works_on_a_fork_whose_specialization_table_has_no_client_style_group_at_all() {
        // A real, reproduced bug found live: the actual production server this project targets
        // (unlike the stale "Othlyn" deployment this project's own earlier testing mistakenly
        // happened against) simply has no ClientStyleGroup column on Specialization at all -
        // confirmed directly against its real schema. Unconditionally assuming the column existed
        // made every single spec clone fail outright ("table Specialization has no column named
        // ClientStyleGroup") the moment it was actually tried against that real database - this
        // had gone undetected because nothing had exercised a fresh clone_spec call against that
        // specific schema shape until then. Mirrors that exact schema (no ClientStyleGroup at
        // all), rather than reusing setup_test_db, which always has the column.
        let dir = std::env::temp_dir().join("daoc-class-editor-specs-test-no-client-style-group");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let db = dir.join("data/opendaoc.sqlite3.db");
        let sql = "
            CREATE TABLE Specialization (KeyName TEXT, Name TEXT, Description TEXT, Icon INTEGER, Implementation TEXT);
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

            INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation) VALUES ('slash', 'Slash', 'Slashing weapons', 100, NULL);
            INSERT INTO ClassXSpecialization VALUES (1, 'slash', 0);
            INSERT INTO Style VALUES (NULL, 1, 'Pulverize', 'slash', 5, 300, 5, 0, 1, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);
        ";
        let status = Command::new("sqlite3").arg(&db).arg(sql).status().unwrap();
        assert!(status.success());
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "slash".to_string(),
            new_key: "earthshatter".to_string(),
            new_name: "Earthshatter".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        let result = apply_class_specs(&cfg, 64, &assignments, &[]);
        assert!(result.is_ok(), "{result:?}");

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT Name FROM Specialization WHERE KeyName='earthshatter_64';",
        )
        .unwrap();
        assert_eq!(str_field(&rows[0], "Name"), "Earthshatter");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_new_combat_spec_seeds_an_immediately_editable_placeholder_style() {
        // A from-scratch spec with nothing in it at all would be invisible to both the "Combat
        // specs" and "Spells" summary lists (ClassSpecSummary::has_styles/has_spell_lines are
        // both false) - this proves the new spec is seeded with exactly enough to show up in the
        // former and be immediately editable via the already-built style field editor, with no
        // new "empty spec" UI state required.
        let dir = setup_test_db("create-combat-spec");
        let cfg = test_cfg(&dir);

        let summary = create_new_combat_spec(&cfg, 2, "Earthshatter", 5).unwrap();
        assert_eq!(summary.key_name, "earthshatter_2");
        assert!(summary.is_owned);
        assert!(summary.has_styles);
        assert!(!summary.has_spell_lines);

        let spec_rows = run_remote_sql_json(
            &cfg,
            "SELECT Name FROM Specialization WHERE KeyName = 'earthshatter_2';",
        )
        .unwrap();
        assert_eq!(str_field(&spec_rows[0], "Name"), "Earthshatter");

        let assignment_rows = run_remote_sql_json(
            &cfg,
            "SELECT LevelAcquired FROM ClassXSpecialization WHERE ClassID = 2 AND SpecKeyName = 'earthshatter_2';",
        )
        .unwrap();
        assert_eq!(int_field(&assignment_rows[0], "LevelAcquired"), 5);

        let style_rows = run_remote_sql_json(
            &cfg,
            "SELECT Name, ClassId, AttackResultRequirement FROM Style WHERE SpecKeyName = 'earthshatter_2';",
        )
        .unwrap();
        assert_eq!(style_rows.len(), 1);
        assert_eq!(str_field(&style_rows[0], "Name"), "New Style");
        assert_eq!(int_field(&style_rows[0], "ClassId"), 2);
        // The seeded style must never be a chain style (AttackResultRequirement = 7) - there is
        // no sibling style in this brand-new spec for its OpeningRequirementValue to resolve
        // against, unlike clone_spec's own chain-repoint pass.
        assert_ne!(int_field(&style_rows[0], "AttackResultRequirement"), 7);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_style_to_spec_extends_an_already_owned_combat_spec_at_the_next_level() {
        let dir = setup_test_db("add-style-owned");
        let cfg = test_cfg(&dir);

        // create_new_combat_spec seeds exactly one style at SpecLevelRequirement = 1, already
        // owned by class 2 - the simplest case: nothing left to clone, just append.
        create_new_combat_spec(&cfg, 2, "Earthshatter", 5).unwrap();

        let added = add_style_to_spec(&cfg, 2, "earthshatter_2", None).unwrap();
        assert!(added.is_owned);
        assert_eq!(str_field(&added.fields, "Name"), "New Style");
        assert_eq!(int_field(&added.fields, "ClassId"), 2);
        assert_eq!(int_field(&added.fields, "SpecLevelRequirement"), 2, "one above the seeded style's own level 1");

        let style_rows = run_remote_sql_json(
            &cfg,
            "SELECT StyleID FROM Style WHERE SpecKeyName = 'earthshatter_2';",
        )
        .unwrap();
        assert_eq!(style_rows.len(), 2, "the original placeholder plus the newly added one");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_style_to_spec_clones_a_still_shared_specs_styles_first() {
        let dir = setup_test_db("add-style-shared");
        let cfg = test_cfg(&dir);

        // Class 1 has "slash" assigned under its plain, shared KeyName (never cloned through
        // this tool) - adding a style here must clone the whole spec (and its existing styles)
        // under class 1's own key first, exactly like editing an existing style already does.
        let added = add_style_to_spec(&cfg, 1, "slash", Some(20)).unwrap();
        assert!(added.is_owned);
        assert_eq!(int_field(&added.fields, "ClassId"), 1);
        assert_eq!(int_field(&added.fields, "SpecLevelRequirement"), 20, "explicit level overrides the default");

        let cloned_styles = run_remote_sql_json(
            &cfg,
            "SELECT Name, ClassId FROM Style WHERE SpecKeyName = 'slash_1' ORDER BY Name;",
        )
        .unwrap();
        // Pulverize (originally ClassId=1) + Backlash (originally ClassId=0) carried over from
        // the clone, plus the brand-new "New Style" just added - 3 rows total.
        assert_eq!(cloned_styles.len(), 3, "{cloned_styles:?}");

        // The original "slash" spec must be completely untouched.
        let original_styles = run_remote_sql_json(&cfg, "SELECT StyleID FROM Style WHERE SpecKeyName = 'slash';").unwrap();
        assert_eq!(original_styles.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_new_spell_spec_seeds_an_immediately_editable_placeholder_spell() {
        let dir = setup_test_db("create-spell-spec");
        let cfg = test_cfg(&dir);

        let summary = create_new_spell_spec(&cfg, 2, "Earthshatter", 5).unwrap();
        assert_eq!(summary.key_name, "earthshatter_2");
        assert!(summary.is_owned);
        assert!(!summary.has_styles);
        assert!(summary.has_spell_lines);

        let line_rows = run_remote_sql_json(
            &cfg,
            "SELECT Name, Spec, IsBaseLine FROM SpellLine WHERE KeyName = 'earthshatter_2_line';",
        )
        .unwrap();
        assert_eq!(str_field(&line_rows[0], "Name"), "Earthshatter");
        assert_eq!(str_field(&line_rows[0], "Spec"), "earthshatter_2");
        assert_eq!(int_field(&line_rows[0], "IsBaseLine"), 0);

        let spell_rows = run_remote_sql_json(
            &cfg,
            "SELECT ls.Level, s.Name, s.SpellID FROM LineXSpell ls \
             JOIN Spell s ON s.SpellID = ls.SpellID WHERE ls.LineName = 'earthshatter_2_line';",
        )
        .unwrap();
        assert_eq!(spell_rows.len(), 1);
        assert_eq!(int_field(&spell_rows[0], "Level"), 1);
        assert_eq!(str_field(&spell_rows[0], "Name"), "New Spell");

        // The seeded spell must be this class's own independent clone, not a bare reuse of the
        // template spell's own ID - otherwise editing it later would silently edit whatever else
        // still points at that same source Spell row (see spell_ownership.rs's own module doc
        // comment for the exact bug class this guards against).
        let new_spell_id = int_field(&spell_rows[0], "SpellID");
        assert_ne!(new_spell_id, 9001);
        let ownership_rows = run_remote_sql_json(
            &cfg,
            &format!(
                "SELECT SourceSpellID, OwningClassId FROM ClassEditorSpellClone WHERE NewSpellID = {new_spell_id};"
            ),
        )
        .unwrap();
        assert_eq!(int_field(&ownership_rows[0], "SourceSpellID"), 9001);
        assert_eq!(int_field(&ownership_rows[0], "OwningClassId"), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_spell_to_line_extends_an_already_owned_line_at_the_next_level() {
        let dir = setup_test_db("add-spell-owned");
        let cfg = test_cfg(&dir);

        // create_new_spell_spec seeds exactly one spell at Level = 1, already owned by class 2 -
        // the simplest case: nothing left to clone, just append.
        create_new_spell_spec(&cfg, 2, "Earthshatter", 5).unwrap();

        let added = add_spell_to_line(&cfg, 2, "earthshatter_2", "earthshatter_2_line", None).unwrap();
        assert!(added.is_owned);
        assert_eq!(added.level, 2, "one above the seeded spell's own level 1");
        assert_eq!(str_field(&added.fields, "Name"), "New Spell");

        let spell_rows = run_remote_sql_json(
            &cfg,
            "SELECT LineXSpell_ID FROM LineXSpell WHERE LineName = 'earthshatter_2_line';",
        )
        .unwrap();
        assert_eq!(spell_rows.len(), 2, "the original placeholder plus the newly added one");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_spell_to_line_clones_a_still_shared_lines_spec_first() {
        let dir = setup_test_db("add-spell-shared");
        let cfg = test_cfg(&dir);

        // Class 1 has "smite" assigned under its plain, shared KeyName (never cloned through
        // this tool) - adding a spell to its "smite_base" line must clone the whole spec (and
        // its existing spell line/spells) under class 1's own key first, exactly like editing an
        // existing spell already does.
        let added = add_spell_to_line(&cfg, 1, "smite", "smite_base", Some(25)).unwrap();
        assert!(added.is_owned);
        assert_eq!(added.level, 25, "explicit level overrides the default");

        let cloned_spells = run_remote_sql_json(
            &cfg,
            "SELECT Level FROM LineXSpell WHERE LineName = 'smite_1_smite_base' ORDER BY Level;",
        )
        .unwrap();
        // The original Smite (level 1) + Greater Smite (level 10), carried over from the clone,
        // plus the brand-new spell just added at level 25 - 3 rows total.
        assert_eq!(cloned_spells.len(), 3, "{cloned_spells:?}");
        assert_eq!(int_field(&cloned_spells[2], "Level"), 25);

        // The original "smite_base" line must be completely untouched.
        let original_spells =
            run_remote_sql_json(&cfg, "SELECT LineXSpell_ID FROM LineXSpell WHERE LineName = 'smite_base';").unwrap();
        assert_eq!(original_spells.len(), 2);

        // The new spell is cloned from this class's own copy of "Greater Smite" (the line's own
        // highest-level spell after cloning, not the unrelated global template) - a fully
        // independent new SpellID, tracked the same way every other spell clone in this tool is.
        assert_ne!(added.spell_id, 9002);
        let ownership_rows = run_remote_sql_json(
            &cfg,
            &format!("SELECT 1 FROM ClassEditorSpellClone WHERE NewSpellID = {};", added.spell_id),
        )
        .unwrap();
        assert_eq!(ownership_rows.len(), 1, "the new spell must be tracked as this class's own clone");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_spell_line_renames_an_already_owned_line() {
        let dir = setup_test_db("rename-line-owned");
        let cfg = test_cfg(&dir);

        create_new_spell_spec(&cfg, 2, "Earthshatter", 5).unwrap();
        let result = rename_spell_line(&cfg, 2, "earthshatter_2", "earthshatter_2_line", "Ground Fury").unwrap();
        assert!(result.contains("Ground Fury"), "{result}");

        let rows = run_remote_sql_json(&cfg, "SELECT Name FROM SpellLine WHERE KeyName = 'earthshatter_2_line';").unwrap();
        assert_eq!(str_field(&rows[0], "Name"), "Ground Fury");

        // This spec has exactly one spell line (the only shape create_new_spell_spec itself ever
        // produces) - its own display Name (shown in the "Spells" summary list before it's even
        // expanded) must stay in sync with the rename, not silently stuck at "Earthshatter".
        let spec_rows = run_remote_sql_json(&cfg, "SELECT Name FROM Specialization WHERE KeyName = 'earthshatter_2';").unwrap();
        assert_eq!(str_field(&spec_rows[0], "Name"), "Ground Fury");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_spell_line_leaves_the_owning_specs_own_name_alone_when_it_has_several_lines() {
        let dir = setup_test_db("rename-line-multi");
        let cfg = test_cfg(&dir);

        // "smite" already has one real line (smite_base); add a second so this spec genuinely
        // has several - renaming either one must never touch the spec's own Name, since there's
        // no single unambiguous "same name" to sync it to anymore.
        run_remote_sql(
            &cfg,
            "INSERT INTO SpellLine VALUES ('smite_extra', 'Smite Add-on', 'smite', 0, 1);",
        )
        .unwrap();

        rename_spell_line(&cfg, 1, "smite", "smite_base", "Holy Fury").unwrap();

        let spec_rows = run_remote_sql_json(&cfg, "SELECT Name FROM Specialization WHERE KeyName = 'smite_1';").unwrap();
        assert_eq!(str_field(&spec_rows[0], "Name"), "Smite", "unchanged - this spec has several lines, not just the one renamed");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_spell_line_clones_a_still_shared_lines_spec_first() {
        let dir = setup_test_db("rename-line-shared");
        let cfg = test_cfg(&dir);

        // Class 1 has "smite" assigned under its plain, shared KeyName (never cloned through
        // this tool) - renaming its "smite_base" line must clone the whole spec first, exactly
        // like add_spell_to_line already does, never renaming the real shared line itself.
        rename_spell_line(&cfg, 1, "smite", "smite_base", "Holy Fury").unwrap();

        let cloned = run_remote_sql_json(&cfg, "SELECT Name FROM SpellLine WHERE KeyName = 'smite_1_smite_base';").unwrap();
        assert_eq!(str_field(&cloned[0], "Name"), "Holy Fury");

        let original = run_remote_sql_json(&cfg, "SELECT Name FROM SpellLine WHERE KeyName = 'smite_base';").unwrap();
        assert_eq!(str_field(&original[0], "Name"), "Smiting Spells", "the original shared line must be untouched");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_spell_line_refuses_an_empty_name() {
        let dir = setup_test_db("rename-line-empty");
        let cfg = test_cfg(&dir);

        create_new_spell_spec(&cfg, 2, "Earthshatter", 5).unwrap();
        let result = rename_spell_line(&cfg, 2, "earthshatter_2", "earthshatter_2_line", "   ");
        assert!(result.is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A fresh temp DB with just a stat-field-rich `Spell` table - `spell_type_field_stats` only
    /// ever reads from `Spell`, so this is lighter-weight than `setup_test_db`'s own full schema.
    fn setup_spell_stats_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("daoc-class-editor-spell-stats-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let db = dir.join("data/opendaoc.sqlite3.db");
        let sql = "
            CREATE TABLE Spell (SpellID INTEGER, Name TEXT, Type TEXT, Damage REAL, DamageType INTEGER,
                Value REAL, Range INTEGER, Radius INTEGER, Duration INTEGER, Frequency INTEGER,
                Power INTEGER, CastTime INTEGER, RecastDelay INTEGER, Icon INTEGER, ClientEffect INTEGER,
                Target TEXT);
            INSERT INTO Spell VALUES (1, 'Minor Haste', 'SpeedEnhancement', NULL, NULL, 115, 1500, 0, 600, 0, 0, 2, 0, 400, 400, 'Self');
            INSERT INTO Spell VALUES (2, 'Haste', 'SpeedEnhancement', NULL, NULL, 125, 1500, 0, 600, 0, 0, 2, 0, 400, 400, 'Self');
            INSERT INTO Spell VALUES (3, 'Greater Haste', 'SpeedEnhancement', NULL, NULL, 135, 1500, 0, 600, 0, 0, 2, 0, 400, 400, 'Self');
            INSERT INTO Spell VALUES (4, 'Group Haste', 'SpeedEnhancement', NULL, NULL, 125, 1500, 0, 600, 0, 0, 3, 0, 410, 410, 'Group');
            INSERT INTO Spell VALUES (5, 'Ancient Spell', 'DirectDamage', 50, 1, NULL, 1500, 0, 0, 0, 0, 2, 0, 100, 100, 'Enemy');
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
    fn spell_type_field_stats_suggests_the_lowest_real_value_for_a_ramp_field() {
        let dir = setup_spell_stats_db("ramp-field-lowest");
        let cfg = test_cfg(&dir);

        let stats = spell_type_field_stats(&cfg, "SpeedEnhancement").unwrap();
        let value_stat = &stats["Value"];
        assert_eq!(value_stat.default, "115", "{value_stat:?}");
        assert_eq!(value_stat.options, vec!["115", "125", "135"], "ascending and deduped");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spell_type_field_stats_suggests_the_most_common_real_value_for_a_categorical_field() {
        let dir = setup_spell_stats_db("categorical-field-mode");
        let cfg = test_cfg(&dir);

        let stats = spell_type_field_stats(&cfg, "SpeedEnhancement").unwrap();
        let target_stat = &stats["Target"];
        assert_eq!(target_stat.default, "Self", "3 of 4 SpeedEnhancement spells target Self");
        assert_eq!(target_stat.options, vec!["Self", "Group"], "most common first");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spell_type_field_stats_treats_zero_as_a_real_meaningful_value_not_a_placeholder() {
        let dir = setup_spell_stats_db("zero-is-meaningful");
        let cfg = test_cfg(&dir);

        let stats = spell_type_field_stats(&cfg, "SpeedEnhancement").unwrap();
        // Every SpeedEnhancement fixture spell has Radius=0 (single-target, not an AoE) - that's
        // the real, typical value to suggest, not something to silently discard.
        assert_eq!(stats["Radius"].default, "0");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spell_type_field_stats_omits_a_field_no_spell_of_this_type_ever_set() {
        let dir = setup_spell_stats_db("omits-unset-field");
        let cfg = test_cfg(&dir);

        // The only DirectDamage fixture spell has Value=NULL (damage spells don't use Value the
        // way a buff does) - Damage is the real stat-worthy field there, Value has nothing to
        // suggest at all and must be omitted entirely, not default to some misleading "0".
        let stats = spell_type_field_stats(&cfg, "DirectDamage").unwrap();
        assert_eq!(stats["Damage"].default, "50");
        assert!(!stats.contains_key("Value"), "{stats:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spell_type_field_stats_is_empty_for_a_type_with_no_real_spells_at_all() {
        let dir = setup_spell_stats_db("unknown-type");
        let cfg = test_cfg(&dir);

        let stats = spell_type_field_stats(&cfg, "SomeTypeNobodyUsesYet").unwrap();
        assert!(stats.is_empty(), "{stats:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spell_type_field_stats_still_works_when_this_forks_spell_table_is_missing_a_listed_column() {
        // A real, found bug: this used to build one SQL query listing every SPELL_TYPE_STAT_FIELDS
        // column unconditionally - a server fork's own Spell table missing even one of them (this
        // project's own fixture schema was itself missing Pulse/PulsePower the moment those were
        // added to the list) broke this whole feature outright with a raw "no such column" SQL
        // error, for every Type, not just the one actually missing something. Must instead just
        // quietly suggest nothing for whichever listed field(s) this fork's own schema doesn't
        // have at all, while still correctly suggesting everything it does.
        let dir = setup_spell_stats_db("missing-column");
        let cfg = test_cfg(&dir);

        let stats = spell_type_field_stats(&cfg, "SpeedEnhancement").unwrap();
        assert!(!stats.contains_key("Pulse"), "{stats:?}");
        assert!(!stats.contains_key("PulsePower"), "{stats:?}");
        assert_eq!(stats["Value"].default, "115", "every other real column must still work fine");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_new_combat_spec_avoids_a_key_collision_with_an_earlier_same_named_spec() {
        let dir = setup_test_db("create-combat-spec-collision");
        let cfg = test_cfg(&dir);

        let first = create_new_combat_spec(&cfg, 2, "Earthshatter", 1).unwrap();
        let second = create_new_combat_spec(&cfg, 2, "Earthshatter", 1).unwrap();
        assert_eq!(first.key_name, "earthshatter_2");
        assert_eq!(second.key_name, "earthshatter2_2");
        assert_ne!(first.key_name, second.key_name);

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
            spell_line_selection: Vec::new(),
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
            spell_line_selection: Vec::new(),
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
            spell_line_selection: Vec::new(),
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
            spell_line_selection: Vec::new(),
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
            spell_line_selection: Vec::new(),
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
            spell_line_selection: Vec::new(),
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
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        // Resubmit it exactly as the Edit flow would for an unchanged spec: source_key is now its
        // own already-cloned key, not the original "smite".
        let resubmit = vec![SpecAssignment {
            source_key: "frostbolt_tree_64".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
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
    fn resubmitting_a_spec_with_itself_as_source_still_applies_a_spell_line_customization() {
        // A real, reported bug found live: opening an already-deployed spec's "customize spell
        // lines" picker and unchecking a specific spell did not change source_key, new_key,
        // new_name, or level_acquired at all (only spell_line_selection does) - generating,
        // deploying, even restarting the server did nothing at all, since the self-source skip
        // above (correctly protecting against the real data-loss bug the previous test covers)
        // unconditionally discarded spell_line_selection without ever looking at it. A self
        // -source redeploy with a real customization must still apply it - via an in-place prune
        // of this key's own existing data, never the destructive delete-then-reclone path (there
        // is no separate source left to safely reclone from here).
        let dir = setup_test_db("resubmit-self-source-prune");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        // The Edit flow's own picker always reads back *current* state (via `list_line_spells`)
        // to build its checkboxes - which, now that spells are independently cloned, means the
        // IDs it offers to exclude are this class's own already-cloned ones, not the original
        // source spec's. Look up the clone of "Greater Smite" (originally 9002, level 10) the
        // same way the real picker would.
        let current_spells = run_remote_sql_json(
            &cfg,
            "SELECT SpellID FROM LineXSpell WHERE LineName='frostbolt_tree_64_smite_base' AND Level=10;",
        )
        .unwrap();
        let cloned_greater_smite_id = int_field(&current_spells[0], "SpellID");

        // Resubmit it exactly as the Edit flow's picker would after excluding one spell from the
        // line - source_key is its own already-cloned key (self-source), but this time with a
        // real, non-empty spell_line_selection.
        let resubmit = vec![SpecAssignment {
            source_key: "frostbolt_tree_64".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: vec![SpellLineSelection {
                source_line_key: "frostbolt_tree_64_smite_base".to_string(),
                included: true,
                excluded_spell_ids: vec![cloned_greater_smite_id],
            }],
        }];
        let log = apply_class_specs(&cfg, 64, &resubmit, &[]).unwrap();
        assert!(
            log.contains("removed 1 excluded spell(s) from 'frostbolt_tree_64_smite_base'"),
            "log should mention the prune: {log}"
        );

        let spells = run_remote_sql_json(
            &cfg,
            "SELECT Spell.Name FROM LineXSpell JOIN Spell ON Spell.SpellID = LineXSpell.SpellID \
             WHERE LineXSpell.LineName='frostbolt_tree_64_smite_base';",
        )
        .unwrap();
        assert_eq!(spells.len(), 1, "{spells:?}");
        assert_eq!(str_field(&spells[0], "Name"), "Smite", "9002/Greater Smite must be excluded");

        // The spec and its line must still exist - only the one excluded spell is gone.
        let specs = list_specializations(&cfg).unwrap();
        assert!(specs.iter().any(|s| s.key_name == "frostbolt_tree_64"));
        let lines = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM SpellLine WHERE KeyName='frostbolt_tree_64_smite_base';",
        )
        .unwrap();
        assert!(!lines.is_empty(), "the line itself must survive");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resubmitting_a_spec_with_itself_as_source_can_remove_a_whole_line_in_place() {
        // The exact live incident this fix recovered from: removing a whole spell line (every
        // spell in it, not just some) via the customizer on an already-deployed spec.
        let dir = setup_test_db("resubmit-self-source-prune-whole-line");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO SpellLine VALUES ('smite_advanced', 'Advanced Smiting', 'smite', 0, 1);\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('smite_advanced', 9099, 5, 'seed-advanced-1');",
        )
        .unwrap();

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();
        let lines_before = run_remote_sql_json(
            &cfg,
            "SELECT KeyName FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert_eq!(lines_before.len(), 2, "{lines_before:?}");

        // Resubmit self-sourced, keeping only "smite_base"'s own clone - the other line is simply
        // absent from the selection, matching clone_spec's own "not mentioned = excluded"
        // convention.
        let resubmit = vec![SpecAssignment {
            source_key: "frostbolt_tree_64".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: vec![SpellLineSelection {
                source_line_key: "frostbolt_tree_64_smite_base".to_string(),
                included: true,
                excluded_spell_ids: Vec::new(),
            }],
        }];
        let log = apply_class_specs(&cfg, 64, &resubmit, &[]).unwrap();
        assert!(
            log.contains("removed the whole 'frostbolt_tree_64_smite_advanced' spell line"),
            "{log}"
        );

        let lines_after = run_remote_sql_json(
            &cfg,
            "SELECT KeyName FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert_eq!(lines_after.len(), 1, "{lines_after:?}");
        assert_eq!(
            str_field(&lines_after[0], "KeyName"),
            "frostbolt_tree_64_smite_base"
        );
        let orphaned_spells = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM LineXSpell WHERE LineName='frostbolt_tree_64_smite_advanced';",
        )
        .unwrap();
        assert!(orphaned_spells.is_empty(), "its spells must be cleaned up too");

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
            spell_line_selection: Vec::new(),
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
        let first_spell_id = int_field(&spells[0], "SpellID");
        let second_spell_id = int_field(&spells[1], "SpellID");
        assert_ne!(
            first_spell_id, 9001,
            "each cloned spell must be its own independent Spell row, not a pointer back at the \
             source's own SpellID"
        );
        assert_ne!(second_spell_id, 9002, "same for the second spell");
        assert_eq!(int_field(&spells[0], "Level"), 1);
        assert_eq!(int_field(&spells[1], "Level"), 10);

        // The clone's own independent Spell rows must still carry the same real data the source
        // spells had (Name, SpellGroup, ...) - only the ID itself differs.
        let cloned_spells = run_remote_sql_json(
            &cfg,
            &format!(
                "SELECT SpellID, Name FROM Spell WHERE SpellID IN ({first_spell_id}, {second_spell_id}) ORDER BY SpellID;"
            ),
        )
        .unwrap();
        assert_eq!(cloned_spells.len(), 2, "{cloned_spells:?}");
        assert!(
            cloned_spells.iter().any(|s| str_field(s, "Name") == "Smite"),
            "{cloned_spells:?}"
        );
        assert!(
            cloned_spells
                .iter()
                .any(|s| str_field(s, "Name") == "Greater Smite"),
            "{cloned_spells:?}"
        );

        // Original spell line/assignments, and the original Spell rows themselves, untouched.
        let original_spells = run_remote_sql_json(
            &cfg,
            "SELECT SpellID FROM LineXSpell WHERE LineName='smite_base';",
        )
        .unwrap();
        assert_eq!(original_spells.len(), 2);
        let original_spell_rows = run_remote_sql_json(
            &cfg,
            "SELECT Name FROM Spell WHERE SpellID IN (9001, 9002) ORDER BY SpellID;",
        )
        .unwrap();
        assert_eq!(
            original_spell_rows.len(),
            2,
            "the source Spell rows must still exist, untouched: {original_spell_rows:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_clone_of_a_base_line_is_never_itself_a_base_line() {
        // A real, reproduced bug found live: cloning "Mind Twisting" (a genuine base line for
        // the real Sorcerer - that class's own design grants it free by character level alone,
        // needing no spec investment) carried IsBaseLine=1 over verbatim. The real server's own
        // granting logic (Specialization.cs) sets a base line's effective level straight to the
        // player's *character* level, completely ignoring whatever was actually trained in the
        // spec - so the clone's spec points showed correctly on the character sheet, but every
        // spell up to the character's own level was silently granted regardless, and the
        // in-game trainer window had nothing to preview/train (there was never anything
        // point-gated to show in the first place). A clone must always be a regular, point-gated
        // line, no matter what the source's own IsBaseLine was.
        let dir = setup_test_db("clone-of-base-line");
        let cfg = test_cfg(&dir);
        run_remote_sql(&cfg, "UPDATE SpellLine SET IsBaseLine = 1 WHERE KeyName = 'smite_base';")
            .unwrap();

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let lines = run_remote_sql_json(
            &cfg,
            "SELECT IsBaseLine FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(
            int_field(&lines[0], "IsBaseLine"),
            0,
            "the clone must never be a base line, even though its source is: {lines:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spell_line_selection_excludes_an_entire_line() {
        // Reproduces a real, reported problem: "Mind Magic" has two real SpellLines ("Mind
        // Twisting" and "Domination"), and "Enhancement" has five, one per real class that
        // shares it - cloning the whole spec previously pulled in every single line, with no way
        // to pick just the one actually wanted. A non-empty selection that only lists one of the
        // source's lines (and marks it included) must clone only that one; the other line, never
        // mentioned at all, must not be cloned.
        let dir = setup_test_db("spell-line-select-exclude");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO SpellLine VALUES ('smite_advanced', 'Advanced Smiting', 'smite', 0, 1);\
             INSERT INTO LineXSpell (LineName, SpellID, Level, LineXSpell_ID) VALUES ('smite_advanced', 9099, 5, 'seed-advanced-1');",
        )
        .unwrap();

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: vec![SpellLineSelection {
                source_line_key: "smite_base".to_string(),
                included: true,
                excluded_spell_ids: Vec::new(),
            }],
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let lines = run_remote_sql_json(
            &cfg,
            "SELECT KeyName FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert_eq!(
            lines.len(),
            1,
            "only the selected line must be cloned, the unmentioned one must not: {lines:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spell_line_selection_excludes_specific_spells_within_an_included_line() {
        // "I would like to be able to remove specific spells from the line we copy over as
        // well" - an included line can still drop individual spells, independent of whether
        // whole other lines were excluded.
        let dir = setup_test_db("spell-line-select-exclude-spell");
        let cfg = test_cfg(&dir);

        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: vec![SpellLineSelection {
                source_line_key: "smite_base".to_string(),
                included: true,
                excluded_spell_ids: vec![9002],
            }],
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let lines = run_remote_sql_json(
            &cfg,
            "SELECT KeyName FROM SpellLine WHERE Spec='frostbolt_tree_64';",
        )
        .unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let new_line_key = str_field(&lines[0], "KeyName");

        let spells = run_remote_sql_json(
            &cfg,
            &format!(
                "SELECT Spell.SpellID, Spell.Name FROM LineXSpell \
                 JOIN Spell ON Spell.SpellID = LineXSpell.SpellID \
                 WHERE LineXSpell.LineName='{new_line_key}';"
            ),
        )
        .unwrap();
        assert_eq!(spells.len(), 1, "{spells:?}");
        assert_eq!(
            str_field(&spells[0], "Name"),
            "Smite",
            "the excluded spell (Greater Smite, 9002) must be gone, leaving only the clone of 9001"
        );
        assert_ne!(
            int_field(&spells[0], "SpellID"),
            9001,
            "the remaining spell must be its own independent clone, not the source's own row"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lists_a_specs_spell_lines_and_a_lines_spells() {
        let dir = setup_test_db("list-spell-lines");
        let cfg = test_cfg(&dir);

        let lines = list_spec_spell_lines(&cfg, "smite").unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].key_name, "smite_base");
        assert_eq!(lines[0].name, "Smiting Spells");
        assert!(!lines[0].is_base_line, "the fixture's own smite_base row has IsBaseLine=0");

        let spells = list_line_spells(&cfg, "smite_base").unwrap();
        assert_eq!(spells.len(), 2, "{spells:?}");
        assert_eq!(spells[0].spell_id, 9001);
        assert_eq!(spells[0].level, 1);
        assert_eq!(spells[0].spell_group, 500, "shares a progression with Greater Smite");
        assert_eq!(spells[1].spell_id, 9002);
        assert_eq!(spells[1].level, 10);
        assert_eq!(spells[1].spell_group, 500);

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
                spell_line_selection: Vec::new(),
            },
            SpecAssignment {
                source_key: "smite".to_string(),
                new_key: "frostbolt_tree".to_string(),
                new_name: "Frostbolt Tree".to_string(),
                level_acquired: 0,
                spell_line_selection: Vec::new(),
            },
        ];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        // Redeploy with only "Earthshatter" kept; "Frostbolt Tree" was removed from the list in
        // the UI, and its previously-assigned key is passed back as the Edit flow would.
        let kept = vec![assignments[0].clone()];
        let previous_keys = vec![
            "earthshatter_64".to_string(),
            "frostbolt_tree_64".to_string(),
        ];
        let log = apply_class_specs(&cfg, 64, &kept, &previous_keys).unwrap();
        assert!(
            log.contains("removed 'frostbolt_tree_64'"),
            "log should mention the removal: {log}"
        );

        let class_specs = list_class_specs(&cfg, 64).unwrap();
        assert!(
            class_specs
                .iter()
                .any(|c| c.spec_key_name == "earthshatter_64"),
            "the kept spec must still be assigned: {class_specs:?}"
        );
        assert!(
            !class_specs
                .iter()
                .any(|c| c.spec_key_name == "frostbolt_tree_64"),
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
    fn removing_a_spec_scrubs_it_from_characters_so_they_can_still_log_in() {
        // Reproduces a real crash found live immediately after the removal feature shipped: a
        // character who already had points in a spec that gets removed still carries its now
        // -deleted KeyName in their own saved SerializedSpecs - SkillBase.GetSpecialization
        // null-derefs on a key with no matching row, crashing login entirely, before the
        // character ever reaches the free respec token this same deploy grants. The dangling
        // token must be stripped (and only that exact token - every other spec's own points,
        // and any other character's data entirely, must survive untouched).
        let dir = setup_test_db("scrub-on-removal");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO DOLCharacters (Name, Class, SerializedSpecs) VALUES ('Aeligord', 64, '');\
             INSERT INTO DOLCharacters (Name, Class, SerializedSpecs) VALUES ('SomeoneElse', 23, '');",
        )
        .unwrap();

        let assignments = vec![
            SpecAssignment {
                source_key: "slash".to_string(),
                new_key: "earthshatter".to_string(),
                new_name: "Earthshatter".to_string(),
                level_acquired: 0,
                spell_line_selection: Vec::new(),
            },
            SpecAssignment {
                source_key: "smite".to_string(),
                new_key: "frostbolt_tree".to_string(),
                new_name: "Frostbolt Tree".to_string(),
                level_acquired: 0,
                spell_line_selection: Vec::new(),
            },
        ];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        run_remote_sql(
            &cfg,
            "UPDATE DOLCharacters SET SerializedSpecs = 'earthshatter_64|3;frostbolt_tree_64|2' WHERE Name = 'Aeligord';",
        )
        .unwrap();
        run_remote_sql(
            &cfg,
            "UPDATE DOLCharacters SET SerializedSpecs = 'somethingelse_23|5' WHERE Name = 'SomeoneElse';",
        )
        .unwrap();

        // Redeploy with "Frostbolt Tree" removed from the list.
        let kept = vec![assignments[0].clone()];
        let previous_keys = vec![
            "earthshatter_64".to_string(),
            "frostbolt_tree_64".to_string(),
        ];
        let log = apply_class_specs(&cfg, 64, &kept, &previous_keys).unwrap();
        assert!(
            log.contains("removed stale 'frostbolt_tree_64' points from"),
            "log should mention the scrub: {log}"
        );

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT SerializedSpecs FROM DOLCharacters WHERE Name = 'Aeligord';",
        )
        .unwrap();
        assert_eq!(
            str_field(&rows[0], "SerializedSpecs"),
            "earthshatter_64|3",
            "the removed spec's token must be gone, the kept spec's points must survive untouched"
        );

        // A character on a different class entirely must never be touched.
        let other = run_remote_sql_json(
            &cfg,
            "SELECT SerializedSpecs FROM DOLCharacters WHERE Name = 'SomeoneElse';",
        )
        .unwrap();
        assert_eq!(
            str_field(&other[0], "SerializedSpecs"),
            "somethingelse_23|5"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_plain_shared_keys_own_specialization_row_is_never_deleted_by_the_removal_pass() {
        // A plain/shared key's own Specialization row (e.g. "Sword", a real spec assigned
        // directly via NEVER_CLONE_SPECS, with no class-suffix at all) must never be deleted,
        // even when it's dropped from this class's own assignment list - the same KeyName is very
        // likely still in active use by other real classes. Only this one class's own assignment
        // of it is ever safe to undo (see the dedicated test below for that).
        let dir = setup_test_db("plain-key-specialization-untouched");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO ClassXSpecialization VALUES (64, 'slash', 0);",
        )
        .unwrap();

        apply_class_specs(&cfg, 64, &[], &["slash".to_string()]).unwrap();

        // "slash" (the real, unsuffixed shared spec) must still exist - other classes may use it.
        let specs = list_specializations(&cfg).unwrap();
        assert!(
            specs.iter().any(|s| s.key_name == "slash"),
            "a plain shared key's own Specialization row must never be deleted: {specs:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removing_a_plain_shared_key_unassigns_it_from_only_this_class() {
        // Reproduces a real bug found live: removing "Sword"/"Axe"/"Hammer"/"Thrown Weapons"
        // (all assigned via NEVER_CLONE_SPECS under their real, unsuffixed KeyName) from a
        // class's own assignment list and redeploying left every one of them still fully offered
        // by the class - still on every character's sheet, still in the in-game trainer window.
        // The old removal pass skipped *any* key without this class's own clone suffix entirely,
        // reasoning (correctly) that the spec might be shared by other classes, but wrongly
        // concluding there was nothing at all safe to clean up. There is: this class's own
        // ClassXSpecialization row, and any Style rows locked specifically to this class by
        // grant_styles_for_uncloneable_spec - just never the spec's own shared data, and never
        // another class's own locked styles.
        //
        // Also reproduces a second, related bug found live: even after fixing the removal itself,
        // an existing character who already trained points in "Sword" kept seeing and using it
        // forever regardless - GamePlayer.HandleCharacterSkills blindly re-adds *every* key in a
        // character's own saved SerializedSpecs on login, with no regard at all for whether the
        // class's current ClassXSpecialization still offers it. The removed key must be scrubbed
        // from affected characters too, the same as a removed clone already is.
        let dir = setup_test_db("remove-plain-shared-key");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO ClassXSpecialization VALUES (64, 'slash', 0);\
             INSERT INTO Style VALUES (NULL, 9, 'Some Other Class Style', 'slash', 5, 300, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 900);\
             INSERT INTO Style VALUES (NULL, 64, 'This Class Own Style', 'slash', 5, 300, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 901);\
             INSERT INTO DOLCharacters (Name, Class, SerializedSpecs) VALUES ('Grolf', 64, 'slash|8;smite|3');",
        )
        .unwrap();

        let log = apply_class_specs(&cfg, 64, &[], &["slash".to_string()]).unwrap();
        assert!(
            log.contains("unassigned 'slash' from this class only"),
            "log should explain the unassignment: {log}"
        );
        assert!(
            log.contains("removed stale 'slash' points from 1 character"),
            "log should mention the scrub: {log}"
        );

        let class_specs = list_class_specs(&cfg, 64).unwrap();
        assert!(
            !class_specs.iter().any(|c| c.spec_key_name == "slash"),
            "this class must no longer offer 'slash': {class_specs:?}"
        );

        // The spec's own shared row, and the OTHER class's own locked style, must survive.
        let specs = list_specializations(&cfg).unwrap();
        assert!(specs.iter().any(|s| s.key_name == "slash"), "{specs:?}");
        let other_class_style = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM Style WHERE SpecKeyName='slash' AND ClassId=9;",
        )
        .unwrap();
        assert!(
            !other_class_style.is_empty(),
            "another class's own locked style must never be touched"
        );

        // This class's own locked style duplicate must be gone.
        let own_style = run_remote_sql_json(
            &cfg,
            "SELECT 1 FROM Style WHERE SpecKeyName='slash' AND ClassId=64;",
        )
        .unwrap();
        assert!(
            own_style.is_empty(),
            "this class's own granted style duplicate must be cleaned up"
        );

        // The character's stale points must be scrubbed, but an unrelated spec's points survive.
        let rows = run_remote_sql_json(
            &cfg,
            "SELECT SerializedSpecs FROM DOLCharacters WHERE Name='Grolf';",
        )
        .unwrap();
        assert_eq!(str_field(&rows[0], "SerializedSpecs"), "smite|3");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn assigning_a_never_clone_spec_keeps_its_real_name_uncloned() {
        // Reproduces a real bug found live: cloning "Stealth"/"Sword" for Nosferatu under new
        // KeyNames left both fully trained on the character sheet, but the character could not
        // actually stealth and had lost her weapon's entire spec-driven damage bonus - the server
        // hardcodes these mechanics to the exact literal name (NEVER_CLONE_SPECS's own doc
        // comment has the citations). Assigning straight from a real, denylisted spec must keep
        // it assigned under its own real name - never clone/rename it, no matter what new_name
        // was requested.
        //
        // Also reproduces a second, separate real bug found live immediately after the first fix
        // shipped: a brand-new class assigned "Sword" this way trained it correctly on the
        // character sheet, but the in-game trainer window showed zero combat styles at all - real
        // weapon specs lock their styles to specific existing real ClassIds
        // (confirmed live: Sword's are locked to Warrior/Hunter/Thane/etc., Spear's to Hunter
        // only), and `Specialization.GetStylesForLiving` resolves by the player's own *exact*
        // ClassId with no inheritance - a brand-new class's ID was never going to be one of them.
        // The style set already locked to "Warrior" (ClassId 22 here) must be duplicated onto
        // this new class (64), with every chain-style ID preserved unchanged (confirmed live:
        // every real class's own copy of the same conceptual style shares the same Style.ID).
        let dir = setup_test_db("never-clone-fresh-assign");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
             VALUES ('Sword', 'Sword', 'Sword weapons', 400, NULL, 0);\
             INSERT INTO Style VALUES (NULL, 22, 'Whirling Blade', 'Sword', 2, 500, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 146);\
             INSERT INTO Style VALUES (NULL, 22, 'Northern Lights', 'Sword', 8, 501, 5, 2, 1, 146, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 149);\
             INSERT INTO Style VALUES (NULL, 25, 'Whirling Blade', 'Sword', 2, 500, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 146);\
             INSERT INTO Style VALUES (NULL, 25, 'Northern Lights', 'Sword', 8, 501, 5, 2, 1, 146, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 149);",
        )
        .unwrap();

        let assignments = vec![SpecAssignment {
            source_key: "Sword".to_string(),
            new_key: "earthshatterblade".to_string(),
            new_name: "Earthshatter Blade".to_string(),
            level_acquired: 3,
            spell_line_selection: Vec::new(),
        }];
        let log = apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();
        assert!(
            log.contains("kept 'Sword' assigned under its own real name"),
            "log should explain why: {log}"
        );
        assert!(
            log.contains("granted 2 of 'Sword's own combat style(s)"),
            "log should mention the style grant: {log}"
        );

        let class_specs = list_class_specs(&cfg, 64).unwrap();
        assert_eq!(class_specs.len(), 1, "{class_specs:?}");
        assert_eq!(class_specs[0].spec_key_name, "Sword");
        assert_eq!(class_specs[0].level_acquired, 3);

        let specs = list_specializations(&cfg).unwrap();
        assert!(
            !specs.iter().any(|s| s.key_name == "earthshatterblade_64"),
            "no clone must ever be created for a denylisted spec: {specs:?}"
        );

        let styles = run_remote_sql_json(
            &cfg,
            "SELECT Name, ID, OpeningRequirementValue FROM Style WHERE SpecKeyName='Sword' AND ClassId=64 ORDER BY Name;",
        )
        .unwrap();
        assert_eq!(
            styles.len(),
            2,
            "the new class must get its own locked copy of every style: {styles:?}"
        );
        assert_eq!(int_field(&styles[0], "ID"), 149, "Northern Lights");
        assert_eq!(
            int_field(&styles[0], "OpeningRequirementValue"),
            146,
            "the chain prerequisite must still resolve - the same ID convention real classes already share, no remap needed"
        );
        assert_eq!(int_field(&styles[1], "ID"), 146, "Whirling Blade");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn granting_styles_for_an_uncloneable_spec_is_a_no_op_once_the_class_has_its_own() {
        // Redeploying (the Edit flow resubmits every currently-assigned spec every time) must not
        // duplicate the class's own style rows a second time.
        let dir = setup_test_db("grant-styles-idempotent");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
             VALUES ('Sword', 'Sword', 'Sword weapons', 400, NULL, 0);\
             INSERT INTO Style VALUES (NULL, 22, 'Whirling Blade', 'Sword', 2, 500, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 146);",
        )
        .unwrap();

        let first = grant_styles_for_uncloneable_spec(&cfg, "Sword", 64).unwrap();
        assert!(first.contains("granted 1"), "{first}");
        let second = grant_styles_for_uncloneable_spec(&cfg, "Sword", 64).unwrap();
        assert_eq!(second, "", "a second call must be a silent no-op: {second}");

        let count = run_remote_sql_json(
            &cfg,
            "SELECT COUNT(*) as c FROM Style WHERE SpecKeyName='Sword' AND ClassId=64;",
        )
        .unwrap();
        assert_eq!(int_field(&count[0], "c"), 1, "must not duplicate on a second call");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn granting_styles_for_an_uncloneable_spec_is_a_no_op_if_a_universal_bucket_already_exists() {
        // A spec whose styles already have a ClassId=0 "universal" row needs nothing granted -
        // every class, including a brand-new one, already falls back to seeing it.
        let dir = setup_test_db("grant-styles-universal-noop");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
             VALUES ('Flexible', 'Flexible', 'Flexible weapons', 400, NULL, 0);\
             INSERT INTO Style VALUES (NULL, 0, 'Some Universal Style', 'Flexible', 2, 500, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 1);",
        )
        .unwrap();

        let log = grant_styles_for_uncloneable_spec(&cfg, "Flexible", 64).unwrap();
        assert_eq!(log, "", "no grant needed when a universal bucket exists: {log}");

        let count = run_remote_sql_json(
            &cfg,
            "SELECT COUNT(*) as c FROM Style WHERE SpecKeyName='Flexible' AND ClassId=64;",
        )
        .unwrap();
        assert_eq!(int_field(&count[0], "c"), 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn granting_styles_for_an_uncloneable_spec_is_a_no_op_if_it_has_no_styles_at_all() {
        // Real weapon-adjacent specs like Parry/Envenom/Thrown Weapons grant no combat styles at
        // all - nothing to do, and no error, if the spec simply has zero Style rows.
        let dir = setup_test_db("grant-styles-no-styles-noop");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
             VALUES ('Parry', 'Parry', 'Parry chance', 400, NULL, 0);",
        )
        .unwrap();

        let log = grant_styles_for_uncloneable_spec(&cfg, "Parry", 64).unwrap();
        assert_eq!(log, "", "{log}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn assigning_a_never_clone_spec_heals_a_previous_bad_clone() {
        // The exact live incident this fix recovered from: a previous deploy (before
        // NEVER_CLONE_SPECS existed) cloned "Sword" into "sword_64", carrying its own
        // class-exclusive combat style along, and a character had already trained points under
        // that broken clone. Redeploying today (the Edit flow reading back "sword_64" as the
        // current source, same as it would for any other already-assigned spec) must heal this:
        // migrate the character's points onto the real "Sword" key (preserving the exact value),
        // move the clone's own style onto "Sword" too (never just discard it), and leave the
        // class assigned to "Sword" only.
        let dir = setup_test_db("never-clone-heal");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
             VALUES ('Sword', 'Sword', 'Sword weapons', 400, NULL, 0);\
             INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
             VALUES ('sword_64', 'Sword', 'Sword weapons', 400, NULL, 0);\
             INSERT INTO ClassXSpecialization (ClassID, SpecKeyName, LevelAcquired) VALUES (64, 'sword_64', 0);\
             INSERT INTO Style VALUES (NULL, 64, 'Bloodrage', 'sword_64', 10, 300, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);\
             INSERT INTO DOLCharacters (Name, Class, SerializedSpecs) VALUES ('Lillith', 64, 'sword_64|9;stealth_64|1');",
        )
        .unwrap();

        let assignments = vec![SpecAssignment {
            source_key: "sword_64".to_string(),
            new_key: "sword".to_string(),
            new_name: "Sword".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        let log = apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();
        assert!(
            log.contains("migrated 'sword_64' points to 'Sword'"),
            "{log}"
        );

        let class_specs = list_class_specs(&cfg, 64).unwrap();
        assert_eq!(class_specs.len(), 1, "{class_specs:?}");
        assert_eq!(class_specs[0].spec_key_name, "Sword");

        let specs = list_specializations(&cfg).unwrap();
        assert!(
            !specs.iter().any(|s| s.key_name == "sword_64"),
            "the stale clone's own row must be fully deleted: {specs:?}"
        );

        // The clone's own style must survive - moved onto the real spec's row, not discarded,
        // still locked to this exact class.
        let styles = run_remote_sql_json(
            &cfg,
            "SELECT Name, ClassId FROM Style WHERE SpecKeyName = 'Sword';",
        )
        .unwrap();
        assert_eq!(styles.len(), 1, "{styles:?}");
        assert_eq!(str_field(&styles[0], "Name"), "Bloodrage");
        assert_eq!(int_field(&styles[0], "ClassId"), 64);

        let rows = run_remote_sql_json(
            &cfg,
            "SELECT SerializedSpecs FROM DOLCharacters WHERE Name = 'Lillith';",
        )
        .unwrap();
        assert_eq!(
            str_field(&rows[0], "SerializedSpecs"),
            "Sword|9;stealth_64|1",
            "the real spec's points must survive the migration, untouched specs must be left alone"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_class_spec_summaries_flags_combat_and_spell_specs_and_ownership() {
        let dir = setup_test_db("spec-summaries");
        let cfg = test_cfg(&dir);
        // Class 64 isn't suffix-owned for either spec, and has no `ClassId = 64` style rows of
        // its own yet - both specs must come back un-owned, correctly flagged by what they
        // actually grant ("slash" has styles via the fixture's own Pulverize/Backlash, "smite"
        // has a spell line).
        run_remote_sql(
            &cfg,
            "INSERT INTO ClassXSpecialization VALUES (64, 'slash', 0);\
             INSERT INTO ClassXSpecialization VALUES (64, 'smite', 5);",
        )
        .unwrap();

        let summaries = list_class_spec_summaries(&cfg, 64).unwrap();
        assert_eq!(summaries.len(), 2, "{summaries:?}");
        let slash = summaries.iter().find(|s| s.key_name == "slash").unwrap();
        assert!(slash.has_styles);
        assert!(!slash.has_spell_lines);
        assert!(!slash.is_owned);
        let smite = summaries.iter().find(|s| s.key_name == "smite").unwrap();
        assert!(!smite.has_styles);
        assert!(smite.has_spell_lines);
        assert!(!smite.is_owned);
        assert_eq!(smite.level_acquired, 5);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_class_spec_summaries_marks_a_suffix_owned_spec_as_owned() {
        let dir = setup_test_db("spec-summaries-owned");
        let cfg = test_cfg(&dir);
        let assignments = vec![SpecAssignment {
            source_key: "smite".to_string(),
            new_key: "frostbolt_tree".to_string(),
            new_name: "Frostbolt Tree".to_string(),
            level_acquired: 0,
            spell_line_selection: Vec::new(),
        }];
        apply_class_specs(&cfg, 64, &assignments, &[]).unwrap();

        let summaries = list_class_spec_summaries(&cfg, 64).unwrap();
        let owned = summaries.iter().find(|s| s.key_name == "frostbolt_tree_64").unwrap();
        assert!(owned.is_owned);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_styles_for_spec_prefers_this_classs_own_locked_styles_over_anyone_elses() {
        let dir = setup_test_db("list-styles-for-spec");
        let cfg = test_cfg(&dir);
        // "slash" already has Pulverize (ClassId=1) and Backlash (ClassId=0, universal) from the
        // fixture - class 64 has neither yet, so it should fall through to the universal one.
        let styles = list_styles_for_spec(&cfg, 64, "slash").unwrap();
        assert_eq!(styles.len(), 1, "{styles:?}");
        assert_eq!(str_field(&styles[0].fields, "Name"), "Backlash");
        assert!(!styles[0].is_owned);

        // Once class 64 gets its own locked copy, that must take priority over the universal one.
        run_remote_sql(
            &cfg,
            "INSERT INTO Style VALUES (NULL, 64, 'Pulverize', 'slash', 5, 300, 5, 0, 1, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);",
        )
        .unwrap();
        let styles_after = list_styles_for_spec(&cfg, 64, "slash").unwrap();
        assert_eq!(styles_after.len(), 1, "{styles_after:?}");
        assert_eq!(str_field(&styles_after[0].fields, "Name"), "Pulverize");
        assert!(styles_after[0].is_owned);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_style_owned_grants_a_mechanic_locked_specs_own_copy_on_first_touch() {
        let dir = setup_test_db("ensure-style-owned-uncloneable");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                VALUES ('Sword', 'Sword', 'Sword weapons', 100, NULL, 0);
             INSERT INTO Style VALUES (NULL, 9, 'Whirling Blade', 'Sword', 5, 300, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);",
        )
        .unwrap();
        let source_style_id = int_field(
            &run_remote_sql_json(&cfg, "SELECT StyleID FROM Style WHERE SpecKeyName='Sword';").unwrap()[0],
            "StyleID",
        );

        let owned_id = ensure_style_owned(&cfg, 64, "Sword", source_style_id).unwrap();
        assert_ne!(owned_id, source_style_id, "must be class 64's own, separate row");

        let owned_rows = run_remote_sql_json(
            &cfg,
            &format!("SELECT ClassId, Name FROM Style WHERE StyleID = {owned_id};"),
        )
        .unwrap();
        assert_eq!(int_field(&owned_rows[0], "ClassId"), 64);
        assert_eq!(str_field(&owned_rows[0], "Name"), "Whirling Blade");

        // The spec itself (Specialization/SpellLine) must never be renamed for a mechanic-locked
        // spec - still "Sword", not "sword_64" or similar.
        let specs = run_remote_sql_json(&cfg, "SELECT KeyName FROM Specialization WHERE KeyName='Sword';").unwrap();
        assert_eq!(specs.len(), 1);

        // Calling it again must be a pure no-op - same ID, no second grant.
        let second_call_id = ensure_style_owned(&cfg, 64, "Sword", owned_id).unwrap();
        assert_eq!(second_call_id, owned_id);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_style_owned_promotes_a_regular_specs_whole_spec_when_needed() {
        let dir = setup_test_db("ensure-style-owned-regular-spec");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO ClassXSpecialization VALUES (64, 'slash', 0);",
        )
        .unwrap();
        let source_style_id = int_field(
            &run_remote_sql_json(&cfg, "SELECT StyleID FROM Style WHERE Name='Pulverize';").unwrap()[0],
            "StyleID",
        );

        let owned_id = ensure_style_owned(&cfg, 64, "slash", source_style_id).unwrap();
        assert_ne!(owned_id, source_style_id);

        let assigned = run_remote_sql_json(
            &cfg,
            "SELECT SpecKeyName FROM ClassXSpecialization WHERE ClassID = 64;",
        )
        .unwrap();
        assert_eq!(str_field(&assigned[0], "SpecKeyName"), "slash_64");

        let owned_rows = run_remote_sql_json(
            &cfg,
            &format!("SELECT ClassId, SpecKeyName FROM Style WHERE StyleID = {owned_id};"),
        )
        .unwrap();
        assert_eq!(int_field(&owned_rows[0], "ClassId"), 64);
        assert_eq!(str_field(&owned_rows[0], "SpecKeyName"), "slash_64");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_style_fields_edits_an_already_owned_style_and_refuses_a_shared_one() {
        let dir = setup_test_db("update-style-fields");
        let cfg = test_cfg(&dir);
        let style_id = int_field(
            &run_remote_sql_json(&cfg, "SELECT StyleID FROM Style WHERE Name='Pulverize';").unwrap()[0],
            "StyleID",
        );

        // "Pulverize" is locked to class 1, not 64 - editing it as class 64 must be refused.
        let mut fields = std::collections::HashMap::new();
        fields.insert("EnduranceCost".to_string(), Value::from(99));
        let refused = update_style_fields(&cfg, 64, style_id, &fields);
        assert!(refused.is_err(), "a style not locked to this class must be refused");

        // Its own owner (class 1) can edit it directly.
        let result = update_style_fields(&cfg, 1, style_id, &fields).unwrap();
        assert!(result.contains("updated 1 field"), "{result}");
        let updated = run_remote_sql_json(&cfg, &format!("SELECT EnduranceCost FROM Style WHERE StyleID = {style_id};")).unwrap();
        assert_eq!(int_field(&updated[0], "EnduranceCost"), 99);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_style_level_grants_then_updates_a_mechanic_locked_specs_level() {
        let dir = setup_test_db("update-style-level-uncloneable");
        let cfg = test_cfg(&dir);
        run_remote_sql(
            &cfg,
            "INSERT INTO Specialization (KeyName, Name, Description, Icon, Implementation, ClientStyleGroup) \
                VALUES ('Sword', 'Sword', 'Sword weapons', 100, NULL, 0);
             INSERT INTO Style VALUES (NULL, 9, 'Whirling Blade', 'Sword', 5, 300, 5, 0, 0, 0, 0, 0, 1.0, 1.0, 0, 0, 0, 0, 0, 0);",
        )
        .unwrap();
        let source_style_id = int_field(
            &run_remote_sql_json(&cfg, "SELECT StyleID FROM Style WHERE SpecKeyName='Sword';").unwrap()[0],
            "StyleID",
        );

        let result = update_style_level(&cfg, 64, "Sword", source_style_id, 12).unwrap();
        assert!(result.contains("updated level to 12"), "{result}");

        let owned = run_remote_sql_json(
            &cfg,
            "SELECT SpecLevelRequirement FROM Style WHERE SpecKeyName='Sword' AND ClassId=64;",
        )
        .unwrap();
        assert_eq!(int_field(&owned[0], "SpecLevelRequirement"), 12);

        let original = run_remote_sql_json(
            &cfg,
            &format!("SELECT SpecLevelRequirement FROM Style WHERE StyleID = {source_style_id};"),
        )
        .unwrap();
        assert_eq!(
            int_field(&original[0], "SpecLevelRequirement"),
            5,
            "the original style must be untouched"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
