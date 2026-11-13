// Shared "which spells are really just ranks of the same ability" grouping logic - used both by
// ClonePanel.tsx's own spell-line customizer (merging every level of one ability onto a single
// display row) and ClassSpecsSection.tsx's own spell table (color-coding a progression's shared
// background so a series is easy to spot at a glance). Kept in one place so the two views can
// never silently disagree about what counts as "the same ability".

import { assignGroupColors } from "./groupColors";

/** Groups items that represent ranks of the same real ability - two items are linked if either
 * (a) they share the exact same display Name (the common case, and what the custom Vampiir-
 * sourced spells in this project actually have, since their own SpellGroup is 0 for every one of
 * them - that wasn't part of the data sourced for them), or (b) they share the same nonzero
 * Spell.SpellGroup (the real server's own progression link, still useful for native specs whose
 * display Name changes per level, e.g. a Sorcerer's "Inflict Minor Forgetfulness" -> "Inflict
 * Forgetfulness" -> "Inflict Minor Amnesia" -> "Inflict Amnesia" -> "Inflict Oblivion" all share
 * one SpellGroup despite five different names). A union-find over both links handles either
 * relation (or both at once) correctly. Each returned group - and the groups themselves - are
 * ordered by level, matching how a planner reads top to bottom. */
export function groupSpellsByProgression<T>(
  items: T[],
  getName: (item: T) => string,
  getSpellGroup: (item: T) => number,
  getLevel: (item: T) => number,
): T[][] {
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

  const firstSeenByName = new Map<string, number>();
  const firstSeenByGroup = new Map<number, number>();
  items.forEach((item, i) => {
    const name = getName(item);
    const priorByName = firstSeenByName.get(name);
    if (priorByName !== undefined) union(i, priorByName);
    else firstSeenByName.set(name, i);

    const spellGroup = getSpellGroup(item);
    if (spellGroup !== 0) {
      const priorByGroup = firstSeenByGroup.get(spellGroup);
      if (priorByGroup !== undefined) union(i, priorByGroup);
      else firstSeenByGroup.set(spellGroup, i);
    }
  });

  const groups = new Map<number, T[]>();
  items.forEach((item, i) => {
    const root = find(i);
    const existing = groups.get(root);
    if (existing) existing.push(item);
    else groups.set(root, [item]);
  });
  return Array.from(groups.values())
    .map((g) => [...g].sort((a, b) => getLevel(a) - getLevel(b)))
    .sort((a, b) => getLevel(a[0]) - getLevel(b[0]));
}

/** Maps each item to a background color - see assignGroupColors's own doc comment for exactly how
 * (only an actual multi-member progression gets colored at all). */
export function assignProgressionColors<T>(
  items: T[],
  getKey: (item: T) => string,
  getName: (item: T) => string,
  getSpellGroup: (item: T) => number,
  getLevel: (item: T) => number,
): Map<string, string> {
  return assignGroupColors(groupSpellsByProgression(items, getName, getSpellGroup, getLevel), getKey);
}
