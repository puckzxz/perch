#!/bin/bash
#
# Assemble perch.app around an already-built binary.
#
#   packaging/macos/bundle.sh <binary> <output-dir>
#
# macOS takes an app's icon, name and dock behaviour from a bundle, not from
# anything the process does at runtime and not from the executable's own
# resources — which is the whole difference from Windows, where `build.rs`
# stamps the icon into the .exe and there is nothing else to build. A loose
# binary still runs perfectly well; it just has no icon, and activating it
# behaves oddly because the system has nothing to activate.
#
# A script rather than thirty lines inside the release workflow, because this
# is the part worth being able to run by hand: the failure mode of a bundle is
# a thing that launches and looks wrong, which no CI assertion catches and a
# person spots in about a second.

set -euo pipefail

BINARY=${1:?usage: bundle.sh <binary> <output-dir>}
OUTDIR=${2:?usage: bundle.sh <binary> <output-dir>}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)

APP=$OUTDIR/perch.app
ICO=$ROOT/crates/perch/assets/perch.ico

# The same icon Windows uses, so there is one source of truth for it. Adding a
# committed .icns beside the .ico would be a second file to keep in step, and
# the one that drifts is always the one nobody looks at.
[ -f "$ICO" ] || { echo "no icon at $ICO" >&2; exit 1; }
[ -f "$BINARY" ] || { echo "no binary at $BINARY" >&2; exit 1; }

# The version, from the workspace manifest that now holds the only copy.
#
# It used to be read out of `crates/perch/Cargo.toml`, on the grounds that
# `version` was the one key there not inherited from the workspace. That stopped
# being true: every crate says `version.workspace = true` now, so perch's own
# manifest carries no number to find and this read came back empty.
#
# Scoped to the [workspace.package] block rather than taking the first match in
# the file, because the root manifest also has a [profile.profiling] section and
# dependency entries, and any of them could grow a `version = ` line that would
# otherwise be picked up instead — silently, and with a plausible-looking value.
VERSION=$(awk -F'"' '
    /^\[workspace\.package\]/ { in_section = 1; next }
    /^\[/                     { in_section = 0 }
    in_section && /^version = / { print $2; exit }
' "$ROOT/Cargo.toml")
case $VERSION in
    [0-9]*.[0-9]*) ;;
    *) echo "read '$VERSION' as the version, which is not one" >&2; exit 1 ;;
esac

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# --- icon ---------------------------------------------------------------------
# sips reads .ico and hands back its largest image, which in ours is a 256px
# PNG. Everything below that is downscaled from it; 512 is upscaled, and is
# there only because Finder's largest view would otherwise upscale it anyway
# and this way the softness is decided here rather than at display time.
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
sips -s format png "$ICO" --out "$WORK/icon.png" >/dev/null

mkdir -p "$WORK/perch.iconset"
for spec in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" \
            "128 128x128" "256 128x128@2x" "256 256x256" "512 256x256@2x"; do
    set -- $spec
    sips -z "$1" "$1" "$WORK/icon.png" --out "$WORK/perch.iconset/icon_$2.png" >/dev/null
done
iconutil -c icns "$WORK/perch.iconset" -o "$APP/Contents/Resources/perch.icns"

# --- payload ------------------------------------------------------------------
cp "$BINARY" "$APP/Contents/MacOS/perch"
chmod +x "$APP/Contents/MacOS/perch"

# Generated rather than kept as a template file next to this script, so the
# version substitution has nowhere to go stale.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>perch</string>
	<key>CFBundleIconFile</key>
	<string>perch</string>
	<key>CFBundleIdentifier</key>
	<string>com.github.puckzxz.perch</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>perch</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$VERSION</string>
	<key>CFBundleVersion</key>
	<string>$VERSION</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.entertainment</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
PLIST

# --- signature ----------------------------------------------------------------
# Ad-hoc, which is not a substitute for a Developer ID and is not trying to be.
# An arm64 binary must carry *some* signature to execute at all, and copying
# files into the bundle invalidates the one the linker applied — so without
# this the app is killed on launch. Gatekeeper still quarantines the download;
# RUNNING-macos.txt is where that is explained.
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --strict "$APP"

plutil -lint "$APP/Contents/Info.plist" >/dev/null
echo "built $APP ($VERSION)"
