// Shared "give each multi-member group of related rows its own background color" logic - used by
// both spellProgression.ts (a spell line's own series of ranks) and styleChains.ts (a combat
// spec's own chain-prerequisite styles), so the actual grouping rule each one computes stays
// decoupled from how groups get turned into colors on screen.

// A small, dark-theme-friendly palette (muted/desaturated hues, not literal bright pastels - this
// app's own color-scheme is dark throughout, see App.css's own `:root`, so a true pastel washes
// out the light `--text` color on top of it) cycling through enough distinct hues that two
// adjacent groups in a long list are never confusable. Deliberately distinct from the existing
// --albion/--midgard/--hibernia realm colors so this never looks like a realm indicator.
const GROUP_COLORS = [
  "#2a2216", // amber
  "#16221f", // teal
  "#1c1826", // violet
  "#241a1a", // rose
  "#1a2416", // olive
  "#161e2a", // steel blue
  "#261a24", // plum
  "#221c14", // sand
];

/** Maps each item in `groups` to a background color - every item in the same (more-than-one-
 * member) group shares one color, cycling through GROUP_COLORS by group order; a singleton group
 * (nothing else linked to it at all) gets no color at all (not present in the returned map), left
 * at the table's own default background - tinting every unrelated row a different color would be
 * more visual noise than help, the opposite of the point. */
export function assignGroupColors<T>(groups: T[][], getKey: (item: T) => string): Map<string, string> {
  const colors = new Map<string, string>();
  let colorIndex = 0;
  for (const group of groups) {
    if (group.length < 2) continue;
    const color = GROUP_COLORS[colorIndex % GROUP_COLORS.length];
    colorIndex++;
    for (const item of group) {
      colors.set(getKey(item), color);
    }
  }
  return colors;
}
