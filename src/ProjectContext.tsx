import { createContext, Dispatch, ReactNode, SetStateAction, useContext, useState } from "react";
import { ClassInfo } from "./types";

// Everything here is what gets saved/loaded as a ".daocclass.json" project file - the full state
// of an in-progress class design, since defining one is something you iterate into existence
// across many sessions, not finish in one sitting.

export type Overrides = {
  profession: boolean;
  primary_stat: boolean;
  secondary_stat: boolean;
  tertiary_stat: boolean;
  mana_stat: boolean;
  ws_base: boolean;
  base_hp: boolean;
  specialization_multiplier: boolean;
  class_type: boolean;
  is_focus_caster: boolean;
  eligible_races: boolean;
};

export const noOverrides: Overrides = {
  profession: false,
  primary_stat: false,
  secondary_stat: false,
  tertiary_stat: false,
  mana_stat: false,
  ws_base: false,
  base_hp: false,
  specialization_multiplier: false,
  class_type: false,
  is_focus_caster: false,
  eligible_races: false,
};

/** One spell within a SpellLine shown by the Edit flow's "customize this spec's lines" picker -
 * unchecking one excludes just that spell, independent of whether whole other lines are excluded. */
export interface SpellSelectionUI {
  spell_id: number;
  name: string;
  level: number;
  included: boolean;
}

/** One SpellLine the user can include/exclude when cloning a spec, plus its own spells to
 * individually include/exclude - see specs.rs's SpellLineSelection doc comment for why this
 * exists (several real specs, e.g. "Mind Magic"/"Enhancement", span multiple SpellLines shared
 * across different classes; cloning the whole spec previously pulled in every single one). */
export interface SpellLineSelectionUI {
  source_line_key: string;
  line_name: string;
  included: boolean;
  spells: SpellSelectionUI[];
}

export interface SpecAssignmentUI {
  source_key: string;
  source_name: string;
  new_name: string;
  level_acquired: number;
  /** Set once "Apply to server" has run for this assignment, so re-applying is a visible choice. */
  applied: boolean;
  /** Undefined/empty = not customized, clone every SpellLine and every spell in full (the
   * original, still-default behavior). Populated once the user opens this assignment's "customize
   * spell lines" picker. */
  spellLineSelection?: SpellLineSelectionUI[];
}

export interface TrainerState {
  enabled: boolean;
  guildName: string;
  trainerClassIdent: string;
  minLevel: number;
  generatedCode: string;
  filename: string;
  referenceSearch: string;
  selectedReferenceMobId: string | null;
  npcName: string;
  offsetX: number;
  offsetY: number;
}

export const emptyTrainerState: TrainerState = {
  enabled: false,
  guildName: "",
  trainerClassIdent: "",
  minLevel: 5,
  generatedCode: "",
  filename: "",
  referenceSearch: "",
  selectedReferenceMobId: null,
  npcName: "",
  offsetX: 200,
  offsetY: 0,
};

export interface ProjectState {
  sourceClassIdent: string;
  newName: string;
  newIdent: string;
  newId: number;
  overrides: Overrides;
  values: Record<string, string | number | boolean | string[]>;
  specAssignments: SpecAssignmentUI[];
  /** The exact SpecKeyName values this class had assigned the last time its own specs were read
   * from the server (only meaningful while editing an already-deployed class in place - empty
   * for a brand-new clone). Lets `apply_class_specs` tell "removed from the list on purpose" from
   * "never submitted in the first place" apart, so a spec deleted from the list here and
   * redeployed is actually removed from the server too, not left behind. */
  originalAssignedSpecKeys: string[];
  generatedCode: string;
  filename: string;
  /** Set while editing an already-deployed custom class in place (its own numeric ID) - `null`
   * for a normal fresh clone. Locks identity fields (name/ident/ID/filename) in the UI, and
   * redirects the spec-lines step to read/write this class's own specs instead of a parent's. */
  editingDeployedClassId: number | null;
  trainer: TrainerState;
}

export const emptyProjectState: ProjectState = {
  sourceClassIdent: "",
  newName: "",
  newIdent: "",
  newId: 0,
  overrides: noOverrides,
  values: {},
  specAssignments: [],
  originalAssignedSpecKeys: [],
  generatedCode: "",
  filename: "",
  editingDeployedClassId: null,
  trainer: emptyTrainerState,
};

