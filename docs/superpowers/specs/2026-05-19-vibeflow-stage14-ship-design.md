# vibeflow Stage 14 — v0.1 Ship — design spec

**Date:** 2026-05-19
**Status:** Approved (brainstorm), pending implementation plan
**Branch base:** `main` @ `571e42e` (post-Stage-13 polish merged)

Stage 14 makes the feature-complete vibeflow terminal **installable, discoverable,
and tagged `v0.1.0`** — a clean public front door. No new terminal features; this
is release engineering, docs, and the logo.

> Code references (file:line) below were accurate at spec time on `main`
> @ `571e42e`; verify against current source before editing.

---

## 1. Scope & non-goals

**In scope**

1. **README rewrite** — comprehensive public front door (§2).
2. **Logo assets + icon wiring** — marketing PNGs in docs; runtime window icon
   (winit) + AppImage/`.desktop` icon (§3).
3. **crates.io publish readiness** — flip `crates/vibeflow` `publish = false`,
   fix stale metadata; verified publish ordering (§4).
4. **npm publish of `@vibeflow/protocol`** — the package is NOT currently on the
   registry (verified 2026-05-19: `registry.npmjs.org/@vibeflow/protocol` →
   `{"error":"Not found"}`); the README's "published on npm" claim is false
   today. `bindings/npm/package.json` is already well-formed at `0.1.0`. Publish
   it this phase (§4).
5. **`CHANGELOG.md`** — Keep-a-Changelog, `v0.1.0` entry summarizing the 14
   stages; source for the GitHub Release body (§5).
6. **Release CI** — new `.github/workflows/release.yml` on `v*` tag push: build
   release binary, bundle AppImage, cut GitHub Release, attach AppImage (§5).
7. **GitHub repo metadata** — description, topics, homepage (§6).
8. **`v0.1.0` annotated tag** — semver, distinct from the `stageNN-…-complete`
   internal-milestone pattern (§5).
9. **Personal-identity attribution** — published metadata must not carry the
   `friendly-robots.com` (LLC) domain (§4). Project ships as a personal
   open-source work under "Brian Hengen", no LLC affiliation, no warranty
   (existing dual MIT/Apache disclaimers). Deliberate: the goal is a useful
   free tool + personal AI credentials, not LLC revenue.

**Non-goals (v0.1)**

- `.deb`/apt packaging (deferred by user decision).
- npm-distribution of the **terminal binary** (npm-postinstall-fetch wrapper) —
  wrong channel for a Linux GPU desktop app; crates.io + AppImage suffices.
  Future/likely-never.
- GitHub Pages / docs site; Mac/Windows builds; binary signing/notarization;
  Homebrew/AUR; Tier-2 wrapper shims (separately deferred ~2026-05-21+).

