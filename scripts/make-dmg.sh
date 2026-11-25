#!/bin/bash
# Packages the app as a drag-to-Applications disk image. macOS only.
#
# Usage: scripts/make-dmg.sh "<path to Sportsball Jukebox.app>" <output.dmg>
set -euo pipefail

app="$1"
out="$2"
stage="$(mktemp -d)/Sportsball Jukebox"
mkdir -p "$stage" "$(dirname "$out")"

ditto "$app" "$stage/Sportsball Jukebox.app"
ln -s /Applications "$stage/Applications"
cat > "$stage/First launch - read me.txt" <<'TXT'
Sportsball Jukebox

1. Drag "Sportsball Jukebox" onto "Applications".
2. Open it from Applications. The first time, macOS says it cannot verify
   the app (it is not notarized yet). Click Done, then open
   System Settings > Privacy & Security, scroll down and click
   "Open Anyway" next to Sportsball Jukebox. Confirm.
3. If macOS asks whether the app may find devices on your local network,
   click Allow (needed for the phone remote and OSC).

You only need to do step 2 once for each new version.
TXT

rm -f "$out"
hdiutil create -volname "Sportsball Jukebox" -srcfolder "$stage" \
    -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$out"
hdiutil verify "$out"
ls -la "$out"
