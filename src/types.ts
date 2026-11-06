// Mirrors the Rust structs in src-tauri/src/classdata.rs, codegen.rs, and deploy.rs.

export interface ClassInfo {
  id: number;
  name: string;
  base_class_name: string;
  realm: string;
  class_ident: string;
  parent_ident: string;
  profession: string | null;
  primary_stat: string | null;
  secondary_stat: string | null;
  tertiary_stat: string | null;
  mana_stat: string | null;
  ws_base: number | null;
  base_hp: number | null;
  specialization_multiplier: number | null;
  class_type: string | null;
  is_focus_caster: boolean | null;
  eligible_races: string[];
  effective_eligible_races: string[];
  is_custom: boolean;
  filename: string | null;
  superseded_by: string | null;
  raw_source: string;
}

export interface ClassListResult {
  classes: ClassInfo[];
  deployed_error: string | null;
}

/** Shared realm display order - Albion/Midgard/Hibernia first, anything else (shouldn't normally
 * happen) appended alphabetically after. */
export const REALM_ORDER = ["Albion", "Midgard", "Hibernia"];

/** The base archetype files (ClassFighter, ClassAcolyte, ClassViking, ...) parse into ClassInfo
 * records too (classdata.rs needs them for the EligibleRaces inheritance fallback), but they
 * aren't playable classes themselves - only concrete classes that subclass one of them are real,
 * pickable classes. "CharacterClassBase" is the literal C# parent every archetype file itself
 * extends, so anything still pointing at it directly is an archetype, not a leaf class. */
export function leafClassesOf(classes: ClassInfo[]): ClassInfo[] {
  return classes.filter((c) => c.parent_ident !== "CharacterClassBase");
}

export interface KnownOptions {
  stats: string[];
  races: string[];
  class_types: string[];
  race_realms: Record<string, string>;
}

export interface CloneRequest {
  source: ClassInfo;
  new_name: string;
  new_class_ident: string;
  new_id: number;
  profession: string | null;
  primary_stat: string | null;
  secondary_stat: string | null;
  tertiary_stat: string | null;
  mana_stat: string | null;
  ws_base: number | null;
  base_hp: number | null;
  specialization_multiplier: number | null;
  class_type: string | null;
  is_focus_caster: boolean | null;
  eligible_races: string[] | null;
}

export interface SshConfig {
  host: string;
  user: string;
  port: number;
  identity_file: string;
  remote_server_dir: string;
  start_script_name: string;
}

export const emptySshConfig: SshConfig = {
  host: "",
  user: "",
  port: 22,
  identity_file: "",
  remote_server_dir: "",
  start_script_name: "daoc-server.sh",
};

// Mirrors src-tauri/src/specs.rs.

export interface SpecializationRow {
  key_name: string;
  name: string;
  description: string;
  icon: number;
}

export interface ClassSpecRow {
  spec_key_name: string;
  spec_name: string;
  level_acquired: number;
}

export interface SpecSpellLineRow {
  key_name: string;
  name: string;
  is_base_line: boolean;
}

export interface LineSpellRow {
  spell_id: number;
  name: string;
  level: number;
  spell_group: number;
}

export interface SpellLineSelectionPayload {
  source_line_key: string;
  included: boolean;
  excluded_spell_ids: number[];
}

export interface SpecAssignmentPayload {
  source_key: string;
  new_key: string;
  new_name: string;
  level_acquired: number;
  spell_line_selection: SpellLineSelectionPayload[];
}

// Mirrors src-tauri/src/spell_ownership.rs. A Spell row's real schema has dozens of fields that
// vary between server forks (damage, range, duration, procs, message text, ...), so this is
// deliberately untyped - the editor renders whatever columns actually come back.
export type OwnedSpellRow = Record<string, string | number | boolean | null>;

// Mirrors specs.rs's OwnedSpellInLine/SpellLineGroup.
export interface OwnedSpellInLine {
  line_xspell_id: string;
  level: number;
  spell_id: number;
  is_owned: boolean;
  fields: OwnedSpellRow;
}

export interface SpellLineGroup {
  spec_key: string;
  line_key: string;
  line_name: string;
  spells: OwnedSpellInLine[];
}

// Mirrors specs.rs's ClassSpecSummary - the "Classes" tab's own two summary lists (combat specs
// vs. spells), before the user picks which one to actually expand and edit.
export interface ClassSpecSummary {
  key_name: string;
  name: string;
  level_acquired: number;
  is_owned: boolean;
  has_styles: boolean;
  has_spell_lines: boolean;
}

// Mirrors specs.rs's StyleInfo - a combat style (melee skill) row, the Style-table equivalent of
// OwnedSpellInLine/OwnedSpellRow above.
export interface StyleInfo {
  style_id: number;
  is_owned: boolean;
  fields: OwnedSpellRow;
}

// Mirrors src-tauri/src/trainer.rs.

export interface TrainerRequest {
  class_id: number;
  class_name: string;
  trainer_class_ident: string;
  guild_name: string;
  realm: string;
  archetype_id: number;
  archetype_name: string;
  min_level: number;
}

export interface TrainerNpcCandidate {
  mob_id: string;
  name: string;
  guild: string;
  class_type: string;
  x: number;
  y: number;
  z: number;
  heading: number;
  region: number;
  realm: number;
  model: number;
  level: number;
}
