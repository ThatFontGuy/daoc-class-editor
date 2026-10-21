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
  raw_source: string;
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
  base_class_name: string;
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

export interface SpecAssignmentPayload {
  source_key: string;
  mode: string;
  new_key: string | null;
  new_name: string | null;
  level_acquired: number;
}

// Mirrors src-tauri/src/deployed.rs.

export interface DeployedClass {
  filename: string;
  class_ident: string;
  name: string;
  id: number;
  base_class_name: string;
}
