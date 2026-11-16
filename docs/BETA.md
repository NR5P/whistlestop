# Whistlestop beta: release checklist

For whoever builds a beta and sends it to testers. What the testers read is
in the User Guide (built into the app, 📖 Guide): **Getting started on the
air**, **Tested radios** and **Reporting a problem**.

## Version

- A beta is a **git tag**, `v0.1.0-beta.N`: `version` in `Cargo.toml` stays
  0.1.0. Tag the next number for each build sent out.
- The app shows the version at the right of the **Log** tab's buttons, and
  in brackets the build, from `git describe` (`build.rs`): the tag for a
  release build, `Whistlestop 0.1.0 (v0.1.0-beta.2)`; for a build made
  between tags, the last tag, the commits since and the commit
  (`v0.1.0-beta.2-3-g9e7faad1c`), with `-dirty` for uncommitted changes;
  "unknown" without git.
- The same line heads every saved Log. The HELLO each link starts with, and
  remote operation's version check, use `Cargo.toml`'s version only.

## Before a build goes out

- [ ] `git pull --rebase` onto the latest `origin/main`.
- [ ] `cargo test --release` passes (on the Windows PC: in WSL, with
      `CARGO_TARGET_DIR` under `~/target/`).
- [ ] The two-station bench passes (`docs/BENCH.md`).
- [ ] Protocol or engine changes since the last beta: the RF bench (FTDX3000
      and QMX into dummy loads) passes CQ and answer, chat, a file, the
      Station ID and a search answering a CQ.
- [ ] The Windows build compiles (below).
- [ ] The Guide's **Tested radios** and the known limitations below are up
      to date.
- [ ] Everything is committed and pushed; then tag `v0.1.0-beta.N` and push the tag.

## Building

**Windows**, in PowerShell, with a target folder that isn't the running
app's (`whistlestop-target` holds the exe in use):

```powershell
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\whistlestop-target-beta"
cargo build --release --bin whistlestop
```

The program is `whistlestop.exe` in `$env:CARGO_TARGET_DIR\release`.

**macOS**, on the Mac:

```bash
cargo build --release --bin whistlestop
```

The program is `target/release/whistlestop`. Built on Apple silicon it runs
only on Apple silicon. For Intel Macs as well, build both and join them:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --bin whistlestop --target aarch64-apple-darwin
cargo build --release --bin whistlestop --target x86_64-apple-darwin
lipo -create -output whistlestop \
  target/aarch64-apple-darwin/release/whistlestop \
  target/x86_64-apple-darwin/release/whistlestop
```

**Linux**: the same command, with the packages in `README.md`.

**CI** (`.github/workflows/build.yml`) builds and tests every push on
Windows, macOS (Apple silicon) and Linux and keeps the programs as
artifacts on the run's page. Downloading those needs a GitHub login and
they expire, so don't send testers there.

**Releases** (`.github/workflows/release-windows.yml`): pushing a `v*` tag
builds `whistlestop.exe` on Windows (statically linked, so no Visual C++
redistributable is needed), zips it with `.github/release/READ-ME-FIRST.txt`,
`README.md`, `LICENSE` and the User Guide as
`whistlestop-<tag>-windows-x64.zip` with a `.sha256`, and publishes a GitHub
release with `.github/release/NOTES.md` as its text. A tag with a `-` in it
(every beta) is marked as a pre-release. There is no macOS release build
yet: build it on the Mac (above) and attach it to the release by hand.

## What a tester downloads

1. **Whistlestop**, from the release at `github.com/NR5P/whistlestop/releases`:
   - Windows: `whistlestop-<tag>-windows-x64.zip`. Unzip it and run
     `whistlestop.exe`; nothing to install. It isn't signed, so SmartScreen
     warns the first time: **More info → Run anyway**. A console window
     opens beside the app (the command-line tools print there); closing it
     quits Whistlestop.
   - macOS (when one is attached): `whistlestop`, a program rather than an
     `.app`, not signed or notarized. In Terminal:
     `xattr -d com.apple.quarantine whistlestop`, `chmod +x whistlestop`,
     then `./whistlestop`. macOS asks for microphone access for Terminal
     (the program that started it): allow it, or Whistlestop hears nothing.
   - The User Guide is inside the program (📖 Guide); the zip carries a copy.
2. **Mercury 1.9.23**, from Rhizomatica's release page (the Guide's
   "Getting started on the air", step 1, says which file).
3. Their settings, the Depot and the Log's file live in
   `%APPDATA%\Whistlestop` (Windows), `~/Library/Application Support/Whistlestop`
   (macOS) or `~/.config/Whistlestop` (Linux); a new beta keeps them.

## Known limitations (tell the testers)

- **File-offer time estimates can be off.** The time shown with an offer is
  a prediction; Mercury's real speed on a link can differ a lot from it
  (Mercury climbs its speed ladder only while there's data to send).
- **Mercury's busy detector can hold a CQ for up to 60 s.** With
  **Listen before transmitting** on, local noise that the detector reads as
  a signal makes a CQ (and a Station ID, automatic calls and Roundtable
  frames) wait, at most 60 s, before it goes anyway.
- **Radios with poor image rejection can fool Search.** A QMX heard a whistle
  on its other sideband only about 8 dB down on the bench. Whistlestop now
  checks for mirror images, but strong nearby stations can still cost a
  search time or a missed CQ.
- **Roundtables with three or more stations are only partly tested.**
  Two-station tables pass the bench; three on the air have not been tried
  in full.
- **Not yet tested between stations on real antennas**, only into dummy
  loads.
- **Mercury 1.9.24** (13 November 2026) hasn't been tested with Whistlestop;
  ask testers to use 1.9.23.

## Still to do before testers get it

- A macOS build for the release (there's no workflow for it yet).
- Decide whether the phone apps are part of the beta (the Android app's USB
  radio connection hasn't been tried with a real radio).
