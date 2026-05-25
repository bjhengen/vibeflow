# Changelog

All notable changes to this project are documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2] - 2026-05-25

The first feature-bundle release after v0.1. Five sub-features merged via individual PRs (#1–#4 plus the direct-merged logging facility), each with senior pre-execution review of plans vs actual source, subagent-driven per-task implementation with two-stage review, and a manual VNC smoke walk before merge.

### Added

- **CLI `--version` / `-V` flag.** Short-circuits before any winit/wgpu init so the version is queryable headless (over SSH, in Dockerfile builds, in CI). 3 integration tests pin exit code 0, exact stdout `vibeflow {version}\n`, empty stderr, and elapsed-time bound (no GUI fall-through).
- **"About vibeflow" right-click menu item → centred modal overlay panel.** 5 lines (version, gap, tagline, license + repo URL, dismissal hint). 880×240 logical pixels, theme-coloured, dim backdrop. ESC / any keypress / any mouse click closes (and swallows the event so it doesn't leak to the PTY).
- **File logging facility** (Spec A', closes the Stage-9 TODO). Default stderr filter is now `vibeflow=warn` (quiet by default); INFO+ is captured to a daily-rotated file at `$XDG_STATE_HOME/vibeflow/vibeflow.log.YYYY-MM-DD`. `RUST_LOG` env var still overrides at runtime.
- **OSC 52 clipboard WRITE** (Codex review #5 resolved). Full pipeline parser → `DispatchEvent::Osc52Write` → `SessionEvent::Osc52ClipboardWrite` → `Clipboard::copy_clipboard_only` / `copy_primary`. Read is intentionally not implemented (security default — matches xterm/foot/wezterm — see threat model in the OSC 52 design spec).
- **Per-flag terminal attribute rendering.** `BOLD` and `ITALIC` cells route to embedded JetBrains Mono variant fonts via a `font_attrs_for(cell.flags)` helper; `UNDERLINE` / `DOUBLE_UNDERLINE` / `UNDERCURL` / `DOTTED_UNDERLINE` / `DASHED_UNDERLINE` / `STRIKEOUT` render as decoration quads; `INVERSE` / `HIDDEN` / `DIM` mutate per-cell fg/bg before glyph emission. Order of operations within the cell loop is `INVERSE → HIDDEN → DIM → cursor-invert`. The atlas cache key gains `(Weight, Style)` so all four font variants share the same atlas without collision.
- **JetBrains Mono Bold + Italic + BoldItalic v2.304 fonts embedded** alongside the existing Regular. AppImage growth ~+1.8 MB.
- **Glyph atlas hard caps** with full-reset on overflow (4096 px mono, 2048 px color). Adversarial-input safety net — `tracing::warn!` on reset; cache.clear() + shelves.clear() + texture recreate at initial size; next paint re-rasterises naturally.
- **Supply-chain hardening** (Spec B): all `release.yml` and `ci.yml` GitHub Actions are SHA-pinned to specific commits; `actions/attest-build-provenance@v4.1.0` generates an SLSA build provenance attestation alongside the AppImage; SBOM (cyclonedx) is generated and uploaded with the release; new packaging-assertions CI job validates `.crate` + npm-pack contents end-to-end AND enforces Cargo workspace version == `bindings/npm/package.json` version (catches the v0.1.1 npm-version slip pattern).
- **Config-value bounds** (Codex review #8): `bounds::clamp_with_warn` clamps `ai.polling_interval_ms`, `scrollback.history_lines`, etc. with a warn-log on out-of-range values rather than silently accepting them.

### Changed

- **Cursor rendering for empty cells** (the headline Claude Code TUI bug). The cursor cell is iterated by alacritty's `display_iter` (the earlier hypothesis that empty cells are skipped was wrong); the actual bug was an over-aggressive `continue` that suppressed `INVERSE`-flagged "visual cursor" characters that TUI libraries like Ink draw at the cursor position after hiding the terminal cursor via DECTCEM `?25l`. Cell content now renders regardless of terminal-cursor visibility state; the per-cell `INVERSE` handler above correctly inverts Ink's visual cursor; a standalone cursor quad is emitted after the cell loop only when the cursor cell wasn't iterated at all.
- **Paint cadence — idle CPU 78% → 5%** (the headline performance bug). `about_to_wait` previously called `request_redraw()` unconditionally on every wake; winit on X11 (especially over VNC) wakes `about_to_wait` from background X11 events far more often than vsync rate, which drove vibeflow to paint at ~60 FPS continuously even on a bare bash prompt. The new logic gates `request_redraw()` behind an elapsed-time deadline derived from cursor-blink boundary (idle) or pulse interval (`TabState::Waiting`). Most wakes now just re-set `WaitUntil` and return.
- **Typing latency close to xterm.** New `last_activity_at` tracks user keypresses + PTY echo events; while activity is recent (within 500 ms), `WaitUntil` tightens to 4 ms so the PTY echo queue is drained at ~vsync latency, then falls back to the blink boundary for low idle CPU.
- **About-overlay default panel size 560×200 → 880×240.** First-VNC-smoke catch: the spec's 560 px width didn't fit the tagline or license + URL lines at JetBrains Mono's actual cell pitch (`lesson_layout_default_too_small`).
- **Default startup is quieter.** Per the logging change, stderr no longer streams `info!` from winit / wgpu / cosmic-text / vibeflow's own per-event logs; only `warn!`+`error!` reach stderr at the new default filter.
- **Theme registry capped at 50 themes loaded** (Codex review #8, defensive against `~/.config/vibeflow/themes/` directories with many `.toml` files).
- **`--import-colors` rejects files exceeding 256 KB** (Codex review #8, defensive against pathological iTerm2 `.itermcolors` payloads).
- **`MenuAction::OpenRepoUrl` removed.** The grid-menu "About vibeflow" item used to spawn `xdg-open <REPO_URL>`; with the new About overlay showing the URL as visible text, the `xdg-open` path is no longer needed.

### Fixed

- **Decoration quads (UNDERLINE / STRIKEOUT / etc.) render in solid fg color** rather than blending with whatever pixel happened to live at atlas (0, 0). The shader's mono path is `mix(bg, fg, alpha)`; with a zero-size atlas rect, `alpha` is sampled at atlas (0, 0) — an undefined first-glyph pixel. Passing `(fg, fg)` instead of `(fg, bg)` resolves to pure `fg` regardless. Pinned by a regression test in `quad.rs::tests`.

### Internal

- **Final v0.1.2 sub-feature merged via PR #4** (Render). v0.1.2 shipped via four GitHub PRs (#1 OSC 52, #2 supply-chain, #3 About, #4 Render) plus one direct merge (logging). The PR-based workflow was adopted mid-v0.1.2 and is now the standing pattern for non-trivial work (`feedback_pr_workflow`).
- **Subagent-driven-development cumulative scale**: across the 4 PRs, ~40 implementation tasks were dispatched to fresh per-task subagents with two-stage review (spec-compliance then code-quality). Senior pre-execution Sonnet review of each plan vs actual source caught 12+ compile blockers before dispatch.

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
- **Selection** — character, word, line; **block (Alt+drag) column selection**; shift-extend; `arboard` clipboard.
- **Scrollback** — scrollable buffer with `snap_on_esc`; `Shift+PgUp` / `Shift+PgDn` navigation; correct selection mapping across scrollback rows.
- **Keyboard** — xterm-style modifier arrow keys (`Ctrl+`/`Shift+` arrows), Unicode input, configurable bindings.
- **Configuration** — `~/.config/vibeflow/config.toml` with hot-reload; sections for fonts, colors, bell, AI, scrollback, keybindings.
- **Window icon** — the vibeflow logo, embedded at compile time; shown in the launcher/taskbar entry (also in the AppImage).
- **Distribution** — `cargo install vibeflow`, and a single-file `vibeflow-x86_64.AppImage` attached to the GitHub Release.

### Out of scope for v0.1

- Splits/panes; in-buffer search; macOS/Windows builds; image protocols (kitty/sixel); plugin layer; telemetry; Python binding; headless GPU snapshot tests; binary signing/notarization; `.deb`/Homebrew/AUR packaging.

[0.1.0]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.0
[0.1.1]: https://github.com/bjhengen/vibeflow/releases/tag/v0.1.1
