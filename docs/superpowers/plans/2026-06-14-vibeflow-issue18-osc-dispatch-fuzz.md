# OscDispatcher Fuzz Target (#18) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a libfuzzer target that differentially fuzzes `OscDispatcher::feed` (segmented vs whole input) to harden split-frame reassembly before the blog launch.

**Architecture:** A new, self-contained fuzz crate `crates/vibeflow/fuzz/` (its own `[workspace]`, mirroring `crates/vibeflow-protocol/fuzz/`) depending on the `vibeflow` app crate. One target `osc_dispatch`: take arbitrary `Vec<Vec<u8>>` segments, feed them through one dispatcher and compare the event stream against feeding the concatenation whole — equal after coalescing `PassThrough` runs. Wire a 60s run into the existing CI fuzz job.

**Tech Stack:** Rust nightly, `cargo-fuzz` 0.13 / `libfuzzer-sys` 0.4, `OscDispatcher` in `crates/vibeflow/src/session/osc.rs`.

**Spec:** `docs/superpowers/specs/2026-06-14-vibeflow-issue18-osc-dispatch-fuzz-design.md`

**Prereqs (already verified on this machine):** `rustup` nightly toolchain installed; `cargo-fuzz 0.13.1` on PATH. `OscDispatcher`/`DispatchEvent` are `pub` at `vibeflow::session::osc` and `DispatchEvent` derives `Debug, Clone, PartialEq, Eq`.

> **Heads-up for the implementer:** the first `cargo +nightly fuzz build` compiles the entire `vibeflow` dependency graph (wgpu/winit/...) under nightly + AddressSanitizer. Expect several minutes — it is not hung. If that ASAN build fails for a sanitizer-incompatibility reason (not a code error in our target), report BLOCKED with the error rather than fighting it.

---

### Task 1: Create the fuzz crate and `osc_dispatch` target (red→green)

**Files:**
- Create: `crates/vibeflow/fuzz/.gitignore`
- Create: `crates/vibeflow/fuzz/Cargo.toml`
- Create: `crates/vibeflow/fuzz/fuzz_targets/osc_dispatch.rs`
- Modify: `Cargo.toml` (repo root — add the fuzz crate to `workspace.exclude`)

This task is TDD red→green: write the target with a *naive* equality assert (no coalescing), run the fuzzer and watch it FAIL almost immediately on a benign `PassThrough`-chunking difference (proving the harness actually detects whole-vs-split differences), then add `coalesce_passthrough` and watch a full 60s run pass.

- [ ] **Step 1: Create `.gitignore`** (mirrors the protocol fuzz crate)

`crates/vibeflow/fuzz/.gitignore`:
```gitignore
target/
corpus/
artifacts/
coverage/
Cargo.lock
```

- [ ] **Step 2: Create `Cargo.toml`**

`crates/vibeflow/fuzz/Cargo.toml`:
```toml
[workspace]

[package]
name = "vibeflow-fuzz"
version = "0.0.0"
publish = false
edition = "2021"

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
arbitrary = "1"

[dependencies.vibeflow]
path = ".."

[[bin]]
name = "osc_dispatch"
path = "fuzz_targets/osc_dispatch.rs"
test = false
doc = false
bench = false
```

(The empty `[workspace]` detaches this crate from the root workspace — exactly like `crates/vibeflow-protocol/fuzz/Cargo.toml` — so `cargo build/test --workspace` at the repo root never includes it.)

- [ ] **Step 2b: Exclude the fuzz crate from the root workspace**

The root `Cargo.toml` uses an explicit `members` list and already excludes the protocol fuzz crate. Mirror that for the new one (hygiene + defends against a future glob). In `/home/bhengen/dev/vibeflow/Cargo.toml`, change:
```toml
exclude = ["crates/vibeflow-protocol/fuzz"]
```
to:
```toml
exclude = ["crates/vibeflow-protocol/fuzz", "crates/vibeflow/fuzz"]
```

