# DAoC Class Editor

A desktop app (Tauri + React) to **clone an existing Dark Age of Camelot class into a new custom
class** for an [OpenDAoC](https://github.com/OpenDAoC/OpenDAoC-Core)/Offline DAoC server - like
[catastrophic-impact/daoc-offline-linux-server-bots](https://github.com/catastrophic-impact/daoc-offline-linux-server-bots)
- and deploy it to a running server over SSH. Builds for both macOS and Windows.

> Dark Age of Camelot is the intellectual property of Electronic Arts and Broadsword Online
> Games. This is an unofficial, non-commercial fan tool for a self-hosted, offline-style server;
> it is not affiliated with EA/Broadsword.

## What this actually does

The core insight this tool is built around: a new class is, at minimum, a tiny C# file that
**literally subclasses an existing class** (`public class ClassGrovekeeper : ClassDruid`) -
inheriting every specialization, spell eligibility, and equipment rule the source class has,
automatically. You rename it, give it a new numeric ID, and optionally override a handful of
fields (stats, eligible races, profession text) - nothing else needs to change.

Better still: the server has a **built-in runtime C# compiler**
(`GameServer.CompileScripts`, enabled by default via `EnableCompilation=True`) that recompiles
anything dropped into `<server-folder>/scripts/` **at every server startup** - completely
separate from the server's own `dotnet build` pipeline. So a new class doesn't need a server
rebuild or redeploy at all: this tool generates the `.cs` file, SCPs it to your server's
`scripts/` folder, and restarts the server. That's it.

This was verified for real, not just assumed: a generated file was compiled against the actual
server source (`dotnet build`, 0 errors) before this tool's design was finalized.

## What it can and can't automate

**Fully automated:**
- **Picking a class to clone, by realm then base class** - step 1 narrows to Albion, Midgard, or
  Hibernia; step 2 shows only the archetypes actually used in that realm (Fighter, Acolyte,
  Rogue, ...; most classes are realm-locked already, so there's no point browsing one you can't
  use); step 3 lists the concrete classes descended from that archetype (e.g. Fighter →
  Armsman/Paladin/Mercenary/Reaver/Mauler) to pick the actual clone source from.
- Generating a new class file via literal inheritance from any existing one, with a live code
  preview before you deploy anything.
- **Spec line assignment** - the real fix this needed: specializations are assigned per class by
  raw numeric ID in the server's database (`ClassXSpecialization`), with zero inheritance from
  whatever the class's `.cs` file subclasses. Without this, a cloned class would have no specs, no
  combat styles, and no weapon/armor proficiency at all. Pick each spec to **reuse** (the exact
  same spec, shared with every class that already has it) or **clone & rename** (an independent
  copy - its own `Specialization` row, `SpellLine`(s), `LineXSpell` entries, and `Style` rows,
  with weapon/armor-class locks correctly repointed to the new class).
- Deploying the generated file to your server's `scripts/` folder and restarting it, over SSH or
  locally.
- **Listing and deleting already-deployed classes** - a "Deployed classes" tab lists every
  custom class currently in `scripts/` (parsed straight from each file's
  `[CharacterClass(id, "Name", "Base")]` attribute), with a one-click delete that removes the
  `.cs` file, its `ClassXSpecialization` rows, and - optionally - every `Specialization`/
  `SpellLine`/`LineXSpell`/`Style` row this tool cloned specifically for it (identified precisely,
  because cloned spec keys always embed the owning class's ID, e.g. `earthshatter_64` - a reused
  spec shared with other classes is never touched). Handy for cleaning up a clone that didn't
  work out while debugging, or just as routine housekeeping.
- **Save/Load a project** (top-right of the window) - designing a class is something you iterate
  into existence over multiple sessions, not finish in one sitting; a project file captures the
  source class, every override, and every spec assignment so far.

**Not automated yet** (planned next, see the phased plan in project history/issues, or ask):
- Individual spell and melee-style *details* (damage, duration, range, growth rate, procs, etc.) -
  currently you can only assign/clone whole spec lines, not edit what's inside them field by
  field yet.
- A trainer NPC spawn, starter equipment, and a starting location are needed for a smooth
  first-level experience - reuse an existing one via SQL (there's a "run SQL on the server" box
  for exactly this) or extend the class-editing form later.
- Genuinely new mechanics (a new spell effect, a new pet) still mean writing real C# by hand -
  this tool is about *cloning*, not *inventing*.
- Bot AI quality for the new class: it'll play with the generic `BotBrain` and default to the
  "Damage" party role unless you also touch the bot-side files in the server repo directly.
- **Not safety-checked:** deleting a class currently in use by an existing character isn't
  blocked by this tool - it would leave that character in a broken state. Only delete classes
  nothing has played yet, and stop the server first (same database-lock caution as deploying).

## Setup

1. `npm install`
2. `npm run tauri dev` (development) or `npm run tauri build` (produces an installer for your
   current platform; run on each OS you want a build for - macOS build -> `.dmg`, Windows build
   -> `.msi`/`.exe`).
3. In the app's **Server connection** tab:
   - **If the server runs on the same machine as this app**, leave Host blank and just set the
     server's folder as a local path - no SSH at all, direct filesystem/process access.
   - **If the server runs elsewhere**, fill in Host/user/port/identity file too - this reuses
     your existing SSH keys/agent, nothing is sent anywhere except directly to your own server.
   - Either way, also set the start/stop script's filename (`daoc-server.sh` by default, but may
     be renamed).
4. In **Clone a class**: pick a source class, name your new one, assign its spec lines (reuse or
   clone & rename each), override only what stat/race fields you want to change, generate,
   preview, and deploy. Use **Save**/**Load** (top-right) any time to pick this back up later.

## Where the reference data comes from

`src-tauri/resources/classes/` is a vendored, periodically-updated copy of the relevant files from
the server repo (`playerclasses/`, `trainer/`, and a handful of core enums/base classes) - bundled
with the app so it doesn't need the server repo checked out locally. If the server repo's class
system changes meaningfully, re-sync this folder from there.

## Development

Rust backend: `cargo test --manifest-path src-tauri/Cargo.toml` runs the parser/codegen test
suite, including tests that assert the parser extracts exact known values (e.g. Sluaghbinder's
real stats) and that generated code has balanced braces and only emits the overrides you actually
changed.
