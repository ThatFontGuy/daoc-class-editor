# DAoC Class Editor

A desktop app (Tauri + React) to **clone an existing Dark Age of Camelot class into a new custom
class** for an [OpenDAoC](https://github.com/OpenDAoC/OpenDAoC-Core)/Offline DAoC server - like
[catastrophic-impact/daoc-offline-linux-server-bots](https://github.com/catastrophic-impact/daoc-offline-linux-server-bots)
- and deploy it to a running server over SSH. Builds for both macOS and Windows.

> Dark Age of Camelot is the intellectual property of Electronic Arts and Broadsword Online
> Games. This is an unofficial, non-commercial fan tool for a self-hosted, offline-style server;
> it is not affiliated with EA/Broadsword.

**Compatibility:** this targets **OpenDAoC-Core-derived servers running a SQLite-backed
database** - confirmed to work against at least two independent forks sharing that same schema
(this project's own test server, and `catastrophic-impact/daoc-offline-linux-server-bots`). It
talks to the database by shelling out to the local `sqlite3` CLI (directly, or over SSH), against
a hardcoded `data/opendaoc.sqlite3.db` path - a **MySQL-backed** OpenDAoC server (OpenDAoC-Core
supports both backends) isn't supported today, and a genuinely non-OpenDAoC-lineage server (e.g.
an older vanilla DOLSharp fork with a divergent schema) is untested and likely to fail outright.

## What this actually does

The core insight this tool is built around: a new class is, at minimum, a tiny C# file that
**literally subclasses an existing class** (`public class ClassGrovekeeper : ClassDruid`) -
inheriting every specialization, spell eligibility, and equipment rule the source class has,
automatically. You rename it, give it a new numeric ID, and optionally override a handful of
fields (stats, eligible races, profession text) - nothing else needs to change.

Better still: the server has a **built-in runtime C# compiler**
(`GameServer.CompileScripts`, enabled by default via `EnableCompilation=True`) that recompiles
anything dropped into `<server-folder>/bin/scripts/` **at every server startup** - completely
separate from the server's own `dotnet build` pipeline. So a new class doesn't need a server
rebuild or redeploy at all: this tool generates the `.cs` file, SCPs it to your server's
`bin/scripts/` folder, and restarts the server. That's it. (Note: this is `bin/scripts/`, not a
top-level `scripts/` folder - the server's own `RootDirectory` always resolves to wherever its own
executable lives, confirmed directly in `GameServerConfiguration`'s constructor, and
`tools/build-release.sh` doesn't symlink `scripts` back out to the top level the way it does for
`config`/`navmesh`/`logs`.)

This was verified for real, not just assumed: a generated file was compiled against the actual
server source (`dotnet build`, 0 errors) before this tool's design was finalized, and the actual
deployment path was confirmed against a real running server after an early mix-up (see history).

## What it can and can't automate

**Fully automated:**
- **Picking a class to clone, by realm then base class** - step 1 narrows to Albion, Midgard, or
  Hibernia; step 2 shows only the archetypes actually used in that realm (Fighter, Acolyte,
  Rogue, ...; most classes are realm-locked already, so there's no point browsing one you can't
  use); step 3 lists the concrete classes descended from that archetype (e.g. Fighter →
  Armsman/Paladin/Mercenary/Reaver/Mauler) to pick the actual clone source from - **including any
  custom classes you've already deployed**, so you can clone a clone to refine it further (e.g.
  turn a first-draft Grovekeeper into a second, tweaked version) without starting from scratch
  each time. A deployed class with no race override of its own still correctly inherits races
  (and therefore realm) from whatever it was cloned from, however many levels deep.
- Generating a new class file via literal inheritance from any existing one, with a live code
  preview before you deploy anything.
