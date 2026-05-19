# vibeflow Stage 14 — v0.1 Ship Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make vibeflow installable (crates.io + AppImage), discoverable (README + repo metadata + logo), and tagged `v0.1.0` — a clean public front door for the feature-complete terminal. Personal-FOSS attribution (no LLC).

**Architecture:** Eleven small, mostly-independent tasks landing on a single branch `stage14-ship` off `main`. Code changes are minimal (one tiny module + a one-line window-builder edit + manifest fixes). The bulk is docs (full README rewrite, crate-level README, CHANGELOG), assets (logo PNGs + 256×256 runtime icon embedded inside the crate), and release plumbing (`.desktop` + release CI workflow that builds the AppImage on a `v*` tag push). All external/irreversible actions (`cargo publish`, `npm publish`, push `v0.1.0`, `gh repo edit`) are **human-gated** in a final checklist — never executed autonomously.

**Tech Stack:** Rust workspace (`vibeflow`, `vibeflow-protocol`), winit 0.30, `png` 0.17 (new dep, lighter than `image`), Python 3 + Pillow 12 (one-time icon resize, not a build dep), npm (`vibeflow-protocol` binding), GitHub Actions, `appimagetool` 1.9.1 from `github.com/AppImage/appimagetool` (downloaded in-job, pinned), shields.io dynamic badges.

**Spec reference:** `docs/superpowers/specs/2026-05-19-vibeflow-stage14-ship-design.md` @ `39cc1bd`.

**Branch base:** `main` @ HEAD (`27dee6b` or later — re-check at start). Create branch `stage14-ship`.

---

## ⚠️ PRE-EXECUTION SAFETY GUARDS — applies to every task

These come from the project's hard-won lessons (`lesson_subagent_amend_drift`,
`lesson_review_subagent_destructive`, `feedback_implementer_safety`,
`feedback_senior_review_plans`). Re-state in every dispatched implementer prompt.

1. **Never run `cargo` from outside the workspace root** (`/home/bhengen/dev/vibeflow`). All cargo commands run from there.
2. **Never use `git commit --amend` to fix things** — the failed-amend pattern silently corrupts subsequent diffs. Make a new commit. Controller will run `git status` after every task; any unexpected unstaged drift = stop, do not proceed.
3. **Never delete tests or comment out failures** to make a build green. If a test fails, fix the cause.
4. **Reviewer subagents are READ-ONLY**: no `git checkout`, no `rm`, no `git reset`, no file edits, no shell-state mutation. A reviewer that destructively touches the repo is a critical incident.
5. **No `cargo publish` / `npm publish` / `git push --tags` / `gh repo edit` from inside any task** — those are part of the human-gated finale (§ "Gated finale" at the bottom of this plan), not autonomous work.
6. **No emoji in committed files** unless explicitly requested in the step (per CLAUDE.md user preference).
7. **No backwards-compat shims, no "TODO later", no speculative future-proofing.** YAGNI; remove dead code rather than leaving it for the next stage.

---

## File structure (what each new/modified file is for)

**New files (committed):**

| Path | Purpose |
|---|---|
| `assets/vibeflow_logo_lockup.png` | Repo README header (marketing) |
| `assets/vibeflow_icon_dark_gradient.png` | Marketing icon variant (reference) |
| `assets/vibeflow_icon_light_gradient.png` | Marketing icon variant (reference) |
| `assets/vibeflow_icon_dark_mono.png` | Marketing icon variant (reference) |
| `crates/vibeflow/assets/icon.png` | 256×256 RGBA8 runtime icon. Embedded via `include_bytes!`. MUST live inside the crate (crates.io self-containment). |
| `crates/vibeflow/src/icon.rs` | `load_icon()` → `Option<winit::window::Icon>`. PNG-decode helper with unit tests. |
| `crates/vibeflow/README.md` | Crate-level README (rendered on crates.io). Short — no repo-relative links. |
| `CHANGELOG.md` | Keep-a-Changelog. `v0.1.0` entry = the GitHub Release body source. |
| `packaging/vibeflow.desktop` | XDG desktop entry for the AppImage launcher entry. |
| `.github/workflows/release.yml` | Triggered on `v*` tag push: build release binary, bundle AppImage, cut GitHub Release. |

**Modified files:**

| Path | Change |
|---|---|
| `README.md` | Full rewrite (the current one is stale "pre-alpha"). |
| `Cargo.toml` (workspace) | `authors` → `Brian Hengen <bhengen@gmail.com>`. |
| `crates/vibeflow/Cargo.toml` | Remove `publish = false`; fix `description`; add `readme`/`categories`/`keywords`; add `png` dep. |
| `crates/vibeflow/src/lib.rs` | Add `mod icon;`. |
| `crates/vibeflow/src/window.rs` | Append `.with_window_icon(crate::icon::load_icon())` to the existing builder chain at line ~933. |
| `bindings/npm/package.json` | `"name"`: `@vibeflow/protocol` → `vibeflow-protocol`; `"author"` email → `bhengen@gmail.com`. |
| `bindings/npm/README.md` | Three `@vibeflow/protocol` → `vibeflow-protocol` rewrites. |

**Deletions (working-tree only — file is currently untracked):**

| Path | Why |
|---|---|
| `vibeflow_logo_exports.zip` | Source zip; committed PNGs are the source of truth. |

