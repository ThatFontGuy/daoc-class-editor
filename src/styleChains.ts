// "Does this style have a real chain prerequisite" grouping logic for ClassSpecsSection.tsx's own
// combat-style table - color-codes a chain (e.g. a 3-part combo building up to a finisher) with a
// shared background, the same idea spellProgression.ts already applies to a spell line's own
// ranked series, just linked a different way: a style doesn't share a key with its own
// prerequisite, it explicitly points at it.
//
// Confirmed directly in the real server's own source (Style.cs's own `OpeningRequirementValue`
// doc comment, GameServer/ECS-Components/PlayerStyleComponent.cs's own usability check): a style
// only actually requires a *specific* prior style when both are true - `OpeningRequirementType` is
// Offensive or Defensive (never Positional - that one's own `OpeningRequirementValue` means a
// facing, not a style at all), and `AttackResultRequirement` is exactly the "Style" value (the
// caller resolves this from `known_options()`'s own `attack_result_requirements`, never a
// hardcoded `7`, the same "introspect the real enum, don't assume" approach this app already uses
// for the opening/attack-result dropdowns themselves). Any other style - the common case, "many
// are Anytime" - has no real prerequisite to link at all.
//
// `OpeningRequirementValue` is matched against a style's own *shared* `Style.ID` column, never its
// per-row `StyleID` - confirmed directly in the real server's own `SkillBase.GetStyleByID` (looked
// up via `m_styleIndex`, keyed `(Style.ID, ClassId)`) and live (Infiltrator's own "Eviscerate"
// requires `OpeningRequirementValue=335`, which matches "Backstab"'s own shared `ID`, not its
// much larger per-row `StyleID`) - the exact same shared-vs-per-row-id mixup this project's own
// style-proc feature already had (see specs.rs's own `style_shared_id` doc comment), just for a
// different field. The caller still groups/keys display rows by their own per-row `StyleID`
// (`list_styles_for_spec`'s own return value) - only the *matching* needs the shared `ID`.

import { assignGroupColors } from "./groupColors";

/** Groups styles into chains - a style and whatever (if anything) it explicitly requires, and
 * whatever requires *that* in turn, transitively. `getSharedStyleId` must return each style's own
 * shared `Style.ID` (not its per-row `StyleID`); `getPrerequisiteSharedStyleId` should return
 * `null` for an ordinary, non-chain ("Anytime") style - see this module's own doc comment for
 * exactly when a real prerequisite exists at all. */
export function groupStylesByChain<T>(
  items: T[],
  getSharedStyleId: (item: T) => number,
  getPrerequisiteSharedStyleId: (item: T) => number | null,
): T[][] {
  const indexBySharedStyleId = new Map<number, number>();
  items.forEach((item, i) => indexBySharedStyleId.set(getSharedStyleId(item), i));

  const parent = items.map((_, i) => i);
  function find(x: number): number {
    while (parent[x] !== x) {
      parent[x] = parent[parent[x]];
      x = parent[x];
    }
    return x;
  }
  function union(a: number, b: number) {
    const ra = find(a);
    const rb = find(b);
    if (ra !== rb) parent[ra] = rb;
  }

  items.forEach((item, i) => {
    const prereqId = getPrerequisiteSharedStyleId(item);
    if (prereqId === null) return;
    const prereqIndex = indexBySharedStyleId.get(prereqId);
    // A prerequisite outside this exact list (e.g. a different spec entirely) has nothing here to
    // link to - left ungrouped rather than guessed at.
    if (prereqIndex !== undefined) union(i, prereqIndex);
  });

  const groups = new Map<number, T[]>();
  items.forEach((item, i) => {
    const root = find(i);
    const existing = groups.get(root);
    if (existing) existing.push(item);
    else groups.set(root, [item]);
  });
  return Array.from(groups.values());
}

/** Maps each style to a background color - see assignGroupColors's own doc comment for exactly
 * how (only an actual multi-member chain gets colored; a standalone/"Anytime" style doesn't). */
export function assignChainColors<T>(
  items: T[],
  getKey: (item: T) => string,
  getSharedStyleId: (item: T) => number,
  getPrerequisiteSharedStyleId: (item: T) => number | null,
): Map<string, string> {
  return assignGroupColors(groupStylesByChain(items, getSharedStyleId, getPrerequisiteSharedStyleId), getKey);
}
