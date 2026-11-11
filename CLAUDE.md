# Notes for Claude

## Definition of done for any new feature

Whenever something is added to Whistlestop, it is not done until:

1. **The User Guide covers it**: `docs/USER-GUIDE.md`, which is built into the app (📖 Guide window). Be thorough: what it does, defaults, limits, the real numbers from the code.
2. **Help mode explains every new control**: add a `Topic` in `src/gui/help.rs` (title, explanation, Guide section) and attach it with `.help(&help::X)` on the widget's response. Help mode is off by default and shows these only on hover.
3. **The layout doesn't shift**: readouts whose text changes (states, numbers) get fixed widths and monospace digits.

## Other rules

- Never open or reuse code from `~/Desktop/varac-rust` or `~/Desktop/OpenSlot`.
- RigHelm (`~/Desktop/righelm`, same egui 0.37) is the reference for rig control and RTL-SDR patterns.
- Test with real Mercury on the two-station bench (`docs/BENCH.md`) before pushing protocol or engine changes.
