# Charplanner

A self-contained, static Character Planner website for Divided Realms - modeled on
[Eden's own character planner](https://eden-daoc.net/charplan), styled to match the Divided
Realms server page (https://whozon.ca/othlyn). Draws its class/spec/spell/style/Realm Ability data
straight from the live server's own database (via the same SSH connection the desktop Class
Editor app already uses), with real spec-point and Realm Ability training math ported directly
from the server's own source - not guessed or approximated.

## What's in this folder

- `index.html` - landing page (edit the intro copy for your own server).
- `planner.html` / `planner.js` - the actual Character Planner.
- `get-started.html` - install instructions (copy of the Othlyn download page's own content).
- `assets/charplanner.css` - shared styling across all three pages.
- `data/classes.json` - **generated, not committed to git** - the actual class data export. You
  must regenerate this yourself before the site will show anything real (see below).

Nothing in this folder needs a build step. It's plain HTML/CSS/JS, meant to be copy-pasted
straight onto a webserver as-is.

## Regenerating the data

From the main project's `src-tauri` folder, run:

```
cargo run --bin export_charplanner
```

This reuses the exact same server connection settings already saved in the desktop Class Editor
app (Server connection panel) - nothing else to configure. It's entirely read-only; it never
writes anything to the live server, so there's no write-safety gate to satisfy first, and no need
to stop the server beforehand.

It writes `data/classes.json` and automatically re-stamps `planner.html`'s own script tag with a
fresh cache-busting version number, so returning players' browsers won't keep showing a stale
cached copy of `planner.js` after you re-upload.

Re-run this any time you change a class's specs/spells/styles in the Class Editor app and want the
planner to reflect it.

## Deploying

Upload this whole folder (including the freshly regenerated `data/classes.json`) to your
webserver, preserving the folder structure. That's it - no server-side code, no database
connection from the public-facing site at all.

## Known limitations (by design, not bugs)

- **Realm Abilities tab/Misc** (Eden's own planner has these too) - training costs and granted
  amounts are resolved from the server's own vendored source
  (`src-tauri/resources/classes/core/realmabilities/`), not hand-typed - see
  `src-tauri/src/realm_ability_costs.rs`'s own module doc comment for exactly how. Realm Rank
  (top toolbar) sets the real Realm Ability point budget
  (`AbstractServerRules.GetPlayerRealmPointsTotal`), enforced the same way spec points are.
- Autotrain is modeled (which specs autotrain is read straight from each class's own
  `AutotrainableSkills` field in its `.cs` source - see `ClassInfo::autotrainable_specs`) - matched
  by the spec's real `KeyName`, not its display name, so a spec this tool actually cloned for a
  custom class (which gets a new, suffixed `KeyName`) correctly loses autotrain eligibility the
  same way it would in the real game.
- Race/realm-rank-based stat bonuses aren't computed - this planner is about specs/spells/styles/
  Realm Abilities, not a full stat calculator.
