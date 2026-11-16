#!/usr/bin/env bash
# Builds a universal (Apple Silicon + Intel) Whistlestop.app, signs it with
# a Developer ID, and zips it for a release:
#
#   scripts/macos-app.sh                 # -> dist/Whistlestop-<version>-macos.zip
#   NOTARY_PROFILE=name scripts/macos-app.sh
#
# NOTARY_PROFILE: a `xcrun notarytool store-credentials` profile; when set,
# the app is notarized and stapled, so it opens without Gatekeeper's warning.
# SIGN_ID: the signing identity (default: the first Developer ID Application).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
SIGN_ID="${SIGN_ID:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)}"
T=target/rel
OUT=dist
APP="$OUT/Whistlestop.app"

for t in aarch64-apple-darwin x86_64-apple-darwin; do
  CARGO_TARGET_DIR=$T cargo build --release --target $t --bin whistlestop
done

rm -rf "$APP" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/whistlestop" \
  $T/aarch64-apple-darwin/release/whistlestop $T/x86_64-apple-darwin/release/whistlestop

# Icon: the app draws its own; make every size macOS wants from a 1024 px one.
ICONSET=$(mktemp -d)/Whistlestop.iconset
mkdir -p "$ICONSET"
"$APP/Contents/MacOS/whistlestop" icon-png "$ICONSET/icon_512x512@2x.png" --size 1024 >/dev/null
for s in 16 32 128 256 512; do
  sips -z $s $s "$ICONSET/icon_512x512@2x.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) "$ICONSET/icon_512x512@2x.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Whistlestop.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Whistlestop</string>
  <key>CFBundleDisplayName</key><string>Whistlestop</string>
  <key>CFBundleIdentifier</key><string>com.nr5p.whistlestop</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>whistlestop</string>
  <key>CFBundleIconFile</key><string>Whistlestop</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key>
  <string>Whistlestop listens to your radio's audio (its USB sound card) to find whistles and show the waterfall.</string>
</dict>
</plist>
EOF

ENT=$(mktemp)
cat > "$ENT" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.device.audio-input</key><true/>
</dict>
</plist>
EOF

if [ -n "$SIGN_ID" ]; then
  codesign --force --options runtime --timestamp --entitlements "$ENT" --sign "$SIGN_ID" "$APP"
  codesign --verify --strict --verbose=1 "$APP"
else
  echo "No Developer ID found: the app is not signed" >&2
fi

ZIP="$OUT/Whistlestop-$VERSION-macos.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
if [ -n "${NOTARY_PROFILE:-}" ]; then
  xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  rm -f "$ZIP" && ditto -c -k --keepParent "$APP" "$ZIP"
fi
echo "$ZIP"