**External / irreversible steps are explicitly human-gated** (executed only on
the user's go-ahead, never autonomously): `cargo publish` (×1–2), `npm publish`,
pushing the `v0.1.0` tag, `gh repo edit`, creating the GitHub Release. Everything
up to those is prepared and verified autonomously, including `--dry-run`s.

---

## 2. README rewrite

Full replacement of the current stale README (it still says "Pre-alpha … the
terminal binary is not yet built; this repository currently ships only the
protocol foundation" — false as of v0.1).

Structure (top to bottom):

1. **Logo lockup** (centered HTML `<p align="center"><img>`), tagline:
   *"A GPU-accelerated Linux terminal that knows when your AI tool is waiting on you."*
2. **Badges** — CI status + license always; crates.io (`vibeflow`) and npm
   (`@vibeflow/protocol`) version badges use shields.io's dynamic registry
   endpoints, which auto-populate once the gated publish (§4/§5) completes
   (they render "not found"/grey only in the brief same-day window between
   README merge and publish — self-healing, acceptable). The finale's
   README-claim-correction step (§4) verifies they resolve post-publish; if a
   publish is *deferred* (not just pending), the corresponding badge/line is
   removed so the README never makes a false claim.
3. **The idea** — one short paragraph: the AI-state thesis; the per-tab
   amber (waiting) / blue (working) cue.
4. **The 3-tier model** — concise: Tier-1 native OSC 1338 / Tier-2 wrapper
   shims *(planned)* / Tier-3 `/proc` heuristic.
5. **Features** — bullets: GPU render (wgpu), tabs, themes + iTerm2
   `--import-colors`, scrollback, configurable bell, keyboard/clipboard,
   per-tab AI-state.
6. **Install** — three subsections:
   - `cargo install vibeflow`
   - AppImage: download from Releases, `chmod +x`, run.
   - From source: `git clone … && cargo build --release`.
7. **Quick start** + **"Make AI-state work with Claude Code"** — the exact
   5-hook `~/.claude/settings.json` snippet **verbatim from
   `lesson_osc1338_hook_coverage`** (UserPromptSubmit/PreToolUse/PostToolUse →
   `vibeflow-emit working --tool=claude`; Stop/Notification → `… waiting …`),
   a one-line note for other tools (Codex/opencode), and the **plain-shell
   caveat** verbatim-in-spirit from `lesson_spec_promise_vs_shell_integration`
   / the post-Stage-13 spec §3 (a de-escalated Waiting tab in a bare bash
   shell without OSC 133 holds amber until a Tier-1/OSC-133 signal — intended,
   not stuck; enable OSC 133 in PS1 for prompt-driven recovery).
8. **Configuration** — `~/.config/vibeflow/config.toml` location, a documented
   sample config block, themes (`--import-colors <path> [--overwrite]`),
   keybindings table.
9. **Protocol** — link `docs/protocol.md`; the crates.io/npm package links
   (state-accurate).
10. **Contributing / Testing** — link `docs/TESTING.md`, CI expectations.
11. **License** — dual MIT/Apache (unchanged wording from current README).

A **crate-level README** (`crates/vibeflow/README.md`, referenced by
`readme = "README.md"` in §4) is a trimmed variant: tagline + install + the
AI-state quick-start, linking back to the repo for the full doc. (Separate
file because crates.io renders the crate-dir README, and the repo-root README
contains repo-relative asset/links that would 404 on crates.io.)

---

## 3. Logo assets & icon wiring

**Asset placement (two locations, deliberate):**

- `assets/` (repo root) — marketing PNGs extracted from
  `vibeflow_logo_exports.zip`: `vibeflow_logo_lockup.png` (README header),
  `vibeflow_icon_dark_gradient.png`, `vibeflow_icon_light_gradient.png`,
  `vibeflow_icon_dark_mono.png`. Referenced by the repo README only.
- `crates/vibeflow/assets/icon.png` — the **square runtime icon**: source
  `vibeflow_icon_dark_gradient.png` resized to **256×256** (one-time manual
  resize, committed as a static asset; no build-time image processing). Lives
  *inside the crate* so the crates.io package is self-contained (crates.io
  rejects files referenced outside the crate directory).

**Window icon (code — `crates/vibeflow`):**

- Add dependency `png = "0.17"` (lighter than `image`; decodes to RGBA8).
- New helper, e.g. `fn load_icon() -> Option<winit::window::Icon>`:
  `include_bytes!("../assets/icon.png")` → `png` decode → expand to RGBA8 →
  `winit::window::Icon::from_rgba(rgba, w, h).ok()`.
- Wire at window creation in `crates/vibeflow/src/window.rs` via
  `WindowAttributes::with_window_icon(load_icon())`.
- **Failure is non-fatal**: any decode/icon error → `None`, log at `warn`,
  terminal continues. A missing icon must never block startup.

**AppImage / desktop icon (packaging):**

- `packaging/vibeflow.desktop` —
  `[Desktop Entry] Type=Application Name=vibeflow Exec=vibeflow Icon=vibeflow Categories=System;TerminalEmulator;`.
- The 256×256 PNG staged by `release.yml` as
  `usr/share/icons/hicolor/256x256/apps/vibeflow.png` and the AppDir top-level
  `.DirIcon`, so the launcher/taskbar entry shows the logo.

**Cleanup:** after extraction the four PNGs are committed; the source
`vibeflow_logo_exports.zip` is deleted from the working tree (currently
untracked clutter — the committed PNGs are the source of truth).

**Invariant:** the only icon path the published crate depends on is
`crates/vibeflow/assets/icon.png` (embedded via `include_bytes!`). Repo-root
`assets/` is docs/packaging only.

---

## 4. Publish readiness & verification

**`crates/vibeflow/Cargo.toml` fixes (so `cargo install vibeflow` works):**

- Remove `publish = false`.
- `description` → *"GPU-accelerated Linux terminal emulator that knows when
  your AI tool is waiting on you."* (replaces the stale
  "…library crate, Stage 2 of v0.1").
- Add `readme = "README.md"` (the crate-level README from §2),
  `categories = ["command-line-utilities"]`,
  `keywords = ["terminal", "ai", "gpu", "wgpu", "vibeflow"]`
  (≤5 keywords, each ≤20 chars).
- Keep `[lib]` + `[[bin]]` (publishing a crate exposing both is valid;
  `cargo install vibeflow` installs the `vibeflow` binary).

**Verified publish ordering (do NOT trust README claims — the npm claim was
already proven false; treat the crates.io claim as unverified):**

1. **Verify** `vibeflow-protocol` actual crates.io state
   (`cargo search vibeflow-protocol` / registry API, not the README).
2. If absent or older than `0.1.0` → `cargo publish -p vibeflow-protocol`
   **first** (crates.io requires the path+`version="0.1"` dependency to exist
   on the registry before `vibeflow` can publish).
3. Then `cargo publish -p vibeflow`.
4. `npm publish --access public` from `bindings/npm/` for
   `@vibeflow/protocol@0.1.0` (scoped package → `--access public` required;
   independent of the crates).

**All publish actions are gated manual steps**, irreversible (crates.io/npm
yank ≠ unpublish). The plan prepares everything and runs
`cargo publish --dry-run -p vibeflow-protocol`, `-p vibeflow`, and
`npm publish --dry-run`; the real publishes run only on explicit user
go-ahead, in the order above.

**README claim correction:** package badges/links reflect *verified* published
state, not the current false text. If a publish is deferred mid-finale, the
README must not claim it.

**Personal-identity attribution correction (no LLC in public metadata):**

- `Cargo.toml` workspace `authors` and `bindings/npm/package.json` `author`:
  `Brian Hengen <brian@friendly-robots.com>` → `Brian Hengen <bhengen@gmail.com>`
  (own name, personal email, matching the git author identity
  `bjhengen <bhengen@gmail.com>` and the npm publish account). The `vibeflow`
  and `vibeflow-protocol` crates inherit via `authors.workspace = true` — one
  workspace line + one `package.json` line.
- **No further action needed / verified 2026-05-19:** `LICENSE-MIT` already
  reads `Copyright (c) 2026 Brian Hengen` (name only, no email/LLC);
  `LICENSE-APACHE` is unmodified boilerplate with no owner line; git history
  is already authored as `bhengen@gmail.com` (no commit-history rewrite /
  force-push required — that is explicitly out of scope and would be
  destructive on a public repo). The only LLC-domain leakage is the two
  manifest lines above.
- npm scope `@vibeflow` is project-named (not LLC-named) — neutral, no change;
  it just needs an npm org `vibeflow` under the personal account (§ npm
  prerequisites, tracked separately from this spec).

---

## 5. Release CI, AppImage, CHANGELOG, GitHub Release & tag

**`CHANGELOG.md`** (new, Keep a Changelog): a single
`## [0.1.0] - 2026-05-19` entry, user-facing bullets summarizing the 14
stages (GPU render; tabs; themes + iTerm2 import; scrollback; configurable
bell; keyboard/clipboard; **per-tab AI-state via OSC 1338**; config system +
hot reload). This is the verbatim source for the GitHub Release body.

**`.github/workflows/release.yml`** (new) — trigger `push: tags: ['v*']`:

1. Checkout; `dtolnay/rust-toolchain@stable`; `cargo build --release -p vibeflow`.
2. Build AppImage: stage `target/release/vibeflow` + `packaging/vibeflow.desktop`
   + the 256×256 icon into an AppDir; download **pinned** `linuxdeploy` +
   `appimagetool` in-job; produce `vibeflow-x86_64.AppImage`.
3. Create the GitHub Release for the tag, body = the `CHANGELOG.md` `0.1.0`
   section; attach `vibeflow-x86_64.AppImage`.

- Linux/x86_64 only; `ubuntu-latest` runner. Release notes state "built on
  ubuntu-latest; older distros may need `cargo install` / source." Older-glibc
  baseline is out of scope for v0.1.
- Existing `ci.yml` (fmt/clippy/build/test/fuzz/npm) is **unchanged**.

**Tag:** annotated `v0.1.0` (semver — deliberately distinct from the existing
`stageNN-…-complete` tags, which remain as internal stage milestones). Pushing
`v0.1.0` is the release trigger → **gated**.

**Gated finale (all human-confirmed, strict sequence):**

1. Stage-14 work merged to `main`; CI green.
2. Verify + `cargo publish` (`vibeflow-protocol` if needed → then `vibeflow`).
3. `npm publish --access public` (`@vibeflow/protocol`).
4. Push `v0.1.0` → `release.yml` builds AppImage + cuts the GitHub Release.
5. `gh repo edit` metadata (§6).

If any step fails, stop the finale and re-plan — do not proceed to later steps
(e.g. never push `v0.1.0` if `cargo publish` of `vibeflow` failed, since the
README/badges would then lie).

---

## 6. GitHub repo metadata

All `gh` calls — repo-visible, **batched and confirmed before running**:

- **Description:** *"vibeflow - a terminal for vibecoders"* →
  *"GPU-accelerated Linux terminal that knows when your AI tool is waiting on you"*.
- **Homepage:** the repo URL (`https://github.com/bjhengen/vibeflow`) — no
  separate site in scope.
- **Topics:** `terminal`, `terminal-emulator`, `rust`, `wgpu`, `gpu`, `ai`,
  `linux`, `osc`.
- **License rendering:** GitHub auto-detects one license file and shows
  Apache-only; dual licensing is correctly stated in README + all manifests.
  Cosmetic only — no action beyond confirming both `LICENSE-MIT` and
  `LICENSE-APACHE` are present (they are).
- **Social preview image:** setting it is **manual GitHub-UI only** (not
  scriptable via `gh`). The plan emits a one-line instruction for the user to
  upload `assets/vibeflow_logo_lockup.png` (Settings → General → Social
  preview). Not a blocker.

---

## 7. Testing & verification

- **Unit:** `load_icon()` decodes the embedded PNG to non-empty RGBA of the
  expected 256×256 dims (the only new code path); failure-returns-`None` path
  asserted with a deliberately bad byte slice.
- **Manifest/publish:** `cargo publish --dry-run -p vibeflow-protocol` and
  `-p vibeflow` succeed; `npm publish --dry-run` (from `bindings/npm/`) lists
  the expected files; `cargo build --release` clean.
- **Existing gate (CI parity):** full `cargo test --workspace --all-targets`
  + `cargo clippy --workspace --all-targets -- -D warnings` +
  `cargo fmt --all -- --check` green on `main`.
- **Docs:** every repo-README relative link resolves; the crate-README links
  resolve in crates.io context (no repo-relative paths); the 5-hook snippet is
  byte-checked against `lesson_osc1338_hook_coverage`.
- **Release CI dry-run:** validate `release.yml` before the real tag — push a
  throwaway pre-release tag (`v0.1.0-rc.1`) and confirm the workflow builds and
  attaches the AppImage; then **VNC smoke walk**: download/run the AppImage,
  confirm the terminal launches and the **logo icon shows in the taskbar /
  window**. Delete the rc tag/release after validation.
- **Holistic senior review** of the whole branch before the gated finale.

---

## Workflow

Spec → `superpowers:writing-plans` → senior pre-execution Sonnet review of the
plan vs actual source (per `feedback_senior_review_plans`) →
`superpowers:subagent-driven-development` (fresh implementer +
spec-compliance reviewer + code-quality reviewer per task; reviewers
constrained read-only / no git-fs mutation per
`lesson_review_subagent_destructive`; controller runs `git status` after every
task per `lesson_subagent_amend_drift`) → manual VNC smoke walk (AppImage +
window icon) → senior holistic review → merge `main` `--no-ff` → **gated
finale** (§5: publish ×(1–2 crates + npm) → push `v0.1.0` → `gh repo edit`).

`v0.1.0` is the project's first non-stage release tag; the `stageNN-…-complete`
tags are retained as internal milestones.