- [ ] **Step 3: Create the target with a NAIVE assert (the red version)**

`crates/vibeflow/fuzz/fuzz_targets/osc_dispatch.rs`:
```rust
#![no_main]

use libfuzzer_sys::fuzz_target;
use vibeflow::session::osc::OscDispatcher;

fuzz_target!(|segments: Vec<Vec<u8>>| {
    // Whole: feed the concatenation in one call.
    let whole_input: Vec<u8> = segments.concat();
    let mut whole_dispatcher = OscDispatcher::new();
    let whole_events = whole_dispatcher.feed(&whole_input);

    // Split: feed each segment through a fresh dispatcher.
    let mut split_dispatcher = OscDispatcher::new();
    let mut split_events = Vec::new();
    for seg in &segments {
        split_events.extend(split_dispatcher.feed(seg));
    }

    // NAIVE (intentionally wrong) — will fail on PassThrough chunking.
    assert_eq!(whole_events, split_events);
});
```

- [ ] **Step 4: Build the target**

Run (from the app crate dir, which now contains `fuzz/`):
```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow && cargo +nightly fuzz build osc_dispatch
```
Expected: compiles successfully (slow — ASAN build of the app graph). If it fails to find `OscDispatcher`, the path is `vibeflow::session::osc::OscDispatcher` — confirm the import. Paste the final lines.

- [ ] **Step 5: Run the fuzzer and verify it FAILS fast (red)**

Run:
```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow && cargo +nightly fuzz run osc_dispatch -- -max_total_time=20
```
Expected: a crash within ~a second (well before 20s) — libfuzzer prints a panic from the `assert_eq!` and writes an artifact under `fuzz/artifacts/osc_dispatch/` (gitignored). This proves the differential harness detects whole-vs-split differences. The trigger is benign: e.g. segments `[[b'x'], [b'y']]` → split yields `PassThrough([120]), PassThrough([121])` while whole yields `PassThrough([120, 121])`. Paste the crash summary.

- [ ] **Step 6: Add `coalesce_passthrough` and use it (green)**

Replace the file body so it imports `DispatchEvent`, wraps both sides in `coalesce_passthrough`, and defines the helper:
```rust
#![no_main]

use libfuzzer_sys::fuzz_target;
use vibeflow::session::osc::{DispatchEvent, OscDispatcher};

fuzz_target!(|segments: Vec<Vec<u8>>| {
    // Whole: feed the concatenation in one call.
    let whole_input: Vec<u8> = segments.concat();
    let mut whole_dispatcher = OscDispatcher::new();
    let whole_events = whole_dispatcher.feed(&whole_input);

    // Split: feed each segment through a fresh dispatcher.
    let mut split_dispatcher = OscDispatcher::new();
    let mut split_events = Vec::new();
    for seg in &segments {
        split_events.extend(split_dispatcher.feed(seg));
    }

    // A streaming parser must be split-invariant once the only representation
    // difference — how PassThrough byte-runs are chunked across feed() calls —
    // is normalised away. Completed-sequence events (AiState / Prompt /
    // SetTitle / Osc52Write) occur at the same logical point regardless of
    // split, so the coalesced streams must be equal. A genuine reassembly bug
    // (a sequence recognised whole but missed when split, or vice versa) is
    // exactly what this catches and is NOT normalised away.
    assert_eq!(
        coalesce_passthrough(whole_events),
        coalesce_passthrough(split_events),
        "OscDispatcher produced different events for whole vs segmented input"
    );
});

/// Merge consecutive `PassThrough(bytes)` events into one.
fn coalesce_passthrough(events: Vec<DispatchEvent>) -> Vec<DispatchEvent> {
    let mut out: Vec<DispatchEvent> = Vec::with_capacity(events.len());
    for ev in events {
        match (out.last_mut(), &ev) {
            (Some(DispatchEvent::PassThrough(acc)), DispatchEvent::PassThrough(next)) => {
                acc.extend_from_slice(next);
            }
            _ => out.push(ev),
        }
    }
    out
}
```

