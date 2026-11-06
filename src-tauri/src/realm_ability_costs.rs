//! Resolves a Realm Ability's real per-level training cost (and, where resolvable, the per-level
//! numeric amount it actually grants) for the Character Planner export - see
//! `export_charplanner.rs`'s own module doc comment for why this exists at all.
//!
//! Neither of these is ever stored in the database: `Ability`/`ClassXRealmAbility_Atlas` only say
//! *which* abilities a class can train and give each one's name/description - the actual
//! `MaxLevel`/`CostForUpgrade` numbers only exist compiled into each ability's own C# handler
//! class (`Ability.Implementation`, e.g. `DOL.GS.RealmAbilities.ModernAugStr`). The only honest
//! way to know them is to read that class's real source - so this parses the vendored handler
//! source directly (`resources/classes/core/realmabilities/`, vendored the same way
//! `eSpellType.cs` already is) rather than ever guessing.
//!
//! The one thing that makes this actually tractable despite there being 100+ distinct ability
//! handler classes live on the real server: confirmed by reading the real source, almost every
//! one doesn't compute its own cost at all - it just delegates straight to one of two small
//! shared static helper classes, `AtlasRAHelpers` (the older, pre-"Atlas of Camelot" era costs)
//! and `ModernRealmRanks` (the newer, consolidated "Modern" RA tier) - both vendored alongside
//! the handlers. A smaller remainder either inherits `MaxLevel`/`CostForUpgrade` unchanged from a
//! base class (walked recursively), or has its own simple, directly-parseable literal (a bare
//! `return N;`, a `switch` of literal `case N: return M;` pairs, an inline `new[] { ... }` array,
//! or a simple `(level + 1) * N` linear formula - the one pattern this project's own Divided
//! Realms "Sovereign" custom abilities use). Anything else is left explicitly unresolved
//! (`resolved: false`) rather than silently shown as a wrong number - the planner's own job is to
//! say "see the in-game trainer" for those, not guess.

use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize)]
pub struct RaCostInfo {
    pub max_level: Option<i64>,
    /// `costs[i]` is the cost to train from level `i` to level `i + 1` (0-indexed) - the same
    /// shape the real server's own `CostForUpgrade(currentLevel)` uses. `None` if this ability's
    /// cost formula wasn't one of the patterns this resolver actually understands.
    pub costs: Option<Vec<i64>>,
    /// `amounts[i]` is whatever flat numeric bonus level `i + 1` actually grants (e.g. "+12
    /// Strength" at level 2 of Augmented Strength) - best-effort, `None` far more often than
    /// `costs` is (many abilities' real effect isn't a single flat number at all).
    pub amounts: Option<Vec<i64>>,
}

// The two confirmed shared cost tables every "classic"/pre-Atlas-era ability that has more than
// one level almost always delegates straight to - `AtlasOF_RAHelpers.cs`'s own
// `GetCommonUpgradeCostFor5LevelsRA`/`GetCommonUpgradeCostFor3LevelsRA`, vendored verbatim at
// `realmabilities/atlasOF/AtlasOF_RAHelpers.cs`.
const COST_5_CLASSIC: [i64; 5] = [1, 3, 6, 10, 14];
const COST_3_CLASSIC: [i64; 3] = [3, 6, 10];

// The two confirmed shared cost tables every "Modern" (newer, consolidated) ability delegates
// to - `ModernRealmPassives.cs`'s own `ModernRealmRanks.PassiveCost`/`ActiveCost`.
const COST_9_MODERN_PASSIVE: [i64; 9] = [1, 1, 2, 3, 3, 5, 5, 7, 7];
const COST_5_MODERN_ACTIVE: [i64; 5] = [5, 5, 5, 7, 8];

// The three confirmed shared *amount* tables a "classic" stat/property-enhancer ability
// delegates to, same source file as the cost tables above.
const AMOUNT_STAT_ENHANCER_CLASSIC: [i64; 5] = [6, 12, 18, 24, 30];
const AMOUNT_PROPERTY3_CLASSIC: [i64; 5] = [3, 6, 9, 12, 15];
const AMOUNT_PROPERTY5_CLASSIC: [i64; 5] = [5, 10, 15, 20, 25];

