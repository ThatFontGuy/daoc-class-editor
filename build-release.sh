#!/bin/bash
# Builds the DAoC Class Editor as a real installable app (release build + bundle).
#
# No --bin flag is needed here either, same reason as run-dev.sh - the Tauri CLI
# already builds/bundles this package's own main app binary specifically, unrelated
# to the separate export_charplanner binary also living in this Cargo package.
set -e
cd "$(dirname "$0")"
npm run tauri build

echo ""
echo "Build complete. Look for the app/installer under:"
echo "  src-tauri/target/release/bundle/"
find src-tauri/target/release/bundle/macos -maxdepth 1 -name "*.app" 2>/dev/null
find src-tauri/target/release/bundle/dmg -maxdepth 1 -name "*.dmg" 2>/dev/null
find src-tauri/target/release/bundle -maxdepth 2 \( -name "*.AppImage" -o -name "*.deb" -o -name "*.msi" -o -name "*.exe" \) 2>/dev/null