- [ ] **Step 7: Run the fuzzer for 60s and verify it PASSES (green)**

Run:
```bash
cd /home/bhengen/dev/vibeflow/crates/vibeflow && rm -rf fuzz/artifacts/osc_dispatch && cargo +nightly fuzz run osc_dispatch -- -max_total_time=60
```
Expected: runs the full 60s and exits 0 (libfuzzer prints `Done ... ` with no crash). Paste the final summary line.

**If it crashes with a difference that is NOT a PassThrough-chunking artifact:** that may be a *real reassembly bug* in `OscDispatcher`. STOP and report DONE_WITH_CONCERNS with the minimized crashing input (`fuzz/artifacts/osc_dispatch/...`) and your read of whether it's (a) a real dispatcher bug — which becomes its own issue/fix, do NOT paper over it — or (b) another legitimate representation difference that `coalesce_passthrough` should normalise (extend the helper, re-run).

- [ ] **Step 8: Confirm the root workspace is unaffected, then commit**

Run: `cd /home/bhengen/dev/vibeflow && cargo build --workspace 2>&1 | tail -3`
Expected: builds the normal workspace WITHOUT mentioning `vibeflow-fuzz` (the fuzz crate's own `[workspace]` excludes it).

Run: `git -C /home/bhengen/dev/vibeflow status --short`
Expected: the three new `crates/vibeflow/fuzz/` files (`.gitignore`, `Cargo.toml`, `fuzz_targets/osc_dispatch.rs`) plus the modified root ` M Cargo.toml` (the exclude edit), plus the pre-existing untracked `.claude/`, `drafts/`. The fuzz crate's `target/`, `corpus/`, `artifacts/`, `Cargo.lock` must NOT appear (they're gitignored). If `Cargo.lock` or `target/` show up, the `.gitignore` is wrong — fix before committing.

Commit:
```bash
cd /home/bhengen/dev/vibeflow
git add Cargo.toml crates/vibeflow/fuzz/.gitignore crates/vibeflow/fuzz/Cargo.toml crates/vibeflow/fuzz/fuzz_targets/osc_dispatch.rs
git commit -m "$(cat <<'MSG'
test(#18): add osc_dispatch fuzz target (split-vs-whole differential)

New self-contained fuzz crate crates/vibeflow/fuzz (mirrors the protocol fuzz
crate) with an osc_dispatch libfuzzer target: feeds arbitrary Vec<Vec<u8>>
segments through OscDispatcher::feed and asserts the event stream equals
feeding the concatenation whole, after coalescing consecutive PassThrough runs
(the only chunk-dependent representation). Targets split-frame reassembly +
no-panic. No production code change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 2: Wire the target into CI and the changelog

**Files:**
- Modify: `.github/workflows/ci.yml` (the `fuzz:` job — cache workspaces + a run step)
- Modify: `CHANGELOG.md` (`[Unreleased]`)

- [ ] **Step 1: Extend the fuzz job's cache to include the new workspace**

In `.github/workflows/ci.yml`, the `fuzz:` job has a Swatinem cache step. Change its `with:` block from:
```yaml
        with:
          workspaces: "crates/vibeflow-protocol/fuzz"
```
to (Swatinem accepts a newline-separated list of workspaces):
```yaml
        with:
          workspaces: |
            crates/vibeflow-protocol/fuzz
            crates/vibeflow/fuzz
```

- [ ] **Step 2: Add the run step for the new target**

In the same `fuzz:` job, immediately AFTER the existing step:
```yaml
      - name: Run parse fuzzer for 60s
        working-directory: crates/vibeflow-protocol
        run: cargo +nightly fuzz run parse -- -max_total_time=60
