#!/bin/sh
# Builds the engine for the iPhone app: WhistlestopCore.xcframework (device
# and Apple-silicon simulator), which the Xcode project links.
#   mobile/build-ios.sh [debug]
set -e
cd "$(dirname "$0")/.."
PROFILE=release; FLAG=--release
[ "$1" = debug ] && PROFILE=debug && FLAG=
export IPHONEOS_DEPLOYMENT_TARGET=17.0
# Mercury (mobile/build-mercury.sh) is linked in when it has been built.
for T in aarch64-apple-ios aarch64-apple-ios-sim; do
  case $T in *sim) MT=ios-sim ;; *) MT=ios ;; esac
  FEAT=; [ -f mobile/build/mercury/$MT/lib/libmercury_core.a ] && FEAT="--features mercury"
  [ -z "$FEAT" ] && echo "  (no Mercury build for $MT: run mobile/build-mercury.sh to include it)"
  cargo build $FLAG -p whistlestop-mobile --target $T $FEAT
done
OUT=mobile/ios/WhistlestopCore.xcframework
rm -rf "$OUT"
xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/$PROFILE/libwhistlestop_mobile.a -headers mobile/core/include \
  -library target/aarch64-apple-ios-sim/$PROFILE/libwhistlestop_mobile.a -headers mobile/core/include \
  -output "$OUT" >/dev/null
echo "  -> $OUT"