- **Spec line assignment** - the real fix this needed: specializations are assigned per class by
  raw numeric ID in the server's database (`ClassXSpecialization`), with zero inheritance from
  whatever the class's `.cs` file subclasses. Without this, a cloned class would have no specs, no
  combat styles, and no weapon/armor proficiency at all. Every spec you add is always cloned -
  its own independent copy of the `Specialization` row, `SpellLine`(s), `LineXSpell` entries,
  `Style` rows, and `Style` procs (`StyleXSpell`) - which you can rename and edit separately (see
  the **Spells** section of the **Classes** tab, below). This used to also offer "reuse" (sharing
  the exact same spec, unchanged,
  with the class you cloned from), but that was removed after a real, reproduced bug: the server
  looks up whether a class can use a style by an *exact* class-ID match, so a reused spec whose
  styles happened to be locked to some other class (not even necessarily the class you cloned from
  - a shared spec can be locked to an entirely unrelated third class) would train and show a level
  with zero usable styles, invisibly, with no error anywhere. Cloning always repoints any
  class-locked style to your new class, so this can't happen.
- **Every cloned spell is its own independent copy, not a shared pointer** - a second, deeper
  version of the same gap: `LineXSpell`/`StyleXSpell` entries used to just reuse the *source*
  spell's own literal `SpellID` verbatim, so a clone's spells were never actually independent data
  - editing one's damage/range/duration/etc. would have silently edited it for every class/spec
  line referencing that same row, including the class you cloned from. Every spell (cast or proc)
  a clone needs now gets its own fully independent `Spell` row, tracked internally so it can later
  be safely edited or deleted without ever touching a row some other class still depends on.
- Deploying the generated file to your server's `bin/scripts/` folder and restarting it, over
  SSH or locally.
- **One unified "Classes" tab, organized by realm** - every class in the game shows up here by
  name: the ~63 vendored reference classes this tool ships data for, *and* anything you've
  already deployed with it, each tagged with its real status (**Standard Class** vs **Cloned
  Class**). No more needing to already know a class's raw numeric ID just to reach it (a real
  limitation this used to have for a class compiled straight into the server, like a
  native-client-slot class promoted out of `bin/scripts/` - see "Compatibility with Divided
  Realms" below) - picking it by name here is enough. Selecting a class shows its details and its
  own spells together, in one place:
  - **Clone this class** opens the clone wizard with it preselected as the source.
  - For a class *you* deployed (a **Cloned Class**), **Edit class file** opens it back up in the
    clone wizard with its name/ID/filename locked (so redeploying overwrites the same file, not a
    new one) and every stat/profession/race field pre-filled with its *current* values, diffed
    against whatever it actually subclasses - change what you want and redeploy. **Delete class**
    removes the `.cs` file, its `ClassXSpecialization` rows, and - optionally - every
    `Specialization`/`SpellLine`/`LineXSpell`/`Style`/`StyleXSpell` row this tool cloned
    specifically for it (identified precisely, because cloned spec keys always embed the owning
    class's ID, e.g. `earthshatter_64`), plus every independently-owned `Spell` row that cleanup
    leaves orphaned. Handy for fixing or cleaning up a clone that didn't work out while debugging,
    or just as routine housekeeping.
  - The **Spells** section below the class's own details lists every spell line one of its own
    cloned specs owns, in level order, with an editable level per spell. Field-editing a spell's
    values (damage, range, duration, icon, ...) clones it independently for this class the moment
    you hit **Save**, if it isn't an independent copy already - no separate "Migrate" step to run
    first, and no risk of also changing it for the class it was cloned from, or any other class.
  - **Update All Class Data** re-reads the class list and the selected class's own spells fresh
    from the server - handy after deploying or editing something outside this tool (e.g. directly
    via the "run SQL" box, or a server restart that changed `bin/scripts/`).
- **Save/Load a project** (top-right of the window) - designing a class is something you iterate
  into existence over multiple sessions, not finish in one sitting; a project file captures the
  source class, every override, and every spec assignment so far. This is unrelated to the
  **Classes** tab above - it's purely a local file on your own computer, not anything live on the
  server.
- **A trainer, so the class is actually playable** (step 7, after deploying) - nothing beyond the
  game client's built-in class list is directly selectable at character creation, so (exactly
  like every real post-launch class, Sluaghbinder included) a player creates a character as the
  realm-appropriate archetype your class ultimately descends from (resolved automatically, however
  many clone generations deep - e.g. a class cloned from Shadowblade resolves to Midgard Rogue,
  not Shadowblade itself) and a trainer NPC offers to promote them into your class at a level you
  choose. Generates and deploys the trainer script, then spawns the actual NPC by deep-cloning an
  existing trainer's full stats/model/position (search by name, e.g. "Shadowblade Trainer", pick
  a location, done) - no hand-written C# or manual `Mob` row needed.