// "Modern" per-category amount tables - `ModernRealmPassives.cs`'s own `ModernRealmRanks`
// static class (`Stats`/`Crit`/.../`Power`), each indexed the same way (`Value(rank, ...)`,
// i.e. just the Nth literal, clamped to the table's own length for any rank beyond it).
const AMOUNT_MODERN_STATS: [i64; 9] = [4, 8, 12, 17, 22, 28, 34, 41, 48];
const AMOUNT_MODERN_CRIT: [i64; 9] = [3, 6, 9, 13, 17, 22, 27, 33, 39];
const AMOUNT_MODERN_DEFENSE: [i64; 9] = [2, 4, 6, 9, 12, 15, 18, 21, 25];
const AMOUNT_MODERN_HEALING: [i64; 9] = [2, 4, 6, 9, 12, 16, 20, 25, 30];
const AMOUNT_MODERN_MAGERY: [i64; 9] = [2, 3, 4, 6, 8, 10, 12, 14, 16];
const AMOUNT_MODERN_MAGIC: [i64; 9] = [2, 3, 5, 7, 10, 12, 15, 17, 20];
const AMOUNT_MODERN_DETERMINATION: [i64; 9] = [4, 8, 12, 17, 23, 30, 38, 46, 55];
const AMOUNT_MODERN_VEIL: [i64; 9] = [10, 15, 20, 30, 40, 50, 60, 70, 80];
const AMOUNT_MODERN_HEALTH: [i64; 9] = [25, 50, 75, 100, 150, 200, 250, 325, 400];
const AMOUNT_MODERN_POWER: [i64; 9] = [15, 25, 40, 55, 75, 100, 130, 165, 200];

struct ClassEntry {
    /// The literal token right after `:` in the class declaration - `None` for a class with no
    /// base listed at all (shouldn't happen among real ability handlers, which always extend at
    /// least `RealmAbility`, but handled rather than assumed).
    base_name: Option<String>,
    body: String,
}

/// Parses every vendored realm-ability `.cs` file once up front into a name -> (base class,
/// body text) map, then resolves any `Ability.Implementation` against it - walking up the real
/// inheritance chain (recursively, depth-limited) wherever a class doesn't override
/// `MaxLevel`/`CostForUpgrade`/`GetAmountForLevel` itself.
pub struct RealmAbilityCostResolver {
    classes: HashMap<String, ClassEntry>,
}

