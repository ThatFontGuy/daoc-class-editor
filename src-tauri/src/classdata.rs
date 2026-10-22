//! Parses the vendored reference .cs files (from daoc-offline-linux-server-bots) into structured
//! `ClassInfo` records the frontend can list and clone from. This is intentionally a set of small
//! regexes over the source text, not a real C# parser - the vendored files are small and follow a
//! consistent shape (see ClassSluaghbinder.cs in the server repo), so this is simpler and more
//! maintainable than pulling in a full C# AST crate for a handful of well-known patterns.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassInfo {
    /// Numeric class ID from eCharacterClass (resolved via the enum name in the attribute).
    pub id: i32,
    /// Display name as it appears at character creation, e.g. "Druid".
    pub name: String,
    /// The base class name string from the attribute's third argument, e.g. "Acolyte".
    pub base_class_name: String,
    /// Realm folder the file lives under: Albion, Midgard, Hibernia, or Base (abstract bases).
    pub realm: String,
    /// The C# type name, e.g. "ClassDruid" - this is what a clone's `: Base` clause references.
    pub class_ident: String,
    /// The actual C# type it extends, e.g. "ClassNaturalist" for ClassDruid, or
    /// "CharacterClassBase" for a top-level base class. Used to resolve inherited fields
    /// (most importantly EligibleRaces, which most concrete classes don't override themselves).
    pub parent_ident: String,
    pub profession: Option<String>,
    pub primary_stat: Option<String>,
    pub secondary_stat: Option<String>,
    pub tertiary_stat: Option<String>,
    pub mana_stat: Option<String>,
    pub ws_base: Option<i32>,
    pub base_hp: Option<i32>,
    pub specialization_multiplier: Option<i32>,
    pub class_type: Option<String>,
    pub is_focus_caster: Option<bool>,
    pub eligible_races: Vec<String>,
    /// EligibleRaces resolved up the inheritance chain if this class doesn't override it itself
    /// (most don't) - this is what the frontend should actually use to filter race checkboxes.
    /// Equal to `eligible_races` when this class does have its own override.
    pub effective_eligible_races: Vec<String>,
    /// True for a class this tool generated and deployed to a server's `scripts/` folder (as
    /// opposed to one of the ~63 vendored reference classes the server ships with) - lets the
    /// UI badge it, and lets a "clone of a clone" pick it as a source the same as any other.
    #[serde(default)]
    pub is_custom: bool,
    /// The `.cs` filename this was deployed as, e.g. "ClassGrovekeeper.cs" - `None` for the
    /// vendored reference classes, which don't live on any particular server. Lets "Edit" on the
    /// deployed-classes list know which file to overwrite on redeploy.
    #[serde(default)]
    pub filename: Option<String>,
    /// Full original source, kept so the UI can show "what you're cloning" for reference.
    pub raw_source: String,
}

fn capture1(re: &Regex, text: &str) -> Option<String> {
    re.captures(text).map(|c| c[1].to_string())
}

