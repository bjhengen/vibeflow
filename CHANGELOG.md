# Changelog

All notable changes to this project are documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-05-20

### Changed

- **Default palette readability** (slots 4 and 12): the classic xterm blue `#0000ee` (slot 4) is too dark to read against typical dark terminal backgrounds — `ls --color=auto` uses `01;34` for directories, which renders as bold + slot 4 in most shells, producing barely-readable folder names. Slot 4 now defaults to `#6a76fb` (alacritty's value; passes WCAG AA against `#0e0e12`); slot 12 (bright blue) is bumped from `#5c5cff` to `#89b4fa` (Catppuccin Mocha) to keep bright-blue distinctly brighter than blue and clear WCAG AAA. Imported iTerm2 themes override these as before.

### Fixed

- **npm packaging from a clean checkout** (Codex review #1): `bindings/npm/package.json` lacked `prepack` / `prepublishOnly` scripts, so a publish without a pre-existing local `dist/` would ship an empty package. v0.1.0 was healthy by accident (our publish machine had `dist/` from `npm test`); v0.1.1 onward is correct unconditionally.
- **LICENSE files included in published Cargo and npm packages** (Codex review #3): both crates declared `MIT OR Apache-2.0` but the LICENSE text was not in the published artifacts. Now symlinked into each crate dir (Cargo materialises symlinks during packaging) and copied into `bindings/npm/` (npm pack does NOT follow symlinks; uses file copies).
- **README OSC 52 claim removed** (Codex review #5): the README claimed clipboard via OSC 52, but `session/osc.rs` only handles OSC 0/1/2/133/1338. The claim is dropped (arboard support remains). Actual OSC 52 implementation deferred — has real permission/security design work.
- **Clipboard paste sanitised against `ESC[201~` injection** (Codex review #4): bracketed paste mode wraps content with `ESC[200~`/`ESC[201~`. If the clipboard text itself contains `ESC[201~`, a paste could terminate the bracketed-paste frame mid-content and inject the remainder as live user input. Now stripped before the inner bytes reach the PTY.

### Security

- **`appimagetool` SHA256 verification in release CI** (Codex review #2, narrowed): the release workflow now verifies the pinned `appimagetool 1.9.1` against `ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0` before making it executable. The job also emits `vibeflow-x86_64.AppImage.sha256` alongside the AppImage in the GitHub Release for downstream verification.
- **`cargo doc` clean under `-D warnings`**: a broken intra-doc link in `app.rs` had been failing CI's `cargo doc` step on every push since the post-Stage-13 polish merge. Resolved (already on main as commit `387fde2`, before this branch — documented here as part of the v0.1.0→v0.1.1 delta).

### Internal

- **CI packaging-assertion job** (Codex review #10): new job runs `cargo package --list`, `cargo publish --dry-run`, and `npm pack --dry-run` and asserts the expected file set including LICENSE files. Catches future regressions of the #1 / #3 gaps automatically.
- `release.yml` gains `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24` to silence the Node 20 deprecation warning, matching `ci.yml`.

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
[0.1.1]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.1
