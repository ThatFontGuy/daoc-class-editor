#!/bin/bash
# Runs the DAoC Class Editor in Tauri's dev mode (live reload, debug build).
#
# No --bin flag is needed here (unlike the Charplanner's own `cargo run --bin
# export_charplanner`, which does need one): that flag only matters when invoking
# cargo directly in a package with more than one binary target to disambiguate which
# one to run. `npm run tauri dev` goes through the Tauri CLI instead, which already
# knows to build/run this package's own main app binary (the one tauri.conf.json
# describes) regardless of any other binary target (like export_charplanner) that
# happens to also live in this same Cargo package.
set -e
cd "$(dirname "$0")"
npm run tauri dev