/// Extracts every field after the identity (id/name/base_class_name/class_ident/parent_ident)
/// has already been determined - shared by `parse_one` (vendored reference files, whose
/// `[CharacterClass]` attribute references the `eCharacterClass` enum) and
/// `parse_deployed_source` (a tool-generated class on a live server, whose attribute carries a
/// raw integer ID instead, since it has no enum entry at all). `realm` is left blank here: it
/// depends on `effective_eligible_races`, which can only be resolved once every known class
/// (vendored and deployed) is assembled into one set - see `finalize_classes`.
fn parse_fields(
    raw_source: String,
    id: i32,
    name: String,
    base_class_name: String,
    class_ident: String,
    parent_ident: String,
    is_custom: bool,
) -> Option<ClassInfo> {
    let profession = capture1(
        &Regex::new(r#"m_profession\s*=\s*"([^"]+)""#).ok()?,
        &raw_source,
    );
    let primary_stat = capture1(
        &Regex::new(r"m_primaryStat\s*=\s*eStat\.(\w+)").ok()?,
        &raw_source,
    );
    let secondary_stat = capture1(
        &Regex::new(r"m_secondaryStat\s*=\s*eStat\.(\w+)").ok()?,
        &raw_source,
    );
    let tertiary_stat = capture1(
        &Regex::new(r"m_tertiaryStat\s*=\s*eStat\.(\w+)").ok()?,
        &raw_source,
    );
    let mana_stat = capture1(
        &Regex::new(r"m_manaStat\s*=\s*eStat\.(\w+)").ok()?,
        &raw_source,
    );
    let ws_base = capture1(&Regex::new(r"m_wsbase\s*=\s*(\d+)").ok()?, &raw_source)
        .and_then(|v| v.parse().ok());
    let base_hp = capture1(&Regex::new(r"m_baseHP\s*=\s*(\d+)").ok()?, &raw_source)
        .and_then(|v| v.parse().ok());
    let specialization_multiplier = capture1(
        &Regex::new(r"m_specializationMultiplier\s*=\s*(\d+)").ok()?,
        &raw_source,
    )
    .and_then(|v| v.parse().ok());
    let class_type = capture1(
        &Regex::new(r"ClassType\s*=>\s*eClassType\.(\w+)").ok()?,
        &raw_source,
    );
    let is_focus_caster = capture1(
        &Regex::new(r"IsFocusCaster\s*=>\s*(true|false)").ok()?,
        &raw_source,
    )
    .map(|v| v == "true");

    // EligibleRaces, when overridden, is a `new List<PlayerRace>() { PlayerRace.X, PlayerRace.Y };`
    // block - grab every PlayerRace.* after the EligibleRaces keyword appears, up to the closing `;`.
    let mut eligible_races = Vec::new();
    if let Some(idx) = raw_source.find("EligibleRaces") {
        if let Some(end) = raw_source[idx..].find("};") {
            let block = &raw_source[idx..idx + end];
            let race_re = Regex::new(r"PlayerRace\.(\w+)").ok()?;
            for cap in race_re.captures_iter(block) {
                eligible_races.push(cap[1].to_string());
            }
        }
    }

    Some(ClassInfo {
        id,
        name,
        base_class_name,
        realm: String::new(),
        class_ident,
        parent_ident,
        profession,
        primary_stat,
        secondary_stat,
        tertiary_stat,
        mana_stat,
        ws_base,
        base_hp,
        specialization_multiplier,
        class_type,
        is_focus_caster,
        effective_eligible_races: eligible_races.clone(),
        eligible_races,
        is_custom,
        filename: None,
        raw_source,
    })
}

fn parse_one(path: &Path, enum_ids: &HashMap<String, i32>) -> Option<ClassInfo> {
    let raw_source = std::fs::read_to_string(path).ok()?;

    // [CharacterClass((int)eCharacterClass.Druid, "Druid", "Naturalist")] - id via the enum name,
    // looked up in enum_ids (parsed separately from eCharacterClass.cs) rather than assuming the
    // attribute always carries a raw integer literal.
    let attr_re = Regex::new(
        r#"\[CharacterClass\(\s*\(int\)eCharacterClass\.(\w+)\s*,\s*"([^"]+)"\s*,\s*"([^"]+)""#,
    )
    .ok()?;
    let attr_caps = attr_re.captures(&raw_source)?;
    let enum_name = attr_caps[1].to_string();
    let name = attr_caps[2].to_string();
    let base_class_name = attr_caps[3].to_string();
    let id = *enum_ids.get(&enum_name)?;

    let class_decl_re = Regex::new(r"(?m)^\s*public\s+class\s+(\w+)\s*:\s*(\w+)").ok()?;
    let class_decl = class_decl_re.captures(&raw_source)?;
    let class_ident = class_decl[1].to_string();
    let parent_ident = class_decl[2].to_string();

    parse_fields(
        raw_source,
        id,
        name,
        base_class_name,
        class_ident,
        parent_ident,
        false,
    )
}

/// Parses a deployed custom class's source straight from a live server's `scripts/` folder - same
/// field shapes as a vendored file, but the `[CharacterClass(...)]` attribute carries a raw
/// integer ID (as emitted by codegen.rs) instead of an `eCharacterClass` enum reference, since a
/// tool-generated class has no enum entry to reference at all.
pub fn parse_deployed_source(content: &str) -> Option<ClassInfo> {
    let attr_re =
        Regex::new(r#"\[CharacterClass\(\s*(\d+)\s*,\s*"([^"]+)"\s*,\s*"([^"]+)""#).ok()?;
    let attr_caps = attr_re.captures(content)?;
    let id: i32 = attr_caps[1].parse().ok()?;
    let name = attr_caps[2].to_string();
    let base_class_name = attr_caps[3].to_string();

    let class_decl_re = Regex::new(r"(?m)^\s*public\s+class\s+(\w+)\s*:\s*(\w+)").ok()?;
    let class_decl = class_decl_re.captures(content)?;
    let class_ident = class_decl[1].to_string();
    let parent_ident = class_decl[2].to_string();

    parse_fields(
        content.to_string(),
        id,
        name,
        base_class_name,
        class_ident,
        parent_ident,
        true,
    )
}

/// Parses eCharacterClass.cs into a name -> numeric ID map (e.g. "Druid" -> 47), and separately
/// returns the highest ID seen so callers can suggest a free one for a brand-new class.
pub fn parse_enum_ids(enum_file: &Path) -> (HashMap<String, i32>, i32) {
    let mut map = HashMap::new();
    let mut max_id = 0;
    if let Ok(text) = std::fs::read_to_string(enum_file) {
        if let Ok(re) = Regex::new(r"(\w+)\s*=\s*(\d+)\s*,?") {
            for cap in re.captures_iter(&text) {
                if let Ok(id) = cap[2].parse::<i32>() {
                    map.insert(cap[1].to_string(), id);
                    if id > max_id {
                        max_id = id;
                    }
                }
            }
        }
    }
    (map, max_id)
}

/// Extracts the real race -> realm mapping straight from PlayerRace.cs
/// (`{ eRace.Briton, new PlayerRace(eRace.Briton, eRealm.Albion, ...) }`), so class realm
/// classification can be grounded in actual race eligibility rather than guessed from folder
/// names (which mislabels every class filed under the shared `base/` folder).
fn parse_race_realms(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(re) =
            Regex::new(r"eRace\.(\w+)\s*,\s*new\s+PlayerRace\(\s*eRace\.\w+\s*,\s*eRealm\.(\w+)")
        {
            for cap in re.captures_iter(&text) {
                map.insert(cap[1].to_string(), cap[2].to_string());
            }
        }
    }
    map
}

/// Walks the bundled `playerclasses/` resource folder and parses every class file found, without
/// resolving realm/effective-races yet (see `finalize_classes`) - split out so deployed custom
/// classes can be folded into the same set before that resolution pass runs once, together.
fn load_vendored_unfinalized(resources_dir: &Path) -> Vec<ClassInfo> {
    let enum_file = resources_dir.join("core").join("eCharacterClass.cs");
    let (enum_ids, _) = parse_enum_ids(&enum_file);

    let playerclasses_dir = resources_dir.join("playerclasses");
    let mut out = Vec::new();
    for entry in WalkDir::new(&playerclasses_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("cs") {
            if let Some(info) = parse_one(path, &enum_ids) {
                out.push(info);
            }
        }
    }
    out
}

/// Resolves every scalar field a class might not set in its own file (profession, stats,
/// ws_base, base_hp, specialization_multiplier, class_type, is_focus_caster) by walking
/// `parent_ident` upward until an ancestor that *does* set it is found - the same idea as
/// `resolve_effective_eligible_races`, generalized to every other overridable field at once.
/// Without this, a field a class's immediate parent *also* doesn't set directly (inheriting it
/// from one hop further up - e.g. weapon skill base almost always comes from the archetype two
/// hops up from a promoted class) looked "lost" to anything reading that field directly, instead
/// of correctly showing the real inherited value. This bit editing Nosferatu (cloned from
/// Shadowblade): base HP/specialization multiplier are set directly on Shadowblade so a
/// single-hop lookup happened to work, but weapon skill base is set two hops up on the Midgard
/// Rogue archetype, so it still showed as missing even after that was fixed.
fn resolve_effective_scalars(classes: &mut [ClassInfo]) {
    #[derive(Clone)]
    struct Snapshot {
        parent_ident: String,
        profession: Option<String>,
        primary_stat: Option<String>,
        secondary_stat: Option<String>,
        tertiary_stat: Option<String>,
        mana_stat: Option<String>,
        ws_base: Option<i32>,
        base_hp: Option<i32>,
        specialization_multiplier: Option<i32>,
        class_type: Option<String>,
        is_focus_caster: Option<bool>,
    }

    // Snapshot every class's own (pre-resolution) values before mutating anything - a lookup for
    // one class must never see another class's already-resolved effective value instead of its
    // real own one, which the chain walk below depends on.
    let snapshots: HashMap<String, Snapshot> = classes
        .iter()
        .map(|c| {
            (
                c.class_ident.clone(),
                Snapshot {
                    parent_ident: c.parent_ident.clone(),
                    profession: c.profession.clone(),
                    primary_stat: c.primary_stat.clone(),
                    secondary_stat: c.secondary_stat.clone(),
                    tertiary_stat: c.tertiary_stat.clone(),
                    mana_stat: c.mana_stat.clone(),
                    ws_base: c.ws_base,
                    base_hp: c.base_hp,
                    specialization_multiplier: c.specialization_multiplier,
                    class_type: c.class_type.clone(),
                    is_focus_caster: c.is_focus_caster,
                },
            )
        })
        .collect();

    for class in classes.iter_mut() {
        let mut current = class.parent_ident.clone();
        // Capped: the real inheritance chains here are at most 2-3 levels deep; this just
        // guards against an unexpected cycle in malformed vendored data.
        for _ in 0..10 {
            let all_resolved = class.profession.is_some()
                && class.primary_stat.is_some()
                && class.secondary_stat.is_some()
                && class.tertiary_stat.is_some()
                && class.mana_stat.is_some()
                && class.ws_base.is_some()
                && class.base_hp.is_some()
                && class.specialization_multiplier.is_some()
                && class.class_type.is_some()
                && class.is_focus_caster.is_some();
            if all_resolved {
                break;
            }
            let Some(snap) = snapshots.get(&current) else {
                break;
            };
            class.profession = class.profession.clone().or_else(|| snap.profession.clone());
            class.primary_stat = class
                .primary_stat
                .clone()
                .or_else(|| snap.primary_stat.clone());
            class.secondary_stat = class
                .secondary_stat
                .clone()
                .or_else(|| snap.secondary_stat.clone());
            class.tertiary_stat = class
                .tertiary_stat
                .clone()
                .or_else(|| snap.tertiary_stat.clone());
            class.mana_stat = class.mana_stat.clone().or_else(|| snap.mana_stat.clone());
            class.ws_base = class.ws_base.or(snap.ws_base);
            class.base_hp = class.base_hp.or(snap.base_hp);
            class.specialization_multiplier = class
                .specialization_multiplier
                .or(snap.specialization_multiplier);
            class.class_type = class.class_type.clone().or_else(|| snap.class_type.clone());
            class.is_focus_caster = class.is_focus_caster.or(snap.is_focus_caster);

            current = snap.parent_ident.clone();
        }
    }
}

/// Resolves `effective_eligible_races` (walking the inheritance chain) and `realm` (from the
/// first resolved race's real realm, via `PlayerRace.cs`) across a whole set of classes at once,
/// then sorts them - shared by the vendored-only list and the vendored+deployed combined list, so
/// a deployed class whose own clone didn't touch races still inherits correctly from whichever
/// vendored (or other deployed) class it was cloned from.
fn finalize_classes(classes: &mut Vec<ClassInfo>, race_realms: &HashMap<String, String>) {
    resolve_effective_eligible_races(classes);
    resolve_effective_scalars(classes);
    for class in classes.iter_mut() {
        if let Some(race) = class.effective_eligible_races.first() {
            if let Some(realm) = race_realms.get(race) {
                class.realm = realm.clone();
            }
        }
    }
    classes.sort_by(|a, b| a.realm.cmp(&b.realm).then(a.name.cmp(&b.name)));
}

/// Walks the bundled `playerclasses/` resource folder and parses every class file found.
pub fn load_all_classes(resources_dir: &Path) -> Vec<ClassInfo> {
    let race_realms = parse_race_realms(&resources_dir.join("core").join("PlayerRace.cs"));
    let mut out = load_vendored_unfinalized(resources_dir);
    finalize_classes(&mut out, &race_realms);
    out
}

/// Same as `load_all_classes`, but also folds in classes that didn't come from the bundled
/// reference folder at all - namely, custom classes already deployed to a live server (so they
/// can be picked as a clone source too, i.e. "clone of a clone").
pub fn load_all_classes_with_extra(resources_dir: &Path, extra: Vec<ClassInfo>) -> Vec<ClassInfo> {
    let race_realms = parse_race_realms(&resources_dir.join("core").join("PlayerRace.cs"));
    let mut out = load_vendored_unfinalized(resources_dir);
    out.extend(extra);
    finalize_classes(&mut out, &race_realms);
    out
}

/// The next ID at least one above the highest already in use, across any mix of vendored and
/// deployed classes passed in - so a brand-new custom class (which may itself later be cloned
/// again) never collides with one already deployed to the server.
pub fn next_free_id_from(classes: &[ClassInfo]) -> i32 {
    classes.iter().map(|c| c.id).max().unwrap_or(0) + 1
}

/// Walks `parent_ident` up from `class_ident` until it reaches a true archetype (one whose own
/// parent is `CharacterClassBase`) - e.g. Shadowblade *and* a Nosferatu cloned from Shadowblade
/// both resolve to Midgard Rogue. A trainer's promotion gating needs this: in classic DAoC,
/// nothing beyond the client's built-in class list is directly selectable at creation, so a
/// player creates a character as this archetype and a trainer promotes them into the specific
/// (possibly custom) class at some level - exactly how Sluaghbinder itself works from Acolyte.
/// Bounded to 10 hops so a malformed/cyclical parent chain can't loop forever.
pub fn root_archetype<'a>(classes: &'a [ClassInfo], class_ident: &str) -> Option<&'a ClassInfo> {
    let mut current = classes.iter().find(|c| c.class_ident == class_ident)?;
    for _ in 0..10 {
        if current.parent_ident == "CharacterClassBase" {
            return Some(current);
        }
        current = classes
            .iter()
            .find(|c| c.class_ident == current.parent_ident)?;
    }
    None
}