/** True if a child's own declared value differs from its parent's - `null` on the child's side
 * means "no override of its own, inherits fully," which isn't a real difference to show as one. */
function differs<T>(childVal: T | null, parentVal: T | null): boolean {
  return childVal !== null && childVal !== parentVal;
}

function sameRaces(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  const sortedB = [...b].sort();
  return [...a].sort().every((v, i) => v === sortedB[i]);
}

/** Builds a `ProjectState` for editing an already-deployed custom class in place: `cls` is the
 * class being edited, `parent` is whatever it literally subclasses (used the same way a normal
 * clone's source is - as the diff baseline for overrides, and as codegen's `: ParentType`
 * clause) - so regenerating without touching anything reproduces the same file, and only fields
 * the user actually changes get re-emitted as new overrides. */
export function buildEditingProjectState(cls: ClassInfo, parent: ClassInfo): ProjectState {
  const overrides: Overrides = {
    profession: differs(cls.profession, parent.profession),
    primary_stat: differs(cls.primary_stat, parent.primary_stat),
    secondary_stat: differs(cls.secondary_stat, parent.secondary_stat),
    tertiary_stat: differs(cls.tertiary_stat, parent.tertiary_stat),
    mana_stat: differs(cls.mana_stat, parent.mana_stat),
    ws_base: differs(cls.ws_base, parent.ws_base),
    base_hp: differs(cls.base_hp, parent.base_hp),
    specialization_multiplier: differs(
      cls.specialization_multiplier,
      parent.specialization_multiplier,
    ),
    class_type: differs(cls.class_type, parent.class_type),
    is_focus_caster: differs(cls.is_focus_caster, parent.is_focus_caster),
    eligible_races:
      cls.eligible_races.length > 0 && !sameRaces(cls.eligible_races, parent.eligible_races),
  };

  // Only seed `values` for fields that actually have their own override - otherwise the step 4
  // form's "prefer the project's own value, falling back to the source class's" display would
  // wrongly show this seeded value (e.g. 0, "") instead of correctly falling through to the
  // parent's real one, for every field this class simply inherits untouched. This is exactly the
  // bug that made base HP/weapon skill base/specialization multiplier look un-inherited when
  // editing a class that never overrode them in the first place.
  const values: ProjectState["values"] = {};
  if (overrides.profession) values.profession = cls.profession ?? "";
  if (overrides.primary_stat) values.primary_stat = cls.primary_stat ?? "";
  if (overrides.secondary_stat) values.secondary_stat = cls.secondary_stat ?? "";
  if (overrides.tertiary_stat) values.tertiary_stat = cls.tertiary_stat ?? "";
  if (overrides.mana_stat) values.mana_stat = cls.mana_stat ?? "";
  if (overrides.ws_base) values.ws_base = cls.ws_base ?? 0;
  if (overrides.base_hp) values.base_hp = cls.base_hp ?? 0;
  if (overrides.specialization_multiplier)
    values.specialization_multiplier = cls.specialization_multiplier ?? 10;
  if (overrides.class_type) values.class_type = cls.class_type ?? "";
  if (overrides.is_focus_caster) values.is_focus_caster = cls.is_focus_caster ?? false;
  if (overrides.eligible_races) values.eligible_races = cls.effective_eligible_races;

  return {
    ...emptyProjectState,
    sourceClassIdent: parent.class_ident,
    newName: cls.name,
    newIdent: cls.class_ident,
    newId: cls.id,
    overrides,
    values,
    filename: cls.filename ?? `${cls.class_ident}.cs`,
    editingDeployedClassId: cls.id,
  };
}

const ProjectContext = createContext<
  [ProjectState, Dispatch<SetStateAction<ProjectState>>] | null
>(null);

export function ProjectProvider({ children }: { children: ReactNode }) {
  const state = useState<ProjectState>(emptyProjectState);
  return <ProjectContext.Provider value={state}>{children}</ProjectContext.Provider>;
}

export function useProject(): [ProjectState, Dispatch<SetStateAction<ProjectState>>] {
  const ctx = useContext(ProjectContext);
  if (!ctx) throw new Error("useProject must be used inside a ProjectProvider");
  return ctx;
}