**Not automated yet** (planned next, see the phased plan in project history/issues, or ask):
- Starter equipment and a starting location aren't generated yet for a smooth first-level
  experience - reuse an existing one via SQL (there's a "run SQL on the server" box for exactly
  this) or extend the class-editing form later.
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
4. In **Classes**: pick any class, by realm, to see its details and its own spells. Click
   **Clone this class** to open the clone wizard with it preselected as the source - name your
   new class, assign its spec lines (each clones as its own independent copy, optionally rename
   each), override only what stat/race fields you want to change, generate, preview, and deploy.
   Use **Save**/**Load** (top-right) any time to pick this back up later. (For a class you
   already deployed yourself, **Edit class file** opens the same wizard pre-filled with its
   current values instead.)
5. Back in **Classes**, field-edit a class's own independently-cloned spells (damage, range,
   duration, icon, ...) - Save clones a spell for this class automatically the first time it's
   actually touched, so there's no separate step to run
   first.

## Where the reference data comes from

`src-tauri/resources/classes/` is a vendored, periodically-updated copy of the relevant files from
the server repo (`playerclasses/`, `trainer/`, and a handful of core enums/base classes) - bundled
with the app so it doesn't need the server repo checked out locally. If the server repo's class
system changes meaningfully, re-sync this folder from there.

## Compatibility with Divided Realms (github.com/ThatFontGuy/daoc-divided-linux)

This fork of the server (used by its own `daoc-server.sh`/`bin/scripts/` layout, confirmed directly
against its source) renumbered several native-client-slot classes: `Drengr` (64, the slot Midgard's
retired `Stormforged` used to hold), `PendragonSovereign` (65), `DeathKnight` (66),
`TuathaSovereign` (67), and dormant `Stormforged` (70, kept for save/client compatibility). All five
are vendored here (`eCharacterClass.cs` and their own `playerclasses/`/`trainer/` files), so they
show up by name in the **Classes** tab like any other class - tagged **Standard Class** (not
**Cloned Class**) since they're compiled into that server's own `GameServer.dll`, not loose files
in `bin/scripts/`. That also means **Edit class file**/**Delete class** aren't offered for them
(there's no file in `bin/scripts/` for this tool to manage) - a custom class promoted out of
`bin/scripts/` into compiled source this way needs a real server rebuild to edit, same as any
other server-side code change. Their own spec/spell data, however, lives entirely in the database
like any class's does, so the **Spells** section still works on them exactly the same as any
other class's own cloned specs.

New custom classes always get an ID of **at least 100** (`classdata::next_free_id_from`), regardless
of what the highest actually-visible ID is - deliberately well clear of every native slot a
supported fork is known to use (64-70 here), so a class this tool can't see on a particular server
can never collide with a new one it suggests.

## Development

Rust backend: `cargo test --manifest-path src-tauri/Cargo.toml` runs the parser/codegen test
suite, including tests that assert the parser extracts exact known values (e.g. Sluaghbinder's
real stats) and that generated code has balanced braces and only emits the overrides you actually
changed.