/// `EligibleRaces` is abstract in CharacterClassBase, so every concrete class gets it from
/// somewhere in its inheritance chain - but most classes (Cleric, Druid, ...) don't override it
/// themselves, inheriting it from a realm-specific base class (ClassAcolyte, ClassViking,
/// ClassNaturalist, ...) instead. This walks up `parent_ident` for any class with no override of
/// its own, until it finds an ancestor that does have one (or runs out of known ancestors).
fn resolve_effective_eligible_races(classes: &mut [ClassInfo]) {
    let own_races: HashMap<String, Vec<String>> = classes
        .iter()
        .filter(|c| !c.eligible_races.is_empty())
        .map(|c| (c.class_ident.clone(), c.eligible_races.clone()))
        .collect();
    let parents: HashMap<String, String> = classes
        .iter()
        .map(|c| (c.class_ident.clone(), c.parent_ident.clone()))
        .collect();

    for class in classes.iter_mut() {
        if !class.eligible_races.is_empty() {
            continue;
        }
        let mut current = class.parent_ident.clone();
        // Capped: the real inheritance chains here are at most 2-3 levels deep; this just
        // guards against an unexpected cycle in malformed vendored data.
        for _ in 0..10 {
            if let Some(races) = own_races.get(&current) {
                class.effective_eligible_races = races.clone();
                break;
            }
            match parents.get(&current) {
                Some(next) => current = next.clone(),
                None => break,
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownOptions {
    pub stats: Vec<String>,
    pub races: Vec<String>,
    pub class_types: Vec<String>,
    /// Race name -> realm ("Albion"/"Midgard"/"Hibernia"), so the frontend can filter the full
    /// race list down to just the realm of whatever class is being cloned.
    pub race_realms: HashMap<String, String>,
}

/// Extracts the canonical stat/race/class-type names straight from the vendored source files
/// (eStat.cs, PlayerRace.cs, ICharacterClass.cs) rather than hardcoding lists in the frontend,
/// so this stays correct if the server's own enums ever change.
pub fn load_known_options(resources_dir: &Path) -> KnownOptions {
    // `_First`/`_Last` are sentinel bounds, not real trainable stats - skip anything starting `_`.
    let stats = extract_idents(
        resources_dir.join("core").join("eStat.cs"),
        r"(?m)^\s*(\w+)\s*=\s*eProperty\.",
    )
    .into_iter()
    .filter(|s| !s.starts_with('_'))
    .collect();
    let races = extract_idents(
        resources_dir.join("core").join("PlayerRace.cs"),
        r"public static PlayerRace (\w+) =>",
    );
    let race_realms = parse_race_realms(&resources_dir.join("core").join("PlayerRace.cs"));
    // Scoped to just the `enum eClassType { ... }` block, not the whole interface file, so an
    // unrelated trailing-comma line elsewhere can't sneak in as a false match.
    let class_types =
        std::fs::read_to_string(resources_dir.join("core").join("ICharacterClass.cs"))
            .ok()
            .and_then(|text| {
                let start = text.find("enum eClassType")?;
                let open = text[start..].find('{')? + start;
                let close = text[open..].find('}')? + open;
                let block = &text[open..close];
                let re = Regex::new(r"(?m)^\s*(\w+)\s*,?\s*$").ok()?;
                Some(
                    re.captures_iter(block)
                        .map(|c| c[1].to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();
    KnownOptions {
        stats,
        races,
        class_types,
        race_realms,
    }
}

fn extract_idents(path: std::path::PathBuf, pattern: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(re) = Regex::new(pattern) {
            for cap in re.captures_iter(&text) {
                out.push(cap[1].to_string());
            }
        }
    }
    out
}

/// The next unused numeric class ID, for suggesting a default when creating a new class.
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn resources() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/classes")
    }

    #[test]
    fn parses_all_vendored_classes() {
        let classes = load_all_classes(&resources());
        assert!(
            classes.len() > 50,
            "expected 50+ parsed classes, got {}",
            classes.len()
        );
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Druid"), "Druid missing: {names:?}");
        assert!(names.contains(&"Sluaghbinder"), "Sluaghbinder missing");
        assert!(names.contains(&"Cleric"), "Cleric missing");
        assert!(names.contains(&"Warrior"), "Warrior missing");
    }

    #[test]
    fn weapon_skill_base_resolves_through_a_two_hop_inheritance_chain() {
        // ClassShadowblade sets base HP/specialization multiplier directly, but not weapon skill
        // base - that's only set on ClassMidgardRogue, two hops up. A class cloned from
        // Shadowblade (like a real custom Nosferatu) must still resolve it correctly, not just
        // the fields its immediate parent happens to set itself.
        let vendored = load_all_classes(&resources());
        let shadowblade = vendored
            .iter()
            .find(|c| c.class_ident == "ClassShadowblade")
            .expect("ClassShadowblade should parse");
        assert_eq!(shadowblade.ws_base, Some(360));
        assert_eq!(shadowblade.base_hp, Some(760));
        assert_eq!(shadowblade.specialization_multiplier, Some(22));

        let nosferatu_src = r#"
namespace DOL.GS.PlayerClass
{
	[CharacterClass(64, "Nosferatu", "Rogue")]
	public class ClassNosferatu : ClassShadowblade
	{
		public ClassNosferatu()
			: base()
		{
			m_primaryStat = eStat.QUI;
		}
	}
}
"#;
        let nosferatu = parse_deployed_source(nosferatu_src).expect("should parse");
        let combined = load_all_classes_with_extra(&resources(), vec![nosferatu]);
        let nosferatu = combined
            .iter()
            .find(|c| c.class_ident == "ClassNosferatu")
            .expect("Nosferatu should be present");
        assert_eq!(nosferatu.ws_base, Some(360));
        assert_eq!(nosferatu.base_hp, Some(760));
        assert_eq!(nosferatu.specialization_multiplier, Some(22));
    }

    #[test]
    fn sluaghbinder_fields_match_known_values() {
        let classes = load_all_classes(&resources());
        let s = classes
            .iter()
            .find(|c| c.name == "Sluaghbinder")
            .expect("Sluaghbinder should parse");
        assert_eq!(s.id, 63);
        assert_eq!(s.base_class_name, "Acolyte");
        assert_eq!(s.class_ident, "ClassSluaghbinder");
        assert_eq!(s.parent_ident, "ClassAcolyte");
        assert_eq!(s.primary_stat.as_deref(), Some("STR"));
        assert_eq!(s.secondary_stat.as_deref(), Some("INT"));
        assert_eq!(s.tertiary_stat.as_deref(), Some("CON"));
        assert_eq!(s.mana_stat.as_deref(), Some("INT"));
        assert_eq!(s.ws_base, Some(380));
        assert_eq!(s.base_hp, Some(760));
        assert_eq!(s.specialization_multiplier, Some(10));
        assert_eq!(s.class_type.as_deref(), Some("Hybrid"));
        assert_eq!(s.is_focus_caster, Some(false));
        // Sluaghbinder overrides EligibleRaces itself (to Hibernia races, despite subclassing the
        // Albion-base ClassAcolyte purely for stats/mechanics) - own and effective must match.
        assert_eq!(s.eligible_races, vec!["Celt", "Firbolg"]);
        assert_eq!(s.effective_eligible_races, vec!["Celt", "Firbolg"]);
        // Its realm classification should follow its actual races, not its parent's folder.
        assert_eq!(s.realm, "Hibernia");
    }

    #[test]
    fn parse_deployed_source_reads_a_tool_generated_class() {
        // Shaped exactly like codegen.rs's real template: raw integer ID (no enum entry exists
        // for a brand-new class), a few overridden fields, no EligibleRaces override at all
        // (meaning it should inherit through the chain once merged with its parent).
        let content = r#"
using DOL.GS;

namespace DOL.GS
{
    [CharacterClass(64, "Grovekeeper", "Naturalist")]
    public class ClassGrovekeeper : ClassDruid
    {
        public ClassGrovekeeper() : base()
        {
            m_profession = "Keepers of the Grove";
            m_baseHP = 800;
        }
    }
}
"#;
        let info = parse_deployed_source(content).expect("should parse");
        assert_eq!(info.id, 64);
        assert_eq!(info.name, "Grovekeeper");
        assert_eq!(info.base_class_name, "Naturalist");
        assert_eq!(info.class_ident, "ClassGrovekeeper");
        assert_eq!(info.parent_ident, "ClassDruid");
        assert_eq!(info.profession.as_deref(), Some("Keepers of the Grove"));
        assert_eq!(info.base_hp, Some(800));
        assert!(info.is_custom);
        assert!(info.eligible_races.is_empty());
    }

    #[test]
    fn deployed_class_with_no_race_override_inherits_through_its_custom_parent() {
        let vendored = load_vendored_unfinalized(&resources());
        let deployed = parse_deployed_source(
            r#"
            [CharacterClass(64, "Grovekeeper", "Naturalist")]
            public class ClassGrovekeeper : ClassDruid
            {
            }
            "#,
        )
        .unwrap();
        let race_realms = parse_race_realms(&resources().join("core").join("PlayerRace.cs"));
        let mut combined = vendored;
        combined.push(deployed);
        finalize_classes(&mut combined, &race_realms);

        let grovekeeper = combined
            .iter()
            .find(|c| c.class_ident == "ClassGrovekeeper")
            .unwrap();
        // Druid's own races, inherited transitively (Grovekeeper -> Druid -> Naturalist).
        assert_eq!(grovekeeper.realm, "Hibernia");
        assert!(!grovekeeper.effective_eligible_races.is_empty());
        let druid = combined.iter().find(|c| c.name == "Druid").unwrap();
        assert_eq!(
            grovekeeper.effective_eligible_races,
            druid.effective_eligible_races
        );
    }

    #[test]
    fn next_free_id_from_accounts_for_already_deployed_classes() {
        let vendored = load_all_classes(&resources());
        let mut combined = vendored.clone();
        combined.push(
            parse_deployed_source(
                r#"[CharacterClass(200, "Something", "Base")]
                public class ClassSomething : ClassDruid { }"#,
            )
            .unwrap(),
        );
        assert_eq!(next_free_id_from(&combined), 201);
        // Without the deployed class folded in, it should just be one above the vendored max.
        assert!(next_free_id_from(&vendored) < 201);
    }

    #[test]
    fn root_archetype_resolves_a_single_level_class() {
        let classes = load_all_classes(&resources());
        let archetype = root_archetype(&classes, "ClassShadowblade").expect("should resolve");
        assert_eq!(archetype.class_ident, "ClassMidgardRogue");
        assert_eq!(archetype.name, "Rogue");
    }

    #[test]
    fn root_archetype_resolves_through_a_custom_class_two_levels_deep() {
        let vendored = load_vendored_unfinalized(&resources());
        let deployed = parse_deployed_source(
            r#"[CharacterClass(64, "Nosferatu", "Rogue")]
            public class ClassNosferatu : ClassShadowblade { }"#,
        )
        .unwrap();
        let mut combined = vendored;
        combined.push(deployed);

        let archetype = root_archetype(&combined, "ClassNosferatu").expect("should resolve");
        assert_eq!(archetype.class_ident, "ClassMidgardRogue");
    }

    #[test]
    fn druid_narrows_races_relative_to_naturalist() {
        // Every concrete class actually has its own EligibleRaces override in practice (verified
        // across all 63 vendored files) - Druid specifically excludes Graoch, which its base
        // class ClassNaturalist otherwise allows, a real per-class narrowing, not an omission.
        let classes = load_all_classes(&resources());
        let d = classes
            .iter()
            .find(|c| c.name == "Druid")
            .expect("Druid should parse");
        assert_eq!(d.base_class_name, "Naturalist");
        assert_eq!(d.class_ident, "ClassDruid");
        assert_eq!(d.parent_ident, "ClassNaturalist");
        assert_eq!(d.eligible_races, vec!["Celt", "Firbolg", "Sylvan"]);
        assert_eq!(d.effective_eligible_races, d.eligible_races);
        assert_eq!(d.realm, "Hibernia");

        let naturalist = classes
            .iter()
            .find(|c| c.class_ident == "ClassNaturalist")
            .expect("ClassNaturalist should parse");
        assert_eq!(
            naturalist.eligible_races,
            vec!["Celt", "Firbolg", "Sylvan", "Graoch"]
        );
        assert!(
            !d.eligible_races.contains(&"Graoch".to_string()),
            "Druid should not include Graoch even though its base class allows it"
        );
    }

    #[test]
    fn midgard_base_folder_class_gets_correct_realm() {
        // ClassViking (Midgard's warrior base) lives under playerclasses/base/, not
        // playerclasses/midgard/ - folder-based classification would wrongly call it "Base".
        let classes = load_all_classes(&resources());
        let viking = classes
            .iter()
            .find(|c| c.class_ident == "ClassViking")
            .expect("ClassViking should parse");
        assert_eq!(viking.realm, "Midgard", "folder-based realm would say Base");
        assert_eq!(
            viking.eligible_races,
            vec![
                "Dwarf", "Frostalf", "Kobold", "Deifrang", "Norseman", "Troll", "Valkyn"
            ]
        );

        let warrior = classes
            .iter()
            .find(|c| c.name == "Warrior")
            .expect("Warrior should parse");
        assert_eq!(warrior.parent_ident, "ClassViking");
        // Warrior has its own, narrower override too (excludes Frostalf/Deifrang) - also real
        // per-class narrowing, not inheritance.
        assert_eq!(
            warrior.eligible_races,
            vec!["Dwarf", "Kobold", "Norseman", "Troll", "Valkyn"]
        );
        assert_eq!(warrior.realm, "Midgard");
    }

    #[test]
    fn no_class_is_left_unclassified() {
        // Every one of the 63 vendored classes has its own, real EligibleRaces override, so none
        // should fall back to the generic "Base" folder label after realm reclassification.
        let classes = load_all_classes(&resources());
        let unclassified: Vec<&str> = classes
            .iter()
            .filter(|c| c.realm == "Base")
            .map(|c| c.class_ident.as_str())
            .collect();
        assert!(
            unclassified.is_empty(),
            "classes still labeled Base: {unclassified:?}"
        );
        for realm in ["Albion", "Midgard", "Hibernia"] {
            assert!(
                classes.iter().any(|c| c.realm == realm),
                "expected at least one class in {realm}"
            );
        }
    }

    #[test]
    fn next_free_id_is_above_highest_known() {
        let id = next_free_id_from(&load_all_classes(&resources()));
        assert!(
            id >= 64,
            "expected next free id >= 64 (Sluaghbinder is 63), got {id}"
        );
    }
}

#[cfg(test)]
mod known_options_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn extracts_stats_races_and_class_types() {
        let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/classes");
        let opts = load_known_options(&resources);

        assert!(opts.stats.contains(&"STR".to_string()));
        assert!(opts.stats.contains(&"INT".to_string()));
        assert!(opts.stats.contains(&"ACU".to_string()));
        assert!(
            !opts.stats.iter().any(|s| s.starts_with('_')),
            "{:?}",
            opts.stats
        );

        assert!(opts.races.contains(&"Celt".to_string()));
        assert!(opts.races.contains(&"Firbolg".to_string()));
        assert_eq!(opts.races.len(), 21, "{:?}", opts.races);

        assert_eq!(
            opts.class_types,
            vec!["ListCaster", "Hybrid", "PureTank"],
            "{:?}",
            opts.class_types
        );

        assert_eq!(
            opts.race_realms.get("Celt").map(String::as_str),
            Some("Hibernia")
        );
        assert_eq!(
            opts.race_realms.get("Troll").map(String::as_str),
            Some("Midgard")
        );
        assert_eq!(
            opts.race_realms.get("Briton").map(String::as_str),
            Some("Albion")
        );
        assert_eq!(opts.race_realms.len(), 21, "{:?}", opts.race_realms);
    }
}
