//! Generates a new class .cs file by literally subclassing the chosen source class
//! (`public class ClassNewThing : ClassWhateverWasPicked`), so every specialization, spell
//! eligibility, and equipment rule the source class has is inherited automatically - matching
//! "use only existing class features... cloned from existing classes." Only fields the user
//! explicitly changes from the source's own values are emitted as overrides; a clone where
//! nothing was changed produces an (almost) empty constructor body.
//!
//! The "base class" label (the attribute's third argument, e.g. "Rogue") is always taken from
//! `source.base_class_name` directly - it's inherent to which concrete class was cloned, not a
//! caller-supplied value, so there's no field on `CloneRequest` for it at all: cloning a
//! Shadowblade can't produce anything other than a Rogue-base class, by construction.

use crate::classdata::ClassInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneRequest {
    pub source: ClassInfo,
    pub new_name: String,
    pub new_class_ident: String,
    pub new_id: i32,
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
    /// None = inherit the source class's EligibleRaces as-is (no override emitted).
    pub eligible_races: Option<Vec<String>>,
}

/// A conservative sanitizer for the C# type name: letters/digits only, must start with a letter,
/// so a name like "Blood Weaver" becomes "BloodWeaver" safely.
pub fn sanitize_ident(display_name: &str) -> String {
    let mut out = String::new();
    let mut capitalize_next = true;
    for ch in display_name.chars() {
        if ch.is_alphanumeric() {
            if capitalize_next {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
            capitalize_next = false;
        } else {
            capitalize_next = true;
        }
    }
    if out.is_empty() || out.chars().next().map(|c| c.is_numeric()).unwrap_or(false) {
        out = format!("Custom{out}");
    }
    out
}

pub fn generate(req: &CloneRequest) -> String {
    let mut ctor_lines: Vec<String> = Vec::new();
    if let Some(v) = &req.profession {
        if v != req.source.profession.as_deref().unwrap_or("") {
            ctor_lines.push(format!("\t\t\tm_profession = \"{v}\";"));
        }
    }
    if let Some(v) = req.specialization_multiplier {
        if Some(v) != req.source.specialization_multiplier {
            ctor_lines.push(format!("\t\t\tm_specializationMultiplier = {v};"));
        }
    }
    if let Some(v) = &req.primary_stat {
        if Some(v.as_str()) != req.source.primary_stat.as_deref() {
            ctor_lines.push(format!("\t\t\tm_primaryStat = eStat.{v};"));
        }
    }
    if let Some(v) = &req.secondary_stat {
        if Some(v.as_str()) != req.source.secondary_stat.as_deref() {
            ctor_lines.push(format!("\t\t\tm_secondaryStat = eStat.{v};"));
        }
    }
    if let Some(v) = &req.tertiary_stat {
        if Some(v.as_str()) != req.source.tertiary_stat.as_deref() {
            ctor_lines.push(format!("\t\t\tm_tertiaryStat = eStat.{v};"));
        }
    }
    if let Some(v) = &req.mana_stat {
        if Some(v.as_str()) != req.source.mana_stat.as_deref() {
            ctor_lines.push(format!("\t\t\tm_manaStat = eStat.{v};"));
        }
    }
    if let Some(v) = req.ws_base {
        if Some(v) != req.source.ws_base {
            ctor_lines.push(format!("\t\t\tm_wsbase = {v};"));
        }
    }
    if let Some(v) = req.base_hp {
        if Some(v) != req.source.base_hp {
            ctor_lines.push(format!("\t\t\tm_baseHP = {v};"));
        }
    }

    let mut overrides: Vec<String> = Vec::new();
    if let Some(v) = req.is_focus_caster {
        if Some(v) != req.source.is_focus_caster {
            overrides.push(format!(
                "\n\t\tpublic override bool IsFocusCaster => {};\n",
                if v { "true" } else { "false" }
            ));
        }
    }
    if let Some(v) = &req.class_type {
        if Some(v.as_str()) != req.source.class_type.as_deref() {
            overrides.push(format!(
                "\n\t\tpublic override eClassType ClassType => eClassType.{v};\n"
            ));
        }
    }
    if let Some(races) = &req.eligible_races {
        let same = races.len() == req.source.eligible_races.len()
            && races.iter().all(|r| req.source.eligible_races.contains(r));
        if !same {
            let race_lines = races
                .iter()
                .map(|r| format!("\t\t\tPlayerRace.{r},"))
                .collect::<Vec<_>>()
                .join("\n");
            overrides.push(format!(
                "\n\t\tpublic override List<PlayerRace> EligibleRaces => new List<PlayerRace>()\n\t\t{{\n{race_lines}\n\t\t}};\n"
            ));
        }
    }

    let ctor_lines_text = if ctor_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", ctor_lines.join("\n"))
    };

    let overrides_block = if overrides.is_empty() {
        String::new()
    } else {
        overrides.join("")
    };

    format!(
        r#"/*
 * {new_name}: a custom class cloned from {source_name} ({source_ident}).
 * Generated by DAoC Class Editor (https://github.com/ThatFontGuy/daoc-class-editor).
 * Everything not explicitly overridden below is inherited as-is from {source_ident} -
 * specializations, spell eligibility, equipment rules, and anything else the base class defines.
 *
 * This file is not part of the normal Offline DAoC installation. Deploy it to your server's
 * <server-folder>/scripts/ folder and restart - the server's own runtime script compiler
 * (GameServer.CompileScripts, enabled by default via EnableCompilation) picks it up without
 * needing a full rebuild of the server itself.
 */
using System.Collections.Generic;
using DOL.GS.Realm;

namespace DOL.GS.PlayerClass
{{
	[CharacterClass({new_id}, "{new_name}", "{base_class_name}")]
	public class {new_ident} : {source_ident}
	{{
		public {new_ident}()
			: base()
		{{
{ctor_lines_text}		}}
{overrides_block}	}}
}}
"#,
        new_name = req.new_name,
        source_name = req.source.name,
        source_ident = req.source.class_ident,
        new_id = req.new_id,
        base_class_name = req.source.base_class_name,
        new_ident = req.new_class_ident,
        ctor_lines_text = ctor_lines_text,
        overrides_block = overrides_block,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classdata::load_all_classes;
    use std::path::Path;

    fn resources() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/classes")
    }

    fn find(name: &str) -> ClassInfo {
        load_all_classes(&resources())
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} should parse"))
    }

    #[test]
    fn pure_rename_clone_has_no_overrides() {
        let druid = find("Druid");
        let req = CloneRequest {
            new_name: "Grovekeeper".to_string(),
            new_class_ident: "ClassGrovekeeper".to_string(),
            new_id: 64,
            profession: druid.profession.clone(),
            primary_stat: druid.primary_stat.clone(),
            secondary_stat: druid.secondary_stat.clone(),
            tertiary_stat: druid.tertiary_stat.clone(),
            mana_stat: druid.mana_stat.clone(),
            ws_base: druid.ws_base,
            base_hp: druid.base_hp,
            specialization_multiplier: druid.specialization_multiplier,
            class_type: druid.class_type.clone(),
            is_focus_caster: druid.is_focus_caster,
            eligible_races: None,
            source: druid,
        };
        let code = generate(&req);
        assert!(code.contains("public class ClassGrovekeeper : ClassDruid"));
        assert!(code.contains("[CharacterClass(64, \"Grovekeeper\", \"Naturalist\")]"));
        // No fields differ from the source, so the constructor body should be empty and no
        // override properties should be emitted at all - a literal "rename only" clone.
        assert!(!code.contains("m_primaryStat"));
        assert!(!code.contains("override bool IsFocusCaster"));
        assert!(!code.contains("override eClassType ClassType"));
        assert!(!code.contains("override List<PlayerRace>"));
        // Braces balance (a cheap sanity check the template substitution didn't mangle structure).
        assert_eq!(
            code.matches('{').count(),
            code.matches('}').count(),
            "unbalanced braces:\n{code}"
        );
    }

    #[test]
    fn changed_fields_emit_only_those_overrides() {
        let cleric = find("Cleric");
        let mut req = CloneRequest {
            new_name: "Lightbringer".to_string(),
            new_class_ident: "ClassLightbringer".to_string(),
            new_id: 65,
            profession: cleric.profession.clone(),
            primary_stat: cleric.primary_stat.clone(),
            secondary_stat: cleric.secondary_stat.clone(),
            tertiary_stat: cleric.tertiary_stat.clone(),
            mana_stat: cleric.mana_stat.clone(),
            ws_base: cleric.ws_base,
            base_hp: Some(cleric.base_hp.unwrap_or(700) + 40),
            specialization_multiplier: cleric.specialization_multiplier,
            class_type: cleric.class_type.clone(),
            is_focus_caster: cleric.is_focus_caster,
            eligible_races: Some(vec!["Briton".to_string(), "Highlander".to_string()]),
            source: cleric,
        };
        let code = generate(&req);
        assert!(code.contains(&format!("m_baseHP = {};", req.base_hp.unwrap())));
        assert!(code.contains("PlayerRace.Briton"));
        assert!(code.contains("PlayerRace.Highlander"));
        // Nothing else changed, so other fields must not appear as overrides.
        assert!(!code.contains("m_primaryStat"));
        assert!(!code.contains("m_specializationMultiplier"));
        assert_eq!(code.matches('{').count(), code.matches('}').count());

        // Flip base_hp back to identical: that override line should disappear entirely.
        req.base_hp = req.source.base_hp;
        let code2 = generate(&req);
        assert!(!code2.contains("m_baseHP"));
    }

    #[test]
    fn sanitize_ident_handles_spaces_and_symbols() {
        assert_eq!(sanitize_ident("Blood Weaver"), "BloodWeaver");
        assert_eq!(sanitize_ident("shadow-knight"), "ShadowKnight");
        // A leading digit isn't a valid C# identifier start; must be prefixed to stay valid.
        let result = sanitize_ident("9lives");
        assert_eq!(result, "Custom9lives");
        assert!(result.chars().next().unwrap().is_alphabetic());
    }
}

#[cfg(test)]
mod manual_preview {
    use super::*;
    use crate::classdata::load_all_classes;
    use std::path::Path;

    #[test]
    fn print_a_sample_generated_file() {
        let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/classes");
        let cleric = load_all_classes(&resources)
            .into_iter()
            .find(|c| c.name == "Cleric")
            .unwrap();
        let req = CloneRequest {
            new_name: "Lightbringer".to_string(),
            new_class_ident: "ClassLightbringer".to_string(),
            new_id: 65,
            profession: cleric.profession.clone(),
            primary_stat: cleric.primary_stat.clone(),
            secondary_stat: cleric.secondary_stat.clone(),
            tertiary_stat: cleric.tertiary_stat.clone(),
            mana_stat: cleric.mana_stat.clone(),
            ws_base: cleric.ws_base,
            base_hp: Some(cleric.base_hp.unwrap_or(700) + 40),
            specialization_multiplier: cleric.specialization_multiplier,
            class_type: cleric.class_type.clone(),
            is_focus_caster: cleric.is_focus_caster,
            eligible_races: Some(vec!["Briton".to_string(), "Highlander".to_string()]),
            source: cleric,
        };
        println!("{}", generate(&req));
    }
}
