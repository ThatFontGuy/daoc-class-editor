import { createContext, Dispatch, ReactNode, SetStateAction, useContext, useState } from "react";

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

export interface SpecAssignmentUI {
  source_key: string;
  source_name: string;
  mode: "reuse" | "clone";
  new_key: string;
  new_name: string;
  level_acquired: number;
  /** Set once "Apply to server" has run for this assignment, so re-applying is a visible choice. */
  applied: boolean;
}

export interface ProjectState {
  sourceClassIdent: string;
  newName: string;
  newIdent: string;
  newId: number;
  overrides: Overrides;
  values: Record<string, string | number | boolean | string[]>;
  specAssignments: SpecAssignmentUI[];
  generatedCode: string;
  filename: string;
}

export const emptyProjectState: ProjectState = {
  sourceClassIdent: "",
  newName: "",
  newIdent: "",
  newId: 0,
  overrides: noOverrides,
  values: {},
  specAssignments: [],
  generatedCode: "",
  filename: "",
};

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
