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
- Browsing every existing class (all four realms + shared base classes), with their real stats,
  specs, races, and profession text, parsed straight from the server's own source.
- Generating a new class file via literal inheritance from any existing one, with a live code
  preview before you deploy anything.
- Deploying the generated file to your server's `scripts/` folder and restarting it, over SSH.

**Not automated** (genuinely needs your own design judgment, or a manual SQL step this tool gives
you a box for, but doesn't have a dedicated form for yet):
- A trainer NPC spawn, starter equipment, and a starting location are needed for a smooth
  first-level experience - reuse an existing one via SQL (there's a "run SQL on the server" box
  for exactly this) or extend the class-editing form later.
- Genuinely new mechanics (a new spell effect, a new pet) still mean writing real C# by hand -
  this tool is about *cloning*, not *inventing*.
- Bot AI quality for the new class: it'll play with the generic `BotBrain` and default to the
  "Damage" party role unless you also touch the bot-side files in the server repo directly.

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
4. In **Clone a class**, pick a source class, name your new one, override only what you want to
   change, generate, preview, and deploy.

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