/// Isolates every top-level `class` body in `source` (a whole file's text may declare several,
/// e.g. `SovereignRealmAbilities.cs` has four) via brace counting from each declaration's own
/// opening `{` - a plain regex alone can't safely capture "until the matching close brace" for
/// an arbitrarily nested method body.
fn extract_classes(source: &str) -> Vec<(String, Option<String>, String)> {
    let decl_re = Regex::new(
        r"(?m)^\s*public\s+(?:abstract\s+|sealed\s+|static\s+)*class\s+(\w+)(?:\s*:\s*(\w+))?",
    )
    .unwrap();
    let mut out = Vec::new();
    for m in decl_re.find_iter(source) {
        let caps = decl_re.captures(&source[m.start()..]).unwrap();
        let name = caps[1].to_string();
        let base = caps.get(2).map(|b| b.as_str().to_string());
        // Find this declaration's own opening brace, then count braces until it's balanced.
        let after_decl = &source[m.end()..];
        let Some(open_rel) = after_decl.find('{') else {
            continue;
        };
        let body_start = m.end() + open_rel + 1;
        let mut depth = 1i32;
        let mut end = body_start;
        for (i, ch) in source[body_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = body_start + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push((name, base, source[body_start..end].to_string()));
    }
    out
}

fn parse_switch_cases(body: &str, method_name: &str) -> Option<Vec<i64>> {
    let method_re = Regex::new(&format!(r"{method_name}\s*\([^)]*\)\s*\{{")).ok()?;
    let m = method_re.find(body)?;
    // Isolate just this one method's own body the same brace-counting way extract_classes does,
    // so a `case`/`return` belonging to some other method entirely can never bleed in.
    let mut depth = 1i32;
    let start = m.end();
    let mut end = start;
    for (i, ch) in body[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let method_body = &body[start..end];
    let case_re = Regex::new(r"case\s+(\d+)\s*:\s*return\s+(\d+)\s*;").ok()?;
    let mut by_index: HashMap<usize, i64> = HashMap::new();
    let mut max_idx = 0usize;
    for cap in case_re.captures_iter(method_body) {
        let idx: usize = cap[1].parse().ok()?;
        let val: i64 = cap[2].parse().ok()?;
        max_idx = max_idx.max(idx);
        by_index.insert(idx, val);
    }
    if by_index.is_empty() {
        return None;
    }
    Some((0..=max_idx).map(|i| *by_index.get(&i).unwrap_or(&0)).collect())
}

fn resolve_cost_from_body(body: &str, max_level: Option<i64>) -> Option<Vec<i64>> {
    // Only look within CostForUpgrade's own method body - matching "AtlasRAHelpers." anywhere
    // else in the class (e.g. inside CheckRequirement) must never be mistaken for this.
    let method_re = Regex::new(r"CostForUpgrade\s*\([^)]*\)\s*(=>|\{)").ok()?;
    let m = method_re.captures(body)?;
    let is_expr = &m[1] == "=>";
    let whole = m.get(0)?;
    let rest = &body[whole.end()..];
    let snippet: String = if is_expr {
        // An expression-bodied member ends at the first top-level `;`.
        rest.split(';').next().unwrap_or("").to_string()
    } else {
        // A block body - isolate it the same brace-counting way extract_classes does.
        let mut depth = 1i32;
        let mut end = 0usize;
        for (i, ch) in rest.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest[..end].to_string()
    };

    if snippet.contains("AtlasRAHelpers.GetCommonUpgradeCostFor5LevelsRA") {
        return Some(COST_5_CLASSIC.to_vec());
    }
    if snippet.contains("AtlasRAHelpers.GetCommonUpgradeCostFor3LevelsRA") {
        return Some(COST_3_CLASSIC.to_vec());
    }
    if snippet.contains("ModernRealmRanks.PassiveCost") {
        return Some(COST_9_MODERN_PASSIVE.to_vec());
    }
    if snippet.contains("ModernRealmRanks.ActiveCost") {
        return Some(COST_5_MODERN_ACTIVE.to_vec());
    }
    // An inline array literal indexed by level, e.g.
    // `new[] { 1, 1, 2, 2, 3, 3, 3, 5, 5 }[currentLevel]`.
    if let Ok(array_re) = Regex::new(r"new\[\]\s*\{([^}]+)\}\s*\[") {
        if let Some(c) = array_re.captures(&snippet) {
            let nums: Option<Vec<i64>> = c[1].split(',').map(|s| s.trim().parse().ok()).collect();
            if let Some(nums) = nums {
                return Some(nums);
            }
        }
    }
    // A linear `(level + 1) * N` formula (this project's own "Sovereign" custom abilities).
    if let Ok(linear_re) = Regex::new(r"\([a-zA-Z]+\s*\+\s*1\)\s*\*\s*(\d+)") {
        if let Some(c) = linear_re.captures(&snippet) {
            let step: i64 = c[1].parse().ok()?;
            let levels = max_level.unwrap_or(0).max(1);
            return Some((0..levels).map(|i| (i + 1) * step).collect());
        }
    }
    // A `switch` of literal `case N: return M;` pairs.
    if let Some(table) = parse_switch_cases(body, "CostForUpgrade") {
        return Some(table);
    }
    // A single bare constant for every level (e.g. `RR5RealmAbility`'s own `return 0;`).
    if let Ok(const_re) = Regex::new(r"^\s*return\s+(\d+)\s*;?\s*$") {
        if let Some(c) = const_re.captures(snippet.trim()) {
            let val: i64 = c[1].parse().ok()?;
            let levels = max_level.unwrap_or(1).max(1);
            return Some(vec![val; levels as usize]);
        }
    }
    None
}

fn resolve_amount_from_body(body: &str) -> Option<Vec<i64>> {
    let method_re = Regex::new(r"GetAmountForLevel\s*\([^)]*\)\s*(=>|\{)").ok()?;
    let m = method_re.captures(body)?;
    let whole = m.get(0)?;
    let rest = &body[whole.end()..];
    let snippet: String = if &m[1] == "=>" {
        rest.split(';').next().unwrap_or("").to_string()
    } else {
        rest.chars().take(400).collect()
    };

    let table = |name: &str| -> Option<[i64; 9]> {
        match name {
            "Stats" => Some(AMOUNT_MODERN_STATS),
            "Crit" => Some(AMOUNT_MODERN_CRIT),
            "Defense" => Some(AMOUNT_MODERN_DEFENSE),
            "Healing" => Some(AMOUNT_MODERN_HEALING),
            "Magery" => Some(AMOUNT_MODERN_MAGERY),
            "Magic" => Some(AMOUNT_MODERN_MAGIC),
            "Determination" => Some(AMOUNT_MODERN_DETERMINATION),
            "Veil" => Some(AMOUNT_MODERN_VEIL),
            "Health" => Some(AMOUNT_MODERN_HEALTH),
            "Power" => Some(AMOUNT_MODERN_POWER),
            _ => None,
        }
    };
    if let Ok(modern_re) = Regex::new(r"ModernRealmRanks\.(\w+)") {
        if let Some(c) = modern_re.captures(&snippet) {
            if let Some(t) = table(&c[1]) {
                return Some(t.to_vec());
            }
        }
    }
    if snippet.contains("AtlasRAHelpers.GetStatEnhancerAmountForLevel") {
        return Some(AMOUNT_STAT_ENHANCER_CLASSIC.to_vec());
    }
    if snippet.contains("AtlasRAHelpers.GetPropertyEnhancer3AmountForLevel") {
        return Some(AMOUNT_PROPERTY3_CLASSIC.to_vec());
    }
    if snippet.contains("AtlasRAHelpers.GetPropertyEnhancer5AmountForLevel") {
        return Some(AMOUNT_PROPERTY5_CLASSIC.to_vec());
    }
    None
}

fn resolve_max_level_from_body(body: &str) -> Option<i64> {
    let re = Regex::new(r"MaxLevel\s*(?:=>|\{\s*get\s*(?:=>|\{\s*return|\s*return)?)\s*(\d+)").ok()?;
    re.captures(body)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// The two real `ServerProperty` flags (`use_new_actives_ras_scaling`/
/// `use_new_passives_ras_scaling`) that `TimedRealmAbility`/`L5RealmAbility` each branch their own
/// `MaxLevel`/`CostForUpgrade` on - read fresh from the live server at export time, never assumed
/// (confirmed directly: both are `false` on the real Divided Realms server right now, meaning the
/// "classic" tables apply there, but this must keep working correctly if that's ever toggled).
#[derive(Debug, Clone, Copy)]
pub struct ScalingFlags {
    pub new_actives: bool,
    pub new_passives: bool,
}

impl RealmAbilityCostResolver {
    pub fn load(core_dir: &Path) -> Self {
        let mut classes = HashMap::new();
        let ra_dir = core_dir.join("realmabilities");
        for entry in walkdir::WalkDir::new(&ra_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("cs") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            for (name, base, body) in extract_classes(&source) {
                classes.insert(name, ClassEntry { base_name: base, body });
            }
        }
        Self { classes }
    }

    /// `TimedRealmAbility`/`L5RealmAbility` (confirmed directly in their own vendored source)
    /// each branch their *own* `MaxLevel`/`CostForUpgrade` on a live `ServerProperty` via an
    /// `if`/`else` this resolver's plain text-pattern parsing deliberately doesn't try to
    /// generally evaluate - unlike every other pattern this module handles, there are exactly two
    /// of these, so they're hardcoded here instead, picked by whichever branch is actually live
    /// right now (`scaling`, read fresh from the real server by the caller).
    fn base_class_override(name: &str, scaling: ScalingFlags) -> Option<(Option<i64>, Option<Vec<i64>>)> {
        match name {
            "TimedRealmAbility" if scaling.new_actives => Some((None, Some(COST_5_MODERN_ACTIVE.to_vec()))),
            "TimedRealmAbility" => Some((None, Some((0..5).map(|i| (i + 1) * 5).collect()))),
            "L5RealmAbility" if scaling.new_passives => Some((Some(9), Some(COST_9_MODERN_PASSIVE.to_vec()))),
            "L5RealmAbility" => Some((Some(5), Some(COST_5_CLASSIC.to_vec()))),
            _ => None,
        }
    }

    /// Resolves `implementation` (e.g. `DOL.GS.RealmAbilities.ModernAugStr` - the dotted
    /// namespace prefix, if any, is stripped) by walking up its real inheritance chain (depth
    /// -limited to guard against an unexpected cycle) until `MaxLevel`/`CostForUpgrade`/
    /// `GetAmountForLevel` are each found somewhere in it - an ability whose own class doesn't
    /// override one of these inherits it from its base, exactly like the real C# does.
    pub fn resolve(&self, implementation: &str, scaling: ScalingFlags) -> RaCostInfo {
        let class_name = implementation.rsplit('.').next().unwrap_or(implementation);
        let mut max_level = None;
        let mut costs = None;
        let mut amounts = None;
        let mut current = Some(class_name.to_string());
        let mut depth = 0;
        while let Some(name) = current {
            if depth > 10 {
                break;
            }
            depth += 1;
            if let Some((ml, c)) = Self::base_class_override(&name, scaling) {
                if max_level.is_none() {
                    max_level = ml;
                }
                if costs.is_none() {
                    costs = c;
                }
                break;
            }
            let Some(entry) = self.classes.get(&name) else {
                break;
            };
            if max_level.is_none() {
                max_level = resolve_max_level_from_body(&entry.body);
            }
            if costs.is_none() {
                costs = resolve_cost_from_body(&entry.body, max_level);
            }
            if amounts.is_none() {
                amounts = resolve_amount_from_body(&entry.body);
            }
            if max_level.is_some() && costs.is_some() && amounts.is_some() {
                break;
            }
            current = entry.base_name.clone();
        }
        RaCostInfo { max_level, costs, amounts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> RealmAbilityCostResolver {
        let core_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/classes/core");
        RealmAbilityCostResolver::load(&core_dir)
    }

    /// Matches the real Divided Realms server's own current ServerProperty values (confirmed
    /// live: both `use_new_actives_ras_scaling`/`use_new_passives_ras_scaling` are `false`).
    fn scaling() -> ScalingFlags {
        ScalingFlags { new_actives: false, new_passives: false }
    }

    #[test]
    fn resolves_a_modern_passive_via_its_shared_helper() {
        let r = resolver();
        let info = r.resolve("DOL.GS.RealmAbilities.ModernAugStr", scaling());
        assert_eq!(info.max_level, Some(9));
        assert_eq!(info.costs, Some(COST_9_MODERN_PASSIVE.to_vec()));
        assert_eq!(info.amounts, Some(AMOUNT_MODERN_STATS.to_vec()));
    }

    #[test]
    fn resolves_a_classic_atlas_active_via_its_shared_helper() {
        let r = resolver();
        let info = r.resolve("DOL.GS.RealmAbilities.AtlasOF_AvoidPain", scaling());
        assert_eq!(info.max_level, Some(3));
        assert_eq!(info.costs, Some(COST_3_CLASSIC.to_vec()));
    }

    #[test]
    fn resolves_an_rr5_ability_by_inheriting_its_base_classs_free_single_level() {
        let r = resolver();
        let info = r.resolve("DOL.GS.RealmAbilities.ArmsLengthAbility", scaling());
        assert_eq!(info.max_level, Some(1));
        assert_eq!(info.costs, Some(vec![0]));
    }

    #[test]
    fn resolves_a_divided_realms_custom_sovereign_ability_via_its_linear_formula() {
        let r = resolver();
        let info = r.resolve("DOL.GS.RealmAbilities.SovereignRoyalFortitude", scaling());
        assert_eq!(info.max_level, Some(3));
        assert_eq!(info.costs, Some(vec![2, 4, 6]));
    }

    #[test]
    fn resolves_a_modern_active_via_its_shared_helper() {
        let r = resolver();
        // ModernAmelioratingMelodies is a real, live-referenced Modern active ability that
        // inherits ModernGroupRealmAbility's own CostForUpgrade unchanged - picked as a
        // representative "ActiveCost resolved via a base class" case (as opposed to
        // ModernCharge, a real, confirmed exception: its own doc comment says Live 1.128c
        // deliberately uses a bespoke 3/3/3/5/6 table instead of the standard active cost).
        let info = r.resolve("DOL.GS.RealmAbilities.ModernAmelioratingMelodies", scaling());
        assert_eq!(info.max_level, Some(5));
        assert_eq!(info.costs, Some(COST_5_MODERN_ACTIVE.to_vec()));
    }

    #[test]
    fn resolves_an_ability_with_its_own_bespoke_inline_cost_table() {
        let r = resolver();
        // A real, confirmed exception to the standard Modern active cost table - see its own
        // source comment ("Live 1.128c: 3/3/3/5/6 instead of the standard 5/5/5/7/8").
        let info = r.resolve("DOL.GS.RealmAbilities.ModernCharge", scaling());
        assert_eq!(info.max_level, Some(5));
        assert_eq!(info.costs, Some(vec![3, 3, 3, 5, 6]));
    }

    #[test]
    fn an_unrecognized_implementation_resolves_to_nothing_known() {
        let r = resolver();
        let info = r.resolve("DOL.GS.RealmAbilities.ThisDoesNotExist", scaling());
        assert_eq!(info.max_level, None);
        assert_eq!(info.costs, None);
        assert_eq!(info.amounts, None);
    }
}

