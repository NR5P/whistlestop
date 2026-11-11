#!/bin/sh
# Builds the engine for the Android app: libwhistlestop_mobile.so for each
# ABI, into the app's jniLibs.
#   mobile/build-android.sh [debug]
# Needs the NDK (ANDROID_NDK_HOME, or the newest in ~/Library/Android/sdk/ndk).
set -e
cd "$(dirname "$0")/.."
PROFILE=release; FLAG=--release
[ "$1" = debug ] && PROFILE=debug && FLAG=
SDK=${ANDROID_HOME:-$HOME/Library/Android/sdk}
NDK=${ANDROID_NDK_HOME:-$(ls -d "$SDK"/ndk/* | sort -V | tail -1)}
HOST=$(ls "$NDK/toolchains/llvm/prebuilt" | head -1)
BIN="$NDK/toolchains/llvm/prebuilt/$HOST/bin"
API=26
export AR="$BIN/llvm-ar"
for PAIR in aarch64-linux-android:arm64-v8a x86_64-linux-android:x86_64; do
  T=${PAIR%%:*}; ABI=${PAIR##*:}
  UT=$(echo "$T" | tr 'a-z-' 'A-Z_')
  export CARGO_TARGET_${UT}_LINKER="$BIN/$T$API-clang"
  export CC_$(echo "$T" | tr '-' '_')="$BIN/$T$API-clang"
  # Mercury (mobile/build-mercury.sh) is linked in on arm64 when built;
  # its Hamlib ships for arm64 only, so x86_64 (emulators) goes without.
  FEAT=; [ $ABI = arm64-v8a ] && [ -f mobile/build/mercury/android-arm64/lib/libmercury_core.a ] && FEAT="--features mercury"
  cargo build $FLAG -p whistlestop-mobile --target $T $FEAT
  D=mobile/android/app/src/main/jniLibs/$ABI
  mkdir -p "$D"
  cp target/$T/$PROFILE/libwhistlestop_mobile.so "$D/"
  # Symbols stay in target/; the app carries the stripped library.
  [ $PROFILE = release ] && "$BIN/llvm-strip" --strip-unneeded "$D/libwhistlestop_mobile.so"
  echo "  -> $D/libwhistlestop_mobile.so"
done
