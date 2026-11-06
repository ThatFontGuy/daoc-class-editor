# Future improvements

Ideas explicitly deferred by the owner, with enough detail to pick back up without
re-researching from scratch. Not a roadmap or a commitment - just notes.

## Real descriptions for each spell `Type` value, in the Type dropdown

**Requested:** 2026-11-06, while discussing what `ClientEffect`/`Type` actually mean on a spell
(see git history around that date - "Fix a real bug: the spell field editor was missing Type
entirely" and "Make a spell's Type field a dropdown of every real eSpellType value"). The owner
asked specifically about `FOD` as an example of a cryptic value with no obvious meaning from the
name alone, and wants the dropdown (`ClassSpecsSection.tsx`'s spell field editor) to show real
descriptions of what each `Type` value mechanically does, not just the bare enum name.

**Why this isn't trivial - there's no shortcut data source:**
- Confirmed: there is no description column/table anywhere in the live database for spell types.
- Confirmed: the vendored `eSpellType.cs` enum file itself has no per-member comments either - it's
  a bare, 534-entry `enum { A, B, C, ... }` list (see `src-tauri/resources/classes/core/eSpellType.cs`,
  vendored for the dropdown's own value list).
- The *only* place a real description of what a given `Type` value actually does exists at all is
  each type's own `[SpellHandler(eSpellType.X)]`-tagged C# class, scattered across the real server
  source tree (`/Users/thatfontguy/Documents/PROJECTS/daoc-divided-linux/source/server/GameServer/`
  on this machine, as of this writing) - not anything bundled with or reachable by this app today.

**Concrete scope, measured directly against that source tree (2026-11-06):**
- 534 total `eSpellType` enum values.
- 438 of those have a findable `[SpellHandler(eSpellType.X)]` attribute *somewhere* in the whole
  `GameServer/` tree (`grep -rhoE "\[SpellHandler\(eSpellType\.\w+\)\]"`, deduplicated). The
  remaining ~96 appear to have no explicit handler at all - likely legacy/reserved/unused values;
  worth spot-checking a few before assuming that's uniformly true.
- Handler classes aren't confined to one folder - found under at least `spells/` (flat, plus
  subfolders like `spells/Warlock/`, `spells/Vampiir/`, `spells/Archery/`, `spells/Masterlevel/`),
  `realmabilities/`, `realmabilities_atlasOF/`, and a handful of one-off locations (`bots/`,
  `ECS-Effects/`, `styles/`). A real extraction pass needs to walk the whole tree, not just
  `spells/`.
- Of the handlers found, many (but not all) override a `ShortDescription` property with a
  player-facing summary - this is the actual source of truth to extract, e.g.:
  - Simple, single-expression case (easy to regex-extract):
    `spells/DirectDamageSpellHandler.cs`: `public override string ShortDescription => $"Inflicts {Spell.Damage} {Spell.DamageTypeToString()} damage to the target.";`
  - Conditional/multi-line case (harder - needs real parsing, not just a regex grab):
    `spells/SpeedDecreaseSpellHandler.cs`:
    ```csharp
    public override string ShortDescription =>
        Spell.Value >= 99 ?
        "The target is rooted in place." :
        $"The target is slowed by {Spell.Value}%.";
    ```
  - Many handlers don't override `ShortDescription` at all (inherit a generic default from their
    base class) - these would need either a fallback ("no further detail available") or walking
    up the inheritance chain to find whichever ancestor *does* override it.
- A handful of types are implemented with no dedicated file at all (e.g. `FOD`, found inside
  `spells/Masterlevel/Perfecter.cs` alongside several sibling Masterlevel handlers) - one file can
  define many handler classes, so "one handler = one file" can't be assumed when writing a scanner.

**What `FOD` turned out to mean, found by hand (2026-11-06), as a concrete worked example:**
`FODSpellHandler` (`spells/Masterlevel/Perfecter.cs`) is part of the "Perfecter" Masterlevel line
(the comment above it in-source literally says `// Perfecter-9`) - it summons a stationary
`GameFont` object (a placed, pulsing AoE totem) that repeatedly casts a `PowerRend` (power drain)
sub-spell on nearby enemies. Almost certainly short for "Font of Destruction," based on the model/
icon reused (`7310`) and the sibling classes nearby, though that exact phrase isn't written
anywhere verbatim in the source - this was inferred from the code's own behavior, not read off a
label.

**Proposed approach, if/when this gets picked up:**
1. Write a one-off (not shipped with the app) extraction script/tool that walks a local checkout
   of the server source tree, finds every `[SpellHandler(eSpellType.X)]` attribute, and for each,
   attempts to extract that class's own `ShortDescription` (walking up the inheritance chain if
   the class itself doesn't override it) - best-effort text extraction, not a full C# parser;
   flag/skip anything too complex to extract cleanly (log it for manual follow-up rather than
   guessing).
2. Output a single generated data file (JSON, keyed by `eSpellType` name) - *not* hand-maintained
   prose, and *not* the full C# source tree vendored into the app (that would be a much heavier,
   unnecessary bundle just to get ~500 short strings out of it).
3. Vendor that generated JSON file the same way `eSpellType.cs` itself already is
   (`src-tauri/resources/classes/core/`), parse it in `classdata.rs` similarly to
   `load_known_options`, and expose it to the frontend (extend `KnownOptions` again, e.g.
   `spell_type_descriptions: HashMap<String, String>`).
4. In the dropdown (`ClassSpecsSection.tsx`'s `renderDraftField`), append the description (if any)
   to each `<option>`'s label, and/or show it as a one-line hint below the dropdown once a value is
   selected - whichever reads better once there's real data to look at.
5. Re-run the extraction step whenever `eSpellType.cs` is re-vendored from a newer server source,
   same maintenance rhythm as every other vendored reference file already has.

**Open questions to resolve before starting:**
- Is a best-effort regex/line-based extraction good enough, or does the "harder" conditional case
  above (and others like it) actually matter enough to warrant a tiny purpose-built parser instead?
- For the ~96 types with no findable handler at all - confirm they're genuinely dead/unused (and
  skip them cleanly) rather than silently mis-scanned.
- Decide the "no description available" UI treatment (omit entirely vs. a visible placeholder like
  "no further detail available").
