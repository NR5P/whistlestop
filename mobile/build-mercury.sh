#!/bin/sh
# Builds Rhizomatica's Mercury modem (GPL-3.0) as static libraries for the
# phone apps, from a clean checkout of a release tag:
#   mobile/build-mercury.sh [ios-sim|ios|android-arm64 ...]   (default: all)
# Output: mobile/build/mercury/<target>/lib/*.a (+ include/mercury_bridge.h)
# The apps then link it through the Rust crate's `mercury` feature.
set -e
TAG=${MERCURY_TAG:-v1.9.23}
cd "$(dirname "$0")"
M=$PWD
SRC=$M/build/mercury-src-$TAG
if [ ! -d "$SRC" ]; then
  git clone -q --depth 1 --branch "$TAG" https://github.com/Rhizomatica/mercury "$SRC"
fi
SDK=${ANDROID_HOME:-$HOME/Library/Android/sdk}
NDK=${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/* 2>/dev/null | sort -V | tail -1)}
NBIN="$NDK/toolchains/llvm/prebuilt/$(ls "$NDK/toolchains/llvm/prebuilt" 2>/dev/null | head -1)/bin"

TARGETS=${*:-ios-sim ios android-arm64}
for T in $TARGETS; do
  case $T in
    ios-sim) CC="xcrun -sdk iphonesimulator clang -arch arm64 -mios-simulator-version-min=17.0"; AR="xcrun ar" ;;
    ios) CC="xcrun -sdk iphoneos clang -arch arm64 -miphoneos-version-min=17.0"; AR="xcrun ar" ;;
    android-arm64) CC="$NBIN/aarch64-linux-android26-clang"; AR="$NBIN/llvm-ar" ;;
    *) echo "unknown target $T"; exit 1 ;;
  esac
  B=$M/build/mercury/$T
  echo "== Mercury $TAG for $T"
  rm -rf "$B/src"; mkdir -p "$B/lib" "$B/include"
  # Each target builds in its own copy: Mercury builds in its source tree.
  rsync -a --exclude .git "$SRC/" "$B/src/"
  ( cd "$B/src" && \
    make -s libmercury_core.a CC="$CC" AR="$AR" WS_TLS=0 >"$B/build.log" 2>&1 ) \
    || { tail -30 "$B/build.log"; exit 1; }
  cp "$B/src/libmercury_core.a" "$B/src/modem/freedv/libfreedvdata.a" "$B/lib/"
  cp "$B/src/audioio/audioio.a" "$B/lib/libaudioio.a"
  [ $T = android-arm64 ] && cp "$B/src/radio_io/hamlib-android/arm64-v8a/lib/libhamlib.a" "$B/lib/"
  cp "$B/src/gui_interface/fyne-ui/engine/mercury_bridge.h" "$B/include/"
  ls -la "$B/lib"
done