```
add:
```yaml

      - name: Run osc_dispatch fuzzer for 60s
        working-directory: crates/vibeflow
        run: cargo +nightly fuzz run osc_dispatch -- -max_total_time=60
```

- [ ] **Step 3: Validate the workflow YAML parses**

Run: `python3 -c "import yaml; yaml.safe_load(open('/home/bhengen/dev/vibeflow/.github/workflows/ci.yml')); print('YAML OK')"`
Expected: prints `YAML OK` (no exception). This catches indentation/syntax mistakes locally; the actual job behaviour is verified when CI runs on the PR.

- [ ] **Step 4: Add the CHANGELOG entry**

In `CHANGELOG.md`, under `## [Unreleased]` (which already has `### Added` for #19 and `### Fixed` for #17), add a new `### Internal` subsection after the existing ones:
```markdown
### Internal

- **Fuzz target for the streaming OSC dispatcher (#18).** New `crates/vibeflow/fuzz` crate with an `osc_dispatch` libfuzzer target: it feeds arbitrary input through `OscDispatcher::feed` as random segments and asserts the resulting event stream matches feeding the input whole (after coalescing `PassThrough` runs) — a differential check targeting split-frame reassembly — plus the no-panic property. Runs 60s in the CI fuzz smoke alongside the protocol `parse` fuzzer.
```

- [ ] **Step 5: Commit**

```bash
cd /home/bhengen/dev/vibeflow
git add .github/workflows/ci.yml CHANGELOG.md
git commit -m "ci(#18): run osc_dispatch fuzzer 60s in CI; changelog"
```

---

## Self-Review

- **Spec coverage:** §1 crate structure → Task 1 Steps 1–2 (+ rejected-alternatives are design rationale, no task). §2 the target (arbitrary `Vec<Vec<u8>>`, whole vs split, assert) → Task 1 Steps 3–7. §3 soundness / overflow verification + "don't normalise away real bugs" → Task 1 Step 7's stop-condition. §4 CI (cache workspace + run step) → Task 2 Steps 1–3. Testing/validation (build, 60s run, root workspace unaffected) → Task 1 Steps 4,7,8. CHANGELOG note → Task 2 Step 4. `.gitignore`/no-Cargo.lock convention → Task 1 Steps 1, 8. All covered.
- **Placeholder scan:** none — every file body and command is concrete.
- **Type/name consistency:** crate `vibeflow-fuzz`; target `osc_dispatch`; import `vibeflow::session::osc::{DispatchEvent, OscDispatcher}`; helper `coalesce_passthrough`; input `Vec<Vec<u8>>` named `segments`. Consistent across tasks. `DispatchEvent::PassThrough(Vec<u8>)` matches `osc.rs`. The naive red version (Step 3) imports only `OscDispatcher`; the green version (Step 6) adds `DispatchEvent` — intentional, called out.
- **Senior review (2026-06-14, Sonnet) applied — verdict READY:** differential confirmed SOUND against `feed()` source (overflow / malformed-OSC / unterminated / unknown-OSC-forward paths all collapse to `PassThrough` chunking, which `coalesce_passthrough` normalises — no false-positive path). Folded in two nits: trimmed `arbitrary` to `"1"` (the `derive` feature is unused; `Vec<Vec<u8>>` uses libfuzzer-sys's re-exported `Arbitrary` blanket impls — direct dep kept as the common typed-input pattern, version-unifies with libfuzzer-sys's own `arbitrary` 1.x so no duplicate), and added `crates/vibeflow/fuzz` to the root `workspace.exclude` (Step 2b). Confirmed: `vibeflow` has an explicit `[lib]`, the `pub` chain reaches `OscDispatcher`/`DispatchEvent`, `PassThrough(Vec<u8>)` is a tuple variant, the `coalesce` accumulator borrow compiles, root `members` is explicit (no glob), Swatinem multiline `workspaces:` is valid.
