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
    /// Full original source, kept so the UI can show "what you're cloning" for reference.
    pub raw_source: String,
}

fn realm_from_path(path: &Path) -> String {
    let s = path.to_string_lossy().to_lowercase();
    if s.contains("/albion/") {
        "Albion".to_string()
    } else if s.contains("/midgard/") {
        "Midgard".to_string()
    } else if s.contains("/hibernia/") {
        "Hibernia".to_string()
    } else {
        "Base".to_string()
    }
}

fn capture1(re: &Regex, text: &str) -> Option<String> {
    re.captures(text).map(|c| c[1].to_string())
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

    let class_ident_re = Regex::new(r"(?m)^\s*public\s+class\s+(\w+)\s*:\s*\w+").ok()?;
    let class_ident = capture1(&class_ident_re, &raw_source).unwrap_or_default();
    if class_ident.is_empty() {
        return None;
    }

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
        realm: realm_from_path(path),
        class_ident,
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
        eligible_races,
        raw_source,
    })
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

/// Walks the bundled `playerclasses/` resource folder and parses every class file found.
pub fn load_all_classes(resources_dir: &Path) -> Vec<ClassInfo> {
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
    out.sort_by(|a, b| a.realm.cmp(&b.realm).then(a.name.cmp(&b.name)));
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownOptions {
    pub stats: Vec<String>,
    pub races: Vec<String>,
    pub class_types: Vec<String>,
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
    // Scoped to just the `enum eClassType { ... }` block, not the whole interface file, so an
    // unrelated trailing-comma line elsewhere can't sneak in as a false match.
    let class_types = std::fs::read_to_string(resources_dir.join("core").join("ICharacterClass.cs"))
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
pub fn next_free_id(resources_dir: &Path) -> i32 {
    let enum_file = resources_dir.join("core").join("eCharacterClass.cs");
    let (_, max_id) = parse_enum_ids(&enum_file);
    max_id + 1
}

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
    fn sluaghbinder_fields_match_known_values() {
        let classes = load_all_classes(&resources());
        let s = classes
            .iter()
            .find(|c| c.name == "Sluaghbinder")
            .expect("Sluaghbinder should parse");
        assert_eq!(s.id, 63);
        assert_eq!(s.base_class_name, "Acolyte");
        assert_eq!(s.class_ident, "ClassSluaghbinder");
        assert_eq!(s.primary_stat.as_deref(), Some("STR"));
        assert_eq!(s.secondary_stat.as_deref(), Some("INT"));
        assert_eq!(s.tertiary_stat.as_deref(), Some("CON"));
        assert_eq!(s.mana_stat.as_deref(), Some("INT"));
        assert_eq!(s.ws_base, Some(380));
        assert_eq!(s.base_hp, Some(760));
        assert_eq!(s.specialization_multiplier, Some(10));
        assert_eq!(s.class_type.as_deref(), Some("Hybrid"));
        assert_eq!(s.is_focus_caster, Some(false));
        assert_eq!(s.eligible_races, vec!["Celt", "Firbolg"]);
    }

    #[test]
    fn druid_inherits_from_naturalist() {
        let classes = load_all_classes(&resources());
        let d = classes
            .iter()
            .find(|c| c.name == "Druid")
            .expect("Druid should parse");
        assert_eq!(d.base_class_name, "Naturalist");
        assert_eq!(d.class_ident, "ClassDruid");
        assert_eq!(d.realm, "Hibernia");
    }

    #[test]
    fn next_free_id_is_above_highest_known() {
        let id = next_free_id(&resources());
        assert!(id >= 64, "expected next free id >= 64 (Sluaghbinder is 63), got {id}");
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
        assert!(!opts.stats.iter().any(|s| s.starts_with('_')), "{:?}", opts.stats);

        assert!(opts.races.contains(&"Celt".to_string()));
        assert!(opts.races.contains(&"Firbolg".to_string()));
        assert_eq!(opts.races.len(), 21, "{:?}", opts.races);

        assert_eq!(
            opts.class_types,
            vec!["ListCaster", "Hybrid", "PureTank"],
            "{:?}",
            opts.class_types
        );
    }
}
