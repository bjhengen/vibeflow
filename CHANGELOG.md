# Changelog

All notable changes to this project are documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-05-19

Initial public release of the vibeflow terminal — a from-scratch GPU-accelerated Linux terminal emulator whose flagship feature is **per-tab AI-state awareness** via the OSC 1338 protocol.

### Added

- **Per-tab AI-state indicator** (the flagship). A thin stripe per tab visualises whether the program inside is *working* (blue), *waiting on you* (amber, pulsing), or neutral. State arrives via the OSC 1338 protocol; three-tier resolution: native OSC 1338 (Tier 1), `/proc` heuristic on a configured AI-tools list (Tier 3). Tier 2 wrapper shims are planned post-v0.1.
- **OSC 1338 protocol** — open standard for AI-tool state signalling in terminals. Rust reference implementation as the `vibeflow-protocol` crate (includes the `vibeflow-emit` CLI used by tool hooks); TypeScript binding as the `vibeflow-protocol` npm package.
- **GPU rendering** via wgpu — fast text rendering with truecolor, italics, and color emoji.
- **Tabs** with per-tab session, title, indicator, context menu, drag-to-rename.
- **Themes** — full iTerm2 `.itermcolors` import (`vibeflow --import-colors <path>`); per-tab override; theme registry; hot-reload via `[colors] preset` config key.
- **Configurable bell** — `visual` / `audible` / `both` / `silent`, with debounce.
- **Selection** — character, word, line; **block (Alt+drag) column selection**; shift-extend; OSC 52 and `arboard` clipboard.
- **Scrollback** — scrollable buffer with `snap_on_esc`; `Shift+PgUp` / `Shift+PgDn` navigation; correct selection mapping across scrollback rows.
- **Keyboard** — xterm-style modifier arrow keys (`Ctrl+`/`Shift+` arrows), Unicode input, configurable bindings.
- **Configuration** — `~/.config/vibeflow/config.toml` with hot-reload; sections for fonts, colors, bell, AI, scrollback, keybindings.
- **Window icon** — the vibeflow logo, embedded at compile time; shown in the launcher/taskbar entry (also in the AppImage).
- **Distribution** — `cargo install vibeflow`, and a single-file `vibeflow-x86_64.AppImage` attached to the GitHub Release.

### Out of scope for v0.1

- Splits/panes; in-buffer search; macOS/Windows builds; image protocols (kitty/sixel); plugin layer; telemetry; Python binding; headless GPU snapshot tests; binary signing/notarization; `.deb`/Homebrew/AUR packaging.

[0.1.0]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.0
