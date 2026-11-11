# Whistlestop for iPhone and Android

Native apps (SwiftUI, Jetpack Compose) over the same Rust engine as the
desktop app, with Rhizomatica's Mercury modem built in. GPL-3.0-or-later
(Mercury is GPL-3.0).

```
mobile/
  core/           whistlestop-mobile: the engine's C interface (Swift) and JNI (Kotlin)
    include/      whistlestop.h + module.modulemap (Swift imports WhistlestopCore)
    src/mercury.rs  starts Mercury in-process (feature `mercury`)
  ios/            the iPhone app (XcodeGen project.yml, Swift sources, UI tests)
  android/        the Android app (Gradle, Kotlin sources, UI tests)
  build-mercury.sh  Mercury v1.9.23 as static libraries for each phone target
  build-ios.sh      engine (+ Mercury) -> ios/WhistlestopCore.xcframework
  build-android.sh  engine (+ Mercury) -> android/app/src/main/jniLibs
```

## How it fits together

```
 SwiftUI / Compose screens
        │  JSON: Cmd in, State and Config out (ws_* / Core.*)
 whistlestop-mobile (Rust)  ──  the Whistlestop engine (whistle, search, CQ, link, Depot…)
        │  TCP 127.0.0.1:8300/8301, 8100 — exactly as on a desktop
 Mercury (C, mercury_init on its own threads)  ──  phone audio (CoreAudio / AAudio)
```

The engine's `State`, `Cmd` and `Config` are serde types already (remote
operation sends them over the LAN), so the bridge is small:

| C (Swift) | Kotlin `Core.` | does |
|---|---|---|
| `ws_start(dir)` | `start(dir)` | load `dir/whistlestop.toml`, start Mercury (real station, local modem) and the engine |
| `ws_changes()` | `changes()` | counter that moves when the state changes (the apps poll at 10 Hz) |
| `ws_state()` | `state()` | the whole state, JSON |
| `ws_rows()` | `rows()` | new waterfall rows, `[[dB]]` (256 bins over 0–4 kHz) |
| `ws_send(json)` | `send(json)` | one `Cmd`, e.g. `{"Connect":"K7BOT"}`, `"StopSearch"` |
| `ws_config()` / `ws_set_config(json)` | `config()` / `setConfig(json)` | settings; setting saves, applies, and starts Mercury if needed |
| `ws_mercury_embedded()` | `mercuryEmbedded()` | is Mercury running in the app |
| — | `setAudioDevices(list)` | Android's sound devices for Mercury (`id\tin|out\tname`) |
| `ws_stop()`, `ws_free(s)` | `stop()` | |

Mail ids are unsigned 64-bit: Kotlin keeps them as their exact digits
(kotlinx `JsonPrimitive.content`) and sends them back unchanged.

## Building

Needs Rust with the targets `aarch64-apple-ios aarch64-apple-ios-sim
aarch64-linux-android x86_64-linux-android`, Xcode, XcodeGen
(`brew install xcodegen`), and the Android SDK + NDK (Android Studio's JDK).

```bash
mobile/build-mercury.sh          # once (or per Mercury release): ios-sim, ios, android-arm64
mobile/build-ios.sh              # -> mobile/ios/WhistlestopCore.xcframework
mobile/build-android.sh          # -> mobile/android/app/src/main/jniLibs/{arm64-v8a,x86_64}
```

iPhone: `cd mobile/ios && xcodegen`, then open `Whistlestop.xcodeproj`, or:

```bash
xcodebuild test -project mobile/ios/Whistlestop.xcodeproj -scheme Whistlestop -destination 'platform=iOS Simulator,name=<an iPhone simulator>'
```

Android (always name the emulator: a phone on wireless debugging is a
device too):

```bash
cd mobile/android && ANDROID_SERIAL=emulator-5554 ./gradlew connectedDebugAndroidTest
```

Without `build-mercury.sh` the apps still build; they then need a Mercury
elsewhere on the network (Settings → Modem) unless in Demo mode. Android's
Mercury is arm64 only (its Hamlib ships for arm64); the x86_64 library,
for Intel emulators, goes without.

## Tests

- `cargo test -p whistlestop-mobile`: the bridge's JSON round trip
  (start, state, command, settings).
- iPhone UI tests (`WhistlestopUITests`, 4) and Android UI tests
  (`WhistlestopUiTest`, 4), real taps against the demo station: connect →
  chat → reply → disconnect; search → lock → Answer → link; Depot mail;
  Help mode. Launched with `WS_UITEST=1` / `--ez ws_uitest true`, the app
  starts a fresh demo station in a scratch folder.

## Android USB radios

Android apps can't open `/dev/ttyUSB0`, so `UsbCat.kt` opens the radio's
USB serial chip with usb-serial-for-android (MIT; CP210x, FTDI, CH34x,
PL2303, CDC) and the engine's CAT drivers read and write through it over
JNI (`mobile/core/src/lib.rs`, module `usb`; `whistlestop::cat::UsbHost`).
Ports are named `usb:VVVV:PPPP`. Whistlestop serves the radio as rigctld on
`127.0.0.1:4532`, and the embedded Mercury is started with `-R 2 -A
127.0.0.1:4532` once that answers, so it keys the radio. A USB audio device,
when present, is passed to Mercury as `-i/-o dev:<id>`. Mercury's engine
can't be restarted in-process (`mercury_stop` is never reset), so a change
of keying needs the app reopened. Tested so far: the protocol probe over a
simulated USB host (`cargo test radio_through_a_usb_host`) and the emulator
UI tests; not yet on a real radio.

## Status

Working on both: search, whistle, Heard/Answer, CQ, connect, chat, QSY
request, Away, the Depot, Log, settings, Help mode, the waterfall; Mercury
embedded and serving its TNC to the engine (checked in the iPhone simulator:
audio open, engine connected, callsign and listen mode set).

Not yet: files and the library, Roundtables, the schedule, radio controls,
tuner/SWR, the RTL-SDR, remote operation of a desktop station,
choosing the sound device in the app, and a foreground service so Android
keeps running with the screen off.