**NOT touched** (verify, don't edit unless something is genuinely missing):

- `crates/vibeflow-protocol/Cargo.toml` — already publish-ready (`description`, `categories`, `keywords`, `readme`).
- `crates/vibeflow-protocol/README.md` — Rust-only references, no `@vibeflow/protocol` mention.
- `.github/workflows/ci.yml` — existing CI unchanged.
- `LICENSE-MIT` / `LICENSE-APACHE` — already correct.

---

## Branch setup

- [ ] **Step 0.1: Branch off main**

```bash
cd /home/bhengen/dev/vibeflow
git checkout main
git pull --ff-only origin main
git status --short  # expect: only the untracked .claude/ and vibeflow_logo_exports.zip
git checkout -b stage14-ship
```

Expected: on `stage14-ship`, branched from current `main` HEAD.

---

## Task 1: Extract logo assets, generate runtime icon, drop the zip

**Files:**
- Create: `assets/vibeflow_logo_lockup.png`, `assets/vibeflow_icon_dark_gradient.png`, `assets/vibeflow_icon_light_gradient.png`, `assets/vibeflow_icon_dark_mono.png`
- Create: `crates/vibeflow/assets/icon.png` (256×256 RGBA8)
- Delete (working tree only): `vibeflow_logo_exports.zip`

- [ ] **Step 1.1: Make asset directories**

```bash
mkdir -p assets
mkdir -p crates/vibeflow/assets
```

Expected: both directories exist; neither contained pre-existing files (verify with `ls`).

- [ ] **Step 1.2: Extract the four marketing PNGs**

```bash
unzip -o vibeflow_logo_exports.zip -d assets/
ls -1 assets/
```

Expected output (4 files):
```
vibeflow_icon_dark_gradient.png
vibeflow_icon_dark_mono.png
vibeflow_icon_light_gradient.png
vibeflow_logo_lockup.png
```

- [ ] **Step 1.3: Generate the 256×256 runtime icon (Pillow, LANCZOS)**

```bash
python3 - <<'PY'
from PIL import Image
src = Image.open("assets/vibeflow_icon_dark_gradient.png").convert("RGBA")
assert src.size == (1024, 1024), f"unexpected source size: {src.size}"
dst = src.resize((256, 256), Image.LANCZOS)
dst.save("crates/vibeflow/assets/icon.png", format="PNG", optimize=True)
PY
file crates/vibeflow/assets/icon.png
```

Expected `file` output:
```
crates/vibeflow/assets/icon.png: PNG image data, 256 x 256, 8-bit/color RGBA, non-interlaced
```

- [ ] **Step 1.4: Delete the source zip from the working tree**

```bash
rm vibeflow_logo_exports.zip
git status --short
```

Expected: zip no longer listed as untracked. New untracked: `assets/`, `crates/vibeflow/assets/`.

- [ ] **Step 1.5: Commit**

```bash
git add assets/ crates/vibeflow/assets/
git status --short  # confirm both dirs staged, nothing else
git commit -m "feat(stage14): logo assets + 256x256 runtime icon

- assets/: 4 marketing PNGs (lockup + 3 icon variants) for README
- crates/vibeflow/assets/icon.png: 256x256 RGBA8 runtime icon
  (downsampled from 1024x1024 dark-gradient source via PIL LANCZOS).
  Lives inside the crate so the crates.io-published package is
  self-contained.
- Source zip deleted from working tree (committed PNGs are the source of truth)."
```

Expected: commit succeeds; `git status` clean except `.claude/`.

---

## Task 2: `png` dep + `icon.rs` module (TDD)

**Files:**
- Modify: `crates/vibeflow/Cargo.toml` (add `png = "0.17"`)
- Create: `crates/vibeflow/src/icon.rs`
- Modify: `crates/vibeflow/src/lib.rs:1-16` (add `mod icon;`)

- [ ] **Step 2.1: Write the failing tests (icon.rs)**

Create `crates/vibeflow/src/icon.rs`:

```rust
//! Embedded window icon for the vibeflow terminal.
//!
//! The 256x256 RGBA8 PNG at `crates/vibeflow/assets/icon.png` is embedded
//! at compile time via `include_bytes!` so the published crate is
//! self-contained. Decode failure is non-fatal — `load_icon()` returns
//! `None` and the window is created without an icon (logged at WARN).

use winit::window::Icon;

const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Decode the embedded PNG into a winit `Icon`. Returns `None` on any
/// decode/`Icon::from_rgba` failure; failure is non-fatal at startup.
pub fn load_icon() -> Option<Icon> {
    decode_to_icon(ICON_PNG)
}

/// Internal helper, separated so tests can exercise the failure path
/// against arbitrary input without modifying the embedded asset.
fn decode_to_icon(bytes: &[u8]) -> Option<Icon> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    // Truncate to the actual frame bytes (next_frame may write less than capacity).
    buf.truncate(info.buffer_size());
    // Require RGBA8 — what our committed icon.png is encoded as. Anything
    // else means the asset was regenerated incorrectly; surface as None.
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    Icon::from_rgba(buf, info.width, info.height).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_decodes_to_256x256_rgba() {
        // load_icon() returns Some for the committed crates/vibeflow/assets/icon.png.
        // We verify Some-ness here; the dimensions/format are pinned by the
        // separate decode_to_icon round-trip test below using the same bytes.
        assert!(load_icon().is_some(), "embedded icon failed to decode");
    }

    #[test]
    fn decode_round_trip_reports_256x256_rgba8() {
        // Decode the embedded bytes directly and verify the PNG header reports
        // the expected geometry, independent of winit's Icon validation.
        let decoder = png::Decoder::new(ICON_PNG);
        let reader = decoder.read_info().expect("png header");
        let info = reader.info();
        assert_eq!(info.width, 256);
        assert_eq!(info.height, 256);
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
    }

    #[test]
    fn bad_bytes_returns_none() {
        assert!(decode_to_icon(b"not-a-png-at-all").is_none());
    }

    #[test]
    fn empty_bytes_returns_none() {
        assert!(decode_to_icon(b"").is_none());
    }
}
```

- [ ] **Step 2.2: Declare the module in `lib.rs`**

Edit `crates/vibeflow/src/lib.rs` and append `mod icon;` at the end of the file (after the current last line `pub mod window;` at line 16):

```rust
pub mod app;
pub mod clipboard;
pub mod config;
pub mod keymap;
pub mod render;
pub mod session;
pub mod theme;
pub mod window;

// Private — used by window.rs to attach the embedded PNG to the winit window.
mod icon;
```

- [ ] **Step 2.3: Run tests — verify they FAIL with no `png` dep yet**

```bash
cargo test -p vibeflow --lib icon 2>&1 | tail -15
```

Expected: compile error — `unresolved import \`png\`` (or `use of undeclared crate or module \`png\``). This is the "test fails first" state.

- [ ] **Step 2.4: Add `png` dependency**

Edit `crates/vibeflow/Cargo.toml`, in the existing `[dependencies]` block, add **alphabetically** (before `pollster`):

```toml
png = "0.17"
```

- [ ] **Step 2.5: Run tests — verify they PASS**

```bash
cargo test -p vibeflow --lib icon 2>&1 | tail -15
```

Expected output ends with:
```
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; ...
```

- [ ] **Step 2.6: Run the full vibeflow lib test suite (no regressions)**

```bash
cargo test -p vibeflow --lib 2>&1 | tail -5
```

Expected: all tests pass (previous count + 4 new icon tests).

- [ ] **Step 2.7: Clippy on the new code**

```bash
cargo clippy -p vibeflow --lib --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: `Finished` with no warnings. If warnings appear, address them (do NOT `#[allow(...)]` to silence).

- [ ] **Step 2.8: Commit**

```bash
git add crates/vibeflow/Cargo.toml crates/vibeflow/src/icon.rs crates/vibeflow/src/lib.rs
git status --short  # confirm exactly these three paths
git commit -m "feat(stage14): icon.rs — embed 256x256 PNG, decode to winit Icon

- New private module crates/vibeflow/src/icon.rs with
  load_icon() -> Option<winit::window::Icon>.
- Embeds crates/vibeflow/assets/icon.png at compile time via
  include_bytes!; decode failure returns None (non-fatal at startup).
- Adds png = \"0.17\" dependency (lighter than image; no extra features).
- 4 unit tests cover the happy path, dimension/format pinning, and
  the bad-bytes / empty-bytes failure paths."
```

---

## Task 3: Wire the icon to the winit window

**Files:**
- Modify: `crates/vibeflow/src/window.rs:933-935`

- [ ] **Step 3.1: Append `.with_window_icon(...)` to the builder chain**

In `crates/vibeflow/src/window.rs`, the existing `resumed()` builds `window_attrs` at line 933:

```rust
let window_attrs = Window::default_attributes()
    .with_title("vibeflow")
    .with_inner_size(winit::dpi::LogicalSize::new(960, 600));
```

Change to (one line appended):

```rust
let window_attrs = Window::default_attributes()
    .with_title("vibeflow")
    .with_inner_size(winit::dpi::LogicalSize::new(960, 600))
    .with_window_icon(crate::icon::load_icon());
```

- [ ] **Step 3.2: `cargo check`**

```bash
cargo check -p vibeflow 2>&1 | tail -5
```

Expected: `Finished` with no errors. `with_window_icon` on winit 0.30 `WindowAttributes` takes `Option<Icon>`, which matches `load_icon()`'s return type exactly — no wrapping needed.

- [ ] **Step 3.3: Full clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: `Finished` clean.

- [ ] **Step 3.4: Full test suite (no regression)**

```bash
cargo test --workspace 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 3.5: Commit**

```bash
git add crates/vibeflow/src/window.rs
git status --short
git commit -m "feat(stage14): wire embedded icon to the winit window

WindowAttributes builder gains .with_window_icon(crate::icon::load_icon()).
load_icon() returns Option<Icon>, exactly the type winit expects; decode
failure yields None and the window opens with the default OS icon (logged
warning would already be emitted by load_icon's path if desired — current
helper is silent on None which is acceptable for a non-fatal cosmetic
fallback). Visual verification happens during the VNC smoke walk."
```

> **Note for the implementer:** the in-product visual check (taskbar/launcher entry shows the logo) is part of the manual VNC smoke walk after merge, not this task. No unit test asserts that winit actually applies the icon — that's a UI behavior outside vibeflow's testable surface.

---

## Task 4: Manifest fixes — author + crates.io publish readiness

**Files:**
- Modify: `Cargo.toml` (workspace, line 12)
- Modify: `crates/vibeflow/Cargo.toml`

- [ ] **Step 4.1: Update workspace `authors`**

Edit `Cargo.toml` line 12 (workspace `[workspace.package]`):

```toml
authors = ["Brian Hengen <bhengen@gmail.com>"]
```

(Previous value: `"Brian Hengen <brian@friendly-robots.com>"` — the only LLC-domain leakage in the workspace per the spec amendment.)

- [ ] **Step 4.2: Update `crates/vibeflow/Cargo.toml`**

Replace the existing `[package]` block top section so it reads:

```toml
[package]
name = "vibeflow"
description = "GPU-accelerated Linux terminal emulator that knows when your AI tool is waiting on you."
categories = ["command-line-utilities"]
keywords = ["terminal", "ai", "gpu", "wgpu", "vibeflow"]
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
authors.workspace = true
rust-version.workspace = true
```

Changes vs current:
- **Removed** `publish = false` (line 4 in the current file).
- **Rewrote** `description` (was: `"GPU-accelerated terminal emulator for Linux that knows when AI tools are waiting on the user (library crate, Stage 2 of v0.1)"`).
- **Added** `categories = ["command-line-utilities"]` (single category, valid per crates.io's list).
- **Added** `keywords = ["terminal", "ai", "gpu", "wgpu", "vibeflow"]` (5 max, each ≤20 chars — all valid).
- **NOT adding `readme = "README.md"` here** — that lands in Task 5 alongside creating the crate README, so this task's dry-run isn't blocked by a missing file.

Leave the rest of the file (`[lints]`, `[lib]`, `[[bin]]`, `[dependencies]`, `[dev-dependencies]`) **unchanged** — the `png` line from Task 2 stays.

- [ ] **Step 4.3: Build sanity check**

```bash
cargo build --workspace 2>&1 | tail -3
```

Expected: `Finished` clean.

- [ ] **Step 4.4: Dry-run publish for vibeflow-protocol**

```bash
cargo publish --dry-run -p vibeflow-protocol 2>&1 | tail -10
```

Expected: ends with something like:
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in ...s
Packaging vibeflow-protocol v0.1.0 (...)
Packaged ... files, ... (...)
Uploading vibeflow-protocol v0.1.0 (...)
warning: aborting upload due to dry run
```

(The "Uploading" + "aborting" pair is the standard dry-run success signature. No errors above it.)

- [ ] **Step 4.5: Dry-run publish for vibeflow**

```bash
cargo publish --dry-run -p vibeflow 2>&1 | tail -15
```

Expected: same `Packaging vibeflow v0.1.0` … `aborting upload due to dry run` shape, with no errors. (At this point cargo may warn that no `readme` field is set — that's expected and addressed in Task 5; it must NOT be a hard error.)

If cargo produces an actual error (not a warning) about missing files / metadata, investigate before proceeding. Common gotcha: a `[lib]` + `[[bin]]` crate publishes fine; `publish = false` lingering somewhere would block.

- [ ] **Step 4.6: Commit**

```bash
git add Cargo.toml crates/vibeflow/Cargo.toml
git status --short
git commit -m "feat(stage14): crates.io publish readiness for vibeflow crate

- Workspace authors -> Brian Hengen <bhengen@gmail.com> (personal FOSS
  identity per spec §4 amendment; no LLC domain in published metadata).
- crates/vibeflow: remove publish=false; replace stale description
  (library-crate-stage-2 text -> the real v0.1 tagline); add categories
  and keywords (5 max, all ≤20 chars).
- 'readme' field deferred to Task 5 (created with the crate README).
- cargo publish --dry-run -p vibeflow-protocol and -p vibeflow both
  succeed locally."
```

---

## Task 5: Crate-level README (`crates/vibeflow/README.md`)

**Files:**
- Create: `crates/vibeflow/README.md`
- Modify: `crates/vibeflow/Cargo.toml` (add `readme = "README.md"`)

- [ ] **Step 5.1: Create the crate README**

The crate README is what renders on the crates.io package page. It must NOT use repo-relative paths (`./docs/...`, `./assets/...`) because crates.io has no notion of the parent repo — relative links 404. Keep it short and link to the GitHub repo for the full doc.

Create `crates/vibeflow/README.md`:

````markdown
# vibeflow

> A GPU-accelerated Linux terminal emulator that knows when your AI tool is waiting on you.

## Install

```sh
cargo install vibeflow
```

Or download a single-file AppImage from the [GitHub Releases](https://github.com/bjhengen/vibeflow/releases).

## Quick start

Launch it: `vibeflow`. Open a new tab with `Ctrl+Shift+T`. The per-tab indicator stripe shows the AI-state for that tab:

- **Blue** — AI tool is working
- **Amber (pulsing)** — AI tool is waiting on you
- **Neutral** — plain shell / no AI state

State arrives via the **OSC 1338 protocol** (an open standard for AI-tool state signalling) — see the [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol) crate. AI tools emit one byte sequence per state transition; vibeflow renders the cue.

### Make AI-state work with Claude Code

Add these hooks to `~/.claude/settings.json` so Claude Code reports its state to vibeflow:

```json
{
  "hooks": {
    "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "PreToolUse":       [{"matcher": "*", "hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "PostToolUse":      [{"matcher": "*", "hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "Stop":             [{"hooks": [{"type": "command", "command": "vibeflow-emit waiting --tool=claude"}]}],
    "Notification":     [{"hooks": [{"type": "command", "command": "vibeflow-emit waiting --tool=claude"}]}]
  }
}
```

`vibeflow-emit` ships with the `vibeflow-protocol` crate (`cargo install vibeflow-protocol`).

## License

Dual-licensed under either of MIT or Apache-2.0 at your option. See the [GitHub repository](https://github.com/bjhengen/vibeflow) for the full README, configuration reference, themes, and protocol documentation.
````

- [ ] **Step 5.2: Add `readme` to the crate manifest**

Edit `crates/vibeflow/Cargo.toml` `[package]` block — add `readme = "README.md"` directly under `keywords`:

```toml
keywords = ["terminal", "ai", "gpu", "wgpu", "vibeflow"]
readme = "README.md"
```

- [ ] **Step 5.3: Dry-run again — vibeflow now packages its README**

```bash
cargo publish --dry-run -p vibeflow 2>&1 | tail -10
```

Expected: success; no warning about a missing readme; the package listing in the output includes `README.md`.

- [ ] **Step 5.4: Commit**

```bash
git add crates/vibeflow/README.md crates/vibeflow/Cargo.toml
git status --short
git commit -m "docs(stage14): crate-level README for crates.io rendering

Short README, no repo-relative links (crates.io has no repo context).
Tagline, install (cargo + AppImage), per-tab indicator cheat sheet,
the verbatim 5-hook Claude Code settings.json snippet, and a pointer
to the GitHub repo for the full doc. readme = \"README.md\" added to
crates/vibeflow/Cargo.toml so cargo publish picks it up."
```

---

## Task 6: Full repo `README.md` rewrite

**Files:**
- Modify: `README.md` (full replacement)

- [ ] **Step 6.1: Replace `README.md` with the v0.1 front door**

Replace `/home/bhengen/dev/vibeflow/README.md` with the following content **verbatim** (the 5-hook snippet and the OSC 133 caveat are normative and must not be paraphrased):

````markdown
<p align="center">
  <img src="assets/vibeflow_logo_lockup.png" alt="vibeflow" width="520">
</p>

<p align="center"><em>A GPU-accelerated Linux terminal that knows when your AI tool is waiting on you.</em></p>

<p align="center">
  <a href="https://github.com/bjhengen/vibeflow/actions/workflows/ci.yml"><img src="https://github.com/bjhengen/vibeflow/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/vibeflow"><img src="https://img.shields.io/crates/v/vibeflow.svg?label=crates.io%3Avibeflow" alt="crates.io: vibeflow"></a>
  <a href="https://www.npmjs.com/package/vibeflow-protocol"><img src="https://img.shields.io/npm/v/vibeflow-protocol.svg?label=npm%3Avibeflow-protocol" alt="npm: vibeflow-protocol"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="license: MIT OR Apache-2.0"></a>
</p>

## The idea

A modern terminal renders glyphs faster, but it doesn't know what's happening inside it. vibeflow does: every tab carries a small indicator stripe showing whether the program inside is **working**, **waiting on you**, or **idle**. That awareness becomes especially valuable with AI assistants — you can glance at the tab bar and tell at a distance which conversations need you back, without alt-tabbing through them.

State arrives via the **OSC 1338 protocol**, an open standard for AI-tool state signalling in terminals — published in this repo as a [Rust crate](https://crates.io/crates/vibeflow-protocol) and a [TypeScript binding](https://www.npmjs.com/package/vibeflow-protocol).

## How vibeflow learns the state

Three tiers, in priority order:

1. **Tier 1 — Native OSC 1338.** The AI tool emits a one-shot escape sequence on each state transition (`working`, `waiting`, `idle`). Used today for Claude Code via shell-hook configuration (see Quick start).
2. **Tier 2 — Wrapper shims.** Drop-in launchers (e.g. `vibeflow-claude`) that watch the tool's output and emit OSC 1338 on its behalf — for tools that don't natively support it. *(Planned, post-v0.1.)*
3. **Tier 3 — `/proc` heuristic.** As a fallback for tools in the configured AI list, vibeflow infers state from foreground-process activity and output silence. Lower-fidelity, but works for unknown tools.

## Features (v0.1)

- GPU rendering (wgpu) — fast, smooth scrollback.
- Multiple tabs with per-tab AI-state indicator + title.
- Full iTerm2 color-scheme import (`vibeflow --import-colors <path>`).
- Truecolor, italics, color emoji.
- Configurable bell (`visual` / `audible` / `both` / `silent`, debounced).
- Block (Alt+drag) and shift-extend selection; clipboard via OSC 52 and arboard.
- xterm-style modifier arrow keys.
- Hot-reload config (`~/.config/vibeflow/config.toml`).
- OSC 1338 native AI-state + `/proc` heuristic fallback.

## Install

### crates.io (recommended)

```sh
cargo install vibeflow
```

You'll also want the protocol crate for the `vibeflow-emit` CLI used by AI-tool hooks:

```sh
cargo install vibeflow-protocol
```

### AppImage (single file, no toolchain)

Download `vibeflow-x86_64.AppImage` from the [latest release](https://github.com/bjhengen/vibeflow/releases/latest):

```sh
chmod +x vibeflow-x86_64.AppImage
./vibeflow-x86_64.AppImage
```

Built on `ubuntu-latest` (x86_64). On older distros, prefer `cargo install` or build from source.

### From source

```sh
git clone https://github.com/bjhengen/vibeflow.git
cd vibeflow
cargo build --release
./target/release/vibeflow
```

## Quick start

Launch `vibeflow`. New tab: `Ctrl+Shift+T`. Cycle tabs: `Ctrl+Tab` / `Ctrl+Shift+Tab`.

### Make AI-state work with Claude Code

Add these hooks to `~/.claude/settings.json` so Claude Code reports its state to vibeflow (the **five-hook** set — fewer than five and you'll see spurious flickering during multi-tool-call turns; this is a Claude Code semantic, not a vibeflow bug):

```json
{
  "hooks": {
    "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "PreToolUse":       [{"matcher": "*", "hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "PostToolUse":      [{"matcher": "*", "hooks": [{"type": "command", "command": "vibeflow-emit working --tool=claude"}]}],
    "Stop":             [{"hooks": [{"type": "command", "command": "vibeflow-emit waiting --tool=claude"}]}],
    "Notification":     [{"hooks": [{"type": "command", "command": "vibeflow-emit waiting --tool=claude"}]}]
  }
}
```

For other tools (Codex, opencode, aider, …) — the same shape if the tool exposes equivalent hooks. Until Tier-2 wrapper shims land, the Tier-3 `/proc` heuristic is the fallback (lower-fidelity).

### Plain-shell caveat

vibeflow is a faithful renderer of the signals it receives. A **bare shell** (no OSC 133 prompt-marker integration, no AI tool) emits no state signal — so a tab that was previously in **waiting** state (amber) can legitimately persist amber until the next AI-tool turn or until OSC 133 prompt markers are enabled in your `PS1`. This is the intended *"needs you, still unacknowledged"* semantics, not a stuck state. Bash users wanting prompt-driven recovery in plain shells should enable OSC 133 in their prompt; a typical addition:

```sh
# In ~/.bashrc (after any other PS1 customisation)
PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
PROMPT_COMMAND='printf "\e]133;D\a"'"${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
```

## Configuration

Config lives at `~/.config/vibeflow/config.toml` and hot-reloads on save. Themes are imported as iTerm2 `.itermcolors` files:

```sh
vibeflow --import-colors path/to/scheme.itermcolors [--overwrite]
```

Themes land in `~/.config/vibeflow/themes/` and can be selected via the tab context menu or the `[colors] preset` config key.

Keybindings (default; configurable):

| Action | Shortcut |
|---|---|
| New tab | `Ctrl+Shift+T` |
| Close tab | `Ctrl+Shift+W` |
| Next tab | `Ctrl+Tab` |
| Previous tab | `Ctrl+Shift+Tab` |
| Copy | `Ctrl+Shift+C` |
| Paste | `Ctrl+Shift+V` |
| Scrollback up/down | `Shift+PgUp` / `Shift+PgDn` |

## Protocol

The OSC 1338 wire format is specified in [`docs/protocol.md`](docs/protocol.md). Reference implementations:

- Rust: [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol) (includes the `vibeflow-emit` CLI).
- TypeScript: [`vibeflow-protocol`](https://www.npmjs.com/package/vibeflow-protocol) on npm.

## Contributing / testing

See [`docs/TESTING.md`](docs/TESTING.md) for the local test matrix. CI runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace --all-targets`, `cargo doc`, a 60-second protocol fuzz, and the npm binding build/test.

## License

Dual-licensed under either of:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

---

*vibeflow is a personal open-source project by [Brian Hengen](https://github.com/bjhengen). Views and projects are my own, not affiliated with or endorsed by any employer.*
````

- [ ] **Step 6.2: Verify all relative links resolve**

```bash
for p in assets/vibeflow_logo_lockup.png LICENSE-APACHE LICENSE-MIT docs/protocol.md docs/TESTING.md; do
  test -f "$p" && echo "OK $p" || echo "MISSING $p"
done
```

Expected: all five `OK`. (If any `MISSING`, fix the link or restore the target before committing.)

- [ ] **Step 6.3: Markdown lint sanity (no external tool — just structural check)**

```bash
# Basic structure: must have an H1-equivalent (the centered img+tagline counts visually but no literal H1 — OK for this layout);
# verify the 5-hook JSON block parses as JSON.
sed -n '/^```json$/,/^```$/p' README.md | sed '1d;$d' | python3 -c "import sys, json; json.load(sys.stdin); print('5-hook JSON: valid')"
```

Expected:
```
5-hook JSON: valid
```

- [ ] **Step 6.4: Commit**

```bash
git add README.md
git status --short
git commit -m "docs(stage14): full README rewrite for v0.1 ship

Replaces the stale 'pre-alpha, terminal not yet built' README with the
v0.1 front door: logo lockup, badges (shields.io dynamic), the AI-state
thesis, 3-tier model, features, install (crates.io / AppImage / source),
quick start, the verbatim 5-hook Claude Code settings.json snippet, the
plain-shell OSC 133 caveat (per lesson_spec_promise_vs_shell_integration),
config + keybindings, protocol pointer, contributing, dual license, and
the personal-views-not-employer disclaimer per project_identity_positioning."
```

---

## Task 7: npm package rename to unscoped + author email

**Files:**
- Modify: `bindings/npm/package.json` (name + author)
- Modify: `bindings/npm/package-lock.json` (regenerated, not hand-edited)
- Modify: `bindings/npm/README.md` (3 references)
- Modify: `bindings/npm/src/index.ts` (1 JSDoc reference at line 2)

- [ ] **Step 7.1: Rename and re-author `package.json`**

Edit `bindings/npm/package.json` — change exactly two lines:

```json
"name": "vibeflow-protocol",
```
(was `"name": "@vibeflow/protocol"`.)

```json
"author": "Brian Hengen <bhengen@gmail.com>",
```
(was `"author": "Brian Hengen <brian@friendly-robots.com>"`.)

Leave **everything else unchanged** — version, files, scripts, deps, keywords, etc.

> **Why no `"publishConfig": {"access": "public"}`?** That field is only needed for scoped packages (whose default is `restricted`). Unscoped packages default to public, so the field is redundant. Keep the manifest minimal.

- [ ] **Step 7.2: Update `bindings/npm/README.md`**

Replace the three `@vibeflow/protocol` references with `vibeflow-protocol`:

- Line 1: `# @vibeflow/protocol` → `# vibeflow-protocol`
- Line 10: `npm install @vibeflow/protocol` → `npm install vibeflow-protocol`
- Line 16: `import { emitState } from "@vibeflow/protocol";` → `import { emitState } from "vibeflow-protocol";`

Use three explicit string replaces (do NOT bulk-edit elsewhere — there should be exactly three matches in that file; verify with `grep`).

- [ ] **Step 7.3: Update `bindings/npm/src/index.ts` JSDoc**

Line 2 of `bindings/npm/src/index.ts` is a JSDoc comment that still names the old scope:

```ts
/**
 * @vibeflow/protocol — OSC 1338 protocol binding for TypeScript.
```

Change line 2 to:

```ts
 * vibeflow-protocol — OSC 1338 protocol binding for TypeScript.
```

This is a documentation comment, not an import string, but `tsc` propagates it into the generated `dist/src/index.d.ts` so consumers see it. Keeping it consistent with the package name is correct.

- [ ] **Step 7.4: Regenerate `package-lock.json` to match the renamed package**

The current `package-lock.json` has `"name": "@vibeflow/protocol"` at lines 2 and 8 (the project itself + its self-reference in `packages.""`). Hand-editing a lock file is error-prone; npm regenerates it cleanly from the now-renamed `package.json`:

```bash
cd bindings/npm
npm install --package-lock-only
cd -
```

`--package-lock-only` updates `package-lock.json` without re-downloading dependencies (the project has no runtime deps; `typescript` and `@types/node` are dev-only and already installed in `node_modules/`). Verify:

```bash
grep -n '"name"' bindings/npm/package-lock.json | head -5
```

Expected: lines 2 and 8 (or thereabouts) both show `"name": "vibeflow-protocol"`.

- [ ] **Step 7.5: Verify the rename is complete across the binding**

```bash
grep -rn '@vibeflow/protocol' bindings/npm/ --include='*.json' --include='*.md' --include='*.ts' || echo "OK: no @vibeflow/protocol references in tracked sources"
```

Expected: prints `OK: no @vibeflow/protocol references in tracked sources`. (Globs are scoped to tracked source types — `node_modules/` and `dist/` may contain transitive matches in package caches that we don't ship; the grep flags exclude them implicitly by file type.)

- [ ] **Step 7.6: Rebuild + run the existing npm tests**

The JSDoc edit changes a string that flows through `tsc` into `dist/`; rebuild and re-test:

```bash
cd bindings/npm
npm run build 2>&1 | tail -3
npm test 2>&1 | tail -10
cd -
```

Expected: `npm run build` finishes silently (tsc is quiet on success); `npm test` reports pass (`# pass <N>`).

- [ ] **Step 7.7: npm publish dry-run**

```bash
cd bindings/npm
npm publish --dry-run 2>&1 | tail -25
cd -
```

Expected: lists exactly the files in `"files"` plus `package.json` + `README.md` (~5–15 entries); package name reported as `vibeflow-protocol@0.1.0`; no errors.

- [ ] **Step 7.8: Commit**

```bash
git add bindings/npm/package.json bindings/npm/package-lock.json bindings/npm/README.md bindings/npm/src/index.ts
git status --short
git commit -m "feat(stage14): rename npm package to unscoped vibeflow-protocol

The @vibeflow scope is permanently locked to a pre-existing npm user
'vibeflow' (verified: registry.npmjs.org/-/org/vibeflow returns
ResourceNotFound; ~vibeflow page returns 403). The unscoped name
vibeflow-protocol is free on npm and mirrors the Rust crate name —
single canonical name on both registries.

- package.json: name @vibeflow/protocol -> vibeflow-protocol; author
  email -> bhengen@gmail.com (matching git author + crate workspace).
- package-lock.json: regenerated via 'npm install --package-lock-only'
  so the lockfile name matches package.json (was stale at the old name
  on lines 2 + 8).
- README.md: three @vibeflow/protocol references rewritten.
- src/index.ts: JSDoc line 2 reference rewritten (it flows through tsc
  into the generated .d.ts that consumers see).
- No publishConfig needed (unscoped public is the default).
- npm test green; npm publish --dry-run green."
```

---

## Task 8: `CHANGELOG.md` (Keep a Changelog, v0.1.0 entry)

**Files:**
- Create: `CHANGELOG.md`

- [ ] **Step 8.1: Create `CHANGELOG.md`**

Content:

````markdown
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
````

- [ ] **Step 8.2: Commit**

```bash
git add CHANGELOG.md
git status --short
git commit -m "docs(stage14): CHANGELOG.md with v0.1.0 entry

Keep-a-Changelog format. Single v0.1.0 entry summarises the 14 stages
as user-facing bullets (AI-state, OSC 1338, GPU render, tabs, themes,
bell, selection, scrollback, keyboard, config, icon, distribution).
This is the source the GitHub Release body draws from."
```

---

## Task 9: AppImage desktop entry (`packaging/vibeflow.desktop`)

**Files:**
- Create: `packaging/vibeflow.desktop`

- [ ] **Step 9.1: Create the directory and `.desktop` file**

```bash
mkdir -p packaging
```

Create `packaging/vibeflow.desktop`:

```desktop
[Desktop Entry]
Type=Application
Name=vibeflow
GenericName=Terminal Emulator
Comment=GPU-accelerated Linux terminal that knows when your AI tool is waiting on you
Exec=vibeflow
Icon=vibeflow
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;AI;vibeflow;
StartupNotify=true
```

Field rationale:
- `Exec=vibeflow` — name is resolved relative to the AppDir at AppImage runtime.
- `Icon=vibeflow` — name without extension; matches the staged icon file `vibeflow.png` in the AppDir.
- `Terminal=false` — vibeflow **is** the terminal; we don't want a parent terminal launching it.
- `Categories` — XDG-conforming `System;TerminalEmulator;` (each menu maps these).

- [ ] **Step 9.2: Validate with `desktop-file-validate` if available, else syntax-check**

```bash
which desktop-file-validate >/dev/null 2>&1 \
  && desktop-file-validate packaging/vibeflow.desktop \
  || (grep -E '^[A-Za-z]+=' packaging/vibeflow.desktop | wc -l)
```

Expected: if `desktop-file-validate` is installed, exits 0 with no errors; otherwise a fallback count of `9` key=value lines confirms structure.

- [ ] **Step 9.3: Commit**

```bash
git add packaging/vibeflow.desktop
git status --short
git commit -m "feat(stage14): packaging/vibeflow.desktop for the AppImage

XDG-conforming desktop entry. Icon=vibeflow (the staged icon name
inside the AppDir), Terminal=false (we ARE the terminal), and the
System;TerminalEmulator; categories so the launcher places vibeflow
in the right menu."
```

---

## Task 10: Release CI workflow (`.github/workflows/release.yml`)

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 10.1: Create the workflow file**

Create `.github/workflows/release.yml`:

```yaml
name: Release

# Runs on annotated v* tag push. Build the release binary, bundle it
# into an AppImage with the vibeflow icon + .desktop entry, then create
# the GitHub Release and attach the AppImage. Triggered ONLY by tags;
# does not run on branch pushes, so the manual gated finale step
# (`git push origin v0.1.0`) is the explicit handoff.
on:
  push:
    tags:
      - 'v*'

# appimagetool version is pinned so a release never breaks from an upstream
# change between v0.1.0 and v0.1.1. Sourced from github.com/AppImage/appimagetool
# (the active repo); the older github.com/AppImage/AppImageKit is deprecated and
# its release-13 asset URL 404s as of 2026-05.
env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"
  APPIMAGETOOL_VERSION: "1.9.1"

jobs:
  appimage:
    name: Build AppImage (x86_64)
    runs-on: ubuntu-latest
    permissions:
      contents: write   # needed for softprops/action-gh-release to create the Release
    steps:
      - uses: actions/checkout@v4

      - name: Install stable toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Cargo cache
        uses: Swatinem/rust-cache@v2

      - name: Build release binary
        run: cargo build --release -p vibeflow

      - name: Stage AppDir
        run: |
          set -euxo pipefail
          APPDIR="${GITHUB_WORKSPACE}/AppDir"
          mkdir -p "${APPDIR}/usr/bin" \
                   "${APPDIR}/usr/share/applications" \
                   "${APPDIR}/usr/share/icons/hicolor/256x256/apps"
          cp target/release/vibeflow "${APPDIR}/usr/bin/vibeflow"
          cp packaging/vibeflow.desktop "${APPDIR}/usr/share/applications/vibeflow.desktop"
          cp crates/vibeflow/assets/icon.png "${APPDIR}/usr/share/icons/hicolor/256x256/apps/vibeflow.png"
          # appimagetool expects a top-level .desktop, icon, .DirIcon, and AppRun.
          cp packaging/vibeflow.desktop "${APPDIR}/vibeflow.desktop"
          cp crates/vibeflow/assets/icon.png "${APPDIR}/vibeflow.png"
          ln -s vibeflow.png "${APPDIR}/.DirIcon"
          cat > "${APPDIR}/AppRun" <<'APPRUN'
          #!/bin/sh
          HERE="$(dirname "$(readlink -f "$0")")"
          exec "${HERE}/usr/bin/vibeflow" "$@"
          APPRUN
          chmod +x "${APPDIR}/AppRun"

      - name: Download appimagetool
        run: |
          set -euxo pipefail
          curl -fSL -o appimagetool \
            "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-x86_64.AppImage"
          chmod +x appimagetool

      - name: Build AppImage
        run: |
          set -euxo pipefail
          # --no-appstream skips appstreamcli metainfo validation (we don't ship
          # an AppStream XML for v0.1). ARCH=x86_64 disambiguates the output
          # filename for the appimagetool runtime.
          ARCH=x86_64 ./appimagetool --no-appstream AppDir vibeflow-x86_64.AppImage
          chmod +x vibeflow-x86_64.AppImage
          ls -la vibeflow-x86_64.AppImage

      - name: AppImage integrity check (headless-safe)
        run: |
          # We can't launch the GUI on a headless runner; the current CLI also
          # has no fast-exit --help, so attempting to launch hangs/fails. Just
          # verify the AppImage is a well-formed, executable file and that its
          # embedded binary unpacks cleanly.
          set -euxo pipefail
          file vibeflow-x86_64.AppImage
          test -x vibeflow-x86_64.AppImage
          ./vibeflow-x86_64.AppImage --appimage-extract >/dev/null
          test -x squashfs-root/usr/bin/vibeflow
          file squashfs-root/usr/bin/vibeflow
          rm -rf squashfs-root

      - name: Extract release notes from CHANGELOG
        id: notes
        run: |
          set -euxo pipefail
          TAG="${GITHUB_REF_NAME#v}"
          # Pull the section between "## [<TAG>] - " and the next "## [" header.
          awk -v tag="${TAG}" '
            BEGIN { in_section=0 }
            /^## \[/ {
              if (in_section) exit
              if ($0 ~ "^## \\[" tag "\\]") { in_section=1; next }
            }
            in_section { print }
          ' CHANGELOG.md > release-notes.md
          # Pre-release tags (e.g. v0.1.0-rc.1) have no matching CHANGELOG
          # section, so release-notes.md is empty; the resulting GitHub Release
          # body is blank. This is BY DESIGN — rc tags exist only to validate
          # the CI pipeline. The real v0.1.0 tag matches the CHANGELOG and
          # produces a populated body.
          echo "--- release-notes.md ($(wc -l < release-notes.md) lines) ---"
          cat release-notes.md

      - name: Create GitHub Release and attach AppImage
        uses: softprops/action-gh-release@v2
        with:
          name: "vibeflow ${{ github.ref_name }}"
          body_path: release-notes.md
          files: vibeflow-x86_64.AppImage
          draft: false
          prerelease: ${{ contains(github.ref_name, '-') }}
```

Notes on this workflow:

- **`prerelease: ${{ contains(github.ref_name, '-') }}`** — tags like `v0.1.0-rc.1` (containing `-`) are marked as pre-releases automatically. `v0.1.0` is a final release.
- **AppImage tool choice** — `appimagetool` from `github.com/AppImage/appimagetool` (the active repo). The older `github.com/AppImage/AppImageKit` is deprecated and its release-13 asset URL 404s. We deliberately do NOT use `linuxdeploy --output appimage` because it depends on a separately-distributed `linuxdeploy-plugin-appimage` whose discovery in CI is brittle; staging the AppDir ourselves (with explicit `AppRun`, `.DirIcon`, top-level `.desktop` + icon) and invoking `appimagetool` directly is simpler and more robust.
- **`AppRun`** — a minimal shell wrapper that resolves its own directory and execs the embedded `vibeflow` binary, forwarding args. `appimagetool` requires it; without it the resulting AppImage would not launch.
- **AppImage integrity check** — file/exec check + `--appimage-extract` round-trip. We can't launch the GPU on a headless runner; the released AppImage gets real VNC validation during the gated finale (the rc-tag step, or the post-release smoke).
- **`Extract release notes from CHANGELOG`** — pulls just the `v0.1.0` section. Rc tags produce an empty body (by design — see inline comment in the step).

- [ ] **Step 10.2: YAML syntax sanity**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))" \
  && echo "YAML OK"
```

Expected: `YAML OK`.

- [ ] **Step 10.3: Commit**

```bash
git add .github/workflows/release.yml
git status --short
git commit -m "ci(stage14): release workflow — AppImage on v* tag push

Triggers only on annotated v* tag push (so 'git push origin v0.1.0'
is the explicit human handoff; no accidental release from branch
work). Builds release binary, stages an AppDir with the vibeflow
icon + .desktop entry + AppRun shell wrapper, downloads pinned
appimagetool from github.com/AppImage/appimagetool (the active repo;
AppImageKit is deprecated), emits vibeflow-x86_64.AppImage, runs an
integrity check, extracts the matching CHANGELOG section as release
notes, and creates the GitHub Release with the AppImage attached.
Tags containing '-' (e.g. v0.1.0-rc.1) auto-mark as prerelease and
ship with an empty body by design."
```

---

## Task 11: Final verification + gated-finale documentation

**Files:**
- Create: `docs/release/v0.1.0-finale-checklist.md` (the human-run sequence)

- [ ] **Step 11.1: Full workspace gate (CI parity, locally)**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo doc --workspace --no-deps  # RUSTDOCFLAGS=-D warnings would be ideal; CI does that
```

Expected: all four succeed.

- [ ] **Step 11.2: Repo-README link checker (no external tools)**

```bash
python3 - <<'PY'
import re, sys, os
text = open("README.md", encoding="utf-8").read()
# Markdown links + img src (relative only).
refs = set(re.findall(r'\]\((?!https?://|#)([^)]+)\)', text))
refs |= set(re.findall(r'<img[^>]+src="(?!https?://)([^"]+)"', text))
missing = sorted(r for r in refs if not os.path.exists(r))
if missing:
    print("MISSING:", *missing, sep="\n  ")
    sys.exit(1)
print(f"all {len(refs)} relative links resolve")
PY
```

Expected: `all N relative links resolve` (N ≥ 5 — the lockup PNG, 2 LICENSE files, 2 docs files).

- [ ] **Step 11.3: 5-hook snippet round-trip (compare README vs lesson memory)**

This guards against accidental drift between the canonical lesson and the README.

```bash
python3 - <<'PY'
import json, re, sys
md = open("README.md", encoding="utf-8").read()
# Find the FIRST JSON block in the README (the 5-hook one).
m = re.search(r'```json\n(.*?)```', md, re.S)
assert m, "no JSON block found"
hooks = json.loads(m.group(1)).get("hooks", {})
expected = {"UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "Notification"}
got = set(hooks)
if got != expected:
    print("FAIL", "expected:", expected, "got:", got)
    sys.exit(1)
# Each entry runs vibeflow-emit with the right tool.
for k, v in hooks.items():
    cmds = [h["command"] for entry in v for h in entry["hooks"]]
    assert any("vibeflow-emit" in c and "--tool=claude" in c for c in cmds), f"hook {k} missing vibeflow-emit --tool=claude"
print("5-hook snippet: 5 hooks, all vibeflow-emit --tool=claude")
PY
```

Expected: `5-hook snippet: 5 hooks, all vibeflow-emit --tool=claude`.

- [ ] **Step 11.4: Create the gated-finale checklist document**

Create `docs/release/v0.1.0-finale-checklist.md`:

````markdown
# v0.1.0 finale checklist (human-run)

> Run these steps **in order, on the user's go-ahead**, after the
> `stage14-ship` branch has been holistic-reviewed and merged to `main`.
> Every step is irreversible (publish, tag push, release create, repo
> edit). DO NOT execute autonomously.

## Pre-flight

- [ ] `git checkout main && git pull --ff-only` — on green `main`.
- [ ] `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — green.
- [ ] CI status on `main` is green on GitHub (check `actions/workflows/ci.yml`).
- [ ] `npm login` on slmbeast — `npm whoami` returns `bjhengen`.
- [ ] `cargo login <token>` on slmbeast — `~/.cargo/credentials.toml` exists.
- [ ] (Optional but recommended) Employment-agreement legal review complete per project_identity_positioning.

## 1. Publish `vibeflow-protocol` to crates.io (if not already published)

```sh
cargo search vibeflow-protocol | head -3      # verify current registry state
cargo publish --dry-run -p vibeflow-protocol  # final dry-run
cargo publish -p vibeflow-protocol            # REAL publish; irreversible
```

If `vibeflow-protocol@0.1.0` is already on crates.io with the expected content, skip; otherwise publish.

## 2. Publish `vibeflow` to crates.io

```sh
cargo publish --dry-run -p vibeflow           # final dry-run
cargo publish -p vibeflow                     # REAL publish; irreversible
```

After ~5 minutes the crates.io badge in the README will start resolving.

## 3. Publish `vibeflow-protocol` to npm

```sh
cd bindings/npm
npm publish --dry-run                         # final dry-run (no auth needed)
npm publish                                   # REAL publish; will prompt for 2FA OTP if enabled; irreversible
cd -
```

## 4. (Recommended) Validate `release.yml` with a release-candidate tag first

Pushing `v0.1.0` blind risks discovering a `release.yml` bug on the real release. A pre-release tag exercises the entire workflow and the resulting GitHub Release is auto-marked as prerelease (per the `${{ contains(github.ref_name, '-') }}` expression in `release.yml`), so it's clearly distinguishable from the real one.

```sh
git tag -a v0.1.0-rc.1 -m "vibeflow v0.1.0 release-candidate (CI validation)"
git push origin v0.1.0-rc.1
```

Watch the Actions tab; when the workflow finishes, confirm a "vibeflow v0.1.0-rc.1" prerelease with `vibeflow-x86_64.AppImage` attached. Download and run it under VNC:

```sh
curl -fSL -O "https://github.com/bjhengen/vibeflow/releases/download/v0.1.0-rc.1/vibeflow-x86_64.AppImage"
chmod +x vibeflow-x86_64.AppImage
./vibeflow-x86_64.AppImage
```

Verify: window opens, logo shows in the taskbar/launcher entry, AI-state stripe works with `claude`.

Then clean up the rc release/tag:

```sh
gh release delete v0.1.0-rc.1 --yes
git push origin :refs/tags/v0.1.0-rc.1   # delete the remote tag
git tag -d v0.1.0-rc.1                   # delete the local tag
```

## 5. Push the `v0.1.0` annotated tag (this triggers release CI)

```sh
git tag -a v0.1.0 -m "vibeflow v0.1.0 — initial public release"
git push origin v0.1.0
```

The `release.yml` workflow runs on the tag push. Watch it in the GitHub Actions UI; when it completes, a GitHub Release named "vibeflow v0.1.0" appears with `vibeflow-x86_64.AppImage` attached and the CHANGELOG `v0.1.0` section as the body.

## 6. Repo metadata (one-shot, scriptable)

```sh
gh repo edit bjhengen/vibeflow \
  --description "GPU-accelerated Linux terminal that knows when your AI tool is waiting on you" \
  --homepage    "https://github.com/bjhengen/vibeflow" \
  --add-topic   "terminal" \
  --add-topic   "terminal-emulator" \
  --add-topic   "rust" \
  --add-topic   "wgpu" \
  --add-topic   "gpu" \
  --add-topic   "ai" \
  --add-topic   "linux" \
  --add-topic   "osc"
```

## 7. Social preview image (manual UI — `gh` cannot do this)

In the GitHub web UI: `Settings` → `General` → `Social preview` → `Edit` → upload `assets/vibeflow_logo_lockup.png`. Save.

## 8. VNC smoke walk on the released AppImage

```sh
curl -fSL -O "https://github.com/bjhengen/vibeflow/releases/download/v0.1.0/vibeflow-x86_64.AppImage"
chmod +x vibeflow-x86_64.AppImage
./vibeflow-x86_64.AppImage
```

Verify in the VNC session:
- Window opens with the vibeflow icon in the taskbar/launcher.
- Open a new tab; the indicator stripe appears.
- Launch `claude` (with the 5-hook settings in place) — Working/Waiting transitions visible.

If anything fails, see `lesson_spec_promise_vs_shell_integration` and `lesson_osc1338_hook_coverage` before assuming a regression.

## Rollback notes (read-only — no scripted rollback)

- crates.io: cannot unpublish. To withdraw a broken release, `cargo yank --version 0.1.0 -p vibeflow` (yank ≠ unpublish; installed builds keep working, new lookups skip the yanked version). Publish a fix as `0.1.1`.
- npm: cannot unpublish 72h after publish. Within 72h: `npm unpublish vibeflow-protocol@0.1.0`. After 72h: deprecate + publish a fix.
- GitHub Release: delete via UI or `gh release delete v0.1.0`; the tag remains unless also deleted.
- Tag: `git tag -d v0.1.0 && git push origin :refs/tags/v0.1.0` — destructive on shared history, only if no public clones have pulled it.
````

- [ ] **Step 11.5: Commit**

```bash
git add docs/release/v0.1.0-finale-checklist.md
git status --short
git commit -m "docs(stage14): v0.1.0 finale checklist (human-run, gated)

The irreversible publish/tag/release/repo-edit sequence the user runs
after merge. Each step has the exact command, an explicit dry-run,
and a rollback note. Social-preview upload is flagged as
GitHub-UI-only (gh cannot script it). Pre-flight requires npm login,
cargo login, and (recommended) employment-agreement legal review per
project_identity_positioning."
```

---

## Self-review summary (DO before pushing the branch)

After the implementer marks the last task complete, the controller runs:

```bash
git status                              # tree clean except .claude/
git log --oneline main..HEAD            # 11 commits, one per Task
git diff --stat main                    # spot-check no surprise large changes
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"
```

All green → branch is ready for senior holistic Sonnet review (per `feedback_senior_review_plans`), then merge to `main` via `--no-ff`, then the gated finale in Task 11's checklist.

---

## Spec coverage map (every spec section → its task)

- Spec §1 in-scope items 1–9 → covered by Tasks 1–11 as follows:
  - Item 1 (README rewrite) → Task 6 (repo) + Task 5 (crate README).
  - Item 2 (logo + icon wiring) → Task 1 (assets) + Tasks 2–3 (icon module + window wire) + Task 9 (.desktop) + Task 10 (AppImage stages it).
  - Item 3 (crates.io publish readiness) → Task 4.
  - Item 4 (npm publish of vibeflow-protocol unscoped) → Task 7.
  - Item 5 (CHANGELOG) → Task 8.
  - Item 6 (release CI / AppImage) → Task 10.
  - Item 7 (repo metadata) → Task 11.4 (documented; executed in finale).
  - Item 8 (v0.1.0 tag) → Task 11.4 (documented; executed in finale).
  - Item 9 (personal-identity attribution) → Tasks 4 (workspace authors) + 7 (npm author).
- Spec §2 (README depth + 5-hook + caveat) → Task 6 with byte-checked snippet in Task 11.3.
- Spec §3 (asset placement + winit icon + AppImage icon) → Tasks 1 + 2 + 3 + 9 + 10.
- Spec §4 (publish ordering + dry-runs + attribution correction) → Tasks 4 + 7 + 11.4 (finale).
- Spec §5 (CI + AppImage + CHANGELOG + Release + tag + gated finale) → Tasks 8 + 10 + 11.4.
- Spec §6 (repo metadata) → Task 11.4.
- Spec §7 (testing + dry-runs + VNC smoke walk + holistic review) → Tasks 2 + 4 + 7 + 11 + (post-merge: smoke walk + holistic review per workflow).
