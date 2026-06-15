# vibeflow-protocol Implementation Plan (Stage 1 of v0.1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `vibeflow-protocol` foundation — a zero-dependency Rust crate, an npm sibling package, and the `vibeflow-emit` shell helper — that defines and implements the OSC 1338 wire format vibeflow's AI-state awareness is built on.

**Architecture:** Two parallel implementations of the same wire format (`ESC ] 1338 ; key=value... BEL`), with shared docs as the canonical contract. Rust crate is a pure-`std` library plus a tiny CLI binary in the same crate. npm package mirrors the API in TypeScript with no runtime dependencies. Both ship from v0.1 release. Comprehensive unit tests, a proptest round-trip, and a `cargo-fuzz` harness on the parser ensure the format is locked-in correctly before downstream code (Stage 2's `OscDispatcher`) starts depending on it.

**Tech Stack:** Rust 2021 edition (stable), Cargo workspace; `proptest` (dev-dep) for property tests; `cargo-fuzz` + nightly Rust for fuzzing; TypeScript + Node `>=18` for the npm package; GitHub Actions for CI.

**Stage scope:** This plan covers Stage 1 only. Stages 2–N (`OscDispatcher`, `AiStateTracker`, PTY plumbing, GPU rendering, tab bar, input, config, shell hooks) get their own plans written later. Stage 1 is independently shippable: AI tools can adopt the protocol against published artifacts even before vibeflow itself exists.

**Rust newcomer note:** This is Brian's first substantial Rust project. Several tasks include short inline rationale ("**Why:**") for non-obvious idioms (`Copy` on enums, builder methods that consume `self`, `&[u8]` vs `Vec<u8>`, the `?` operator, `derive` attributes). Don't optimise these away — they're load-bearing learning material.

---

## File Structure

Workspace root (`/path/to/vibeflow/`) — files created by this plan:

| Path | Responsibility |
|---|---|
| `Cargo.toml` | Workspace manifest. Members: `crates/vibeflow-protocol`. (`crates/vibeflow` added in Stage 2.) |
| `LICENSE-MIT`, `LICENSE-APACHE` | Dual-license text. |
| `.gitignore` | `target/`, `node_modules/`, fuzz artefacts, editor cruft. |
| `README.md` | Top-level project README. |
| `crates/vibeflow-protocol/Cargo.toml` | Crate manifest. Zero runtime deps; `proptest` in `[dev-dependencies]`. Declares both `lib` and `[[bin]] vibeflow-emit` targets. |
| `crates/vibeflow-protocol/src/lib.rs` | The library: `State`, `Frame`, `ParseError`, `parse`, `to_bytes`, `emit`, `emit_state`, percent-encode helpers, constants. |
| `crates/vibeflow-protocol/src/bin/vibeflow-emit.rs` | CLI binary that emits one frame and exits. |
| `crates/vibeflow-protocol/README.md` | crates.io front-page README. |
| `crates/vibeflow-protocol/fuzz/Cargo.toml` | Fuzz crate manifest (excluded from main workspace; depends on `libfuzzer-sys` and the protocol crate). |
| `crates/vibeflow-protocol/fuzz/fuzz_targets/parse.rs` | Single fuzz target: feed arbitrary bytes to `parse`, assert no panic. |
| `bindings/npm/package.json` | npm manifest for `@vibeflow/protocol`. |
| `bindings/npm/tsconfig.json` | TypeScript config. |
| `bindings/npm/src/index.ts` | TS implementation: `State`, `Frame`, `toBytes`, `parse`, `emit`, `emitState`. |
| `bindings/npm/test/index.test.ts` | Unit tests using Node's built-in `node:test` runner. |
| `bindings/npm/README.md` | npm front-page README. |
| `docs/protocol.md` | The canonical OSC 1338 wire-format spec — referenced by both bindings as the source of truth. |
| `.github/workflows/ci.yml` | GitHub Actions: Rust `build`/`test`/`clippy`/`fmt`, fuzz smoke run, npm `build`/`test`. |

---

## Task 0: Workspace bootstrap

**Files:**
- Create: `/path/to/vibeflow/Cargo.toml`
- Create: `/path/to/vibeflow/.gitignore`
- Create: `/path/to/vibeflow/LICENSE-MIT`
- Create: `/path/to/vibeflow/LICENSE-APACHE`
- Create: `/path/to/vibeflow/README.md`
- Delete: `/path/to/vibeflow/ai_term/` (empty, leftover)

- [ ] **Step 1: Remove the empty leftover directory**

```bash
rmdir /path/to/vibeflow/ai_term
```

Expected: succeeds silently. (If the directory has been touched and is non-empty, stop and inspect — don't `rm -rf`.)

- [ ] **Step 2: Create the workspace manifest**

Write `/path/to/vibeflow/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/vibeflow-protocol"]
exclude = ["crates/vibeflow-protocol/fuzz"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
repository = "https://github.com/bjhengen/vibeflow"
homepage = "https://github.com/bjhengen/vibeflow"
authors = ["Brian Hengen <bhengen@gmail.com>"]
rust-version = "1.75"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
# Default lint groups only. `pedantic`/`nursery` are opt-in per crate later
# if the user wants — they'd otherwise fire on lots of idiomatic code and
# block CI for a Rust newcomer in surprising ways.
all = { level = "warn", priority = -1 }
```

**Why `resolver = "2"`:** the new feature resolver, required for workspaces; default in `edition = "2021"` for a single-crate package but must be set explicitly in workspace manifests.

**Why `exclude = ["…/fuzz"]`:** the fuzz crate depends on `libfuzzer-sys` and only builds with nightly Rust. Keeping it out of the workspace means `cargo build` on stable still works.

- [ ] **Step 3: Create `.gitignore`**

Write `/path/to/vibeflow/.gitignore`:

```gitignore
# Rust
/target
/crates/*/target
**/*.rs.bk
Cargo.lock.bak

# cargo-fuzz
/crates/vibeflow-protocol/fuzz/target
/crates/vibeflow-protocol/fuzz/corpus
/crates/vibeflow-protocol/fuzz/artifacts
/crates/vibeflow-protocol/fuzz/coverage

# Node
node_modules/
dist/
*.tsbuildinfo

# Editors
.idea/
.vscode/
*.swp
*~
.DS_Store

# Local Claude Code state
.claude/settings.local.json

# Superpowers brainstorm scratch (mockups, server state)
.superpowers/

# Logs
*.log
```

**Note:** `Cargo.lock` is *not* ignored — for binaries (and we have one) it should be checked in. For a pure library it would be ignored, but the workspace contains both.

- [ ] **Step 4: Add license files**

Write `/path/to/vibeflow/LICENSE-MIT` (embedded — MIT is short and we don't want to depend on a fetch):

```
MIT License

Copyright (c) 2026 Brian Hengen

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

For Apache-2.0 (the full text is 200+ lines — fetch from the canonical source):

```bash
cd /path/to/vibeflow
curl -sSLo LICENSE-APACHE https://www.apache.org/licenses/LICENSE-2.0.txt
head -3 LICENSE-APACHE
```

Expected: the file begins with `                                 Apache License` and shows version 2.0 in the next two lines. (The standard Apache-2.0 file does not include a copyright notice — that's done in source-file headers, which we're not using.)

- [ ] **Step 5: Write the top-level `README.md`**

Write `/path/to/vibeflow/README.md`:

```markdown
# vibeflow

A GPU-accelerated terminal emulator for Linux that knows when your AI tool is waiting on you.

> **Status:** Pre-alpha. The `vibeflow-protocol` foundation lands first; the terminal itself follows.

## Repository layout

- `crates/vibeflow-protocol/` — the open-standard OSC 1338 protocol library + `vibeflow-emit` CLI. Published to crates.io.
- `bindings/npm/` — `@vibeflow/protocol`, the TypeScript sibling. Published to npm.
- `docs/protocol.md` — the canonical OSC 1338 wire-format specification.
- `docs/superpowers/specs/` — design specs.
- `docs/superpowers/plans/` — implementation plans.

## License

Dual-licensed under either of:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
```

- [ ] **Step 6: Verify the workspace TOML parses**

The protocol crate doesn't exist yet (it lands in Task 1), so `cargo build` will fail. All we want here is "did Cargo accept the workspace manifest?":

```bash
cd /path/to/vibeflow
cargo metadata --no-deps --format-version 1 2>&1 | head -5
```

Expected: either `metadata` JSON output, or an error like `failed to load manifest for workspace member`. **Both are fine.** What we're checking against is a TOML parse error from the workspace manifest itself (e.g., `error parsing TOML at Cargo.toml:N:M`) — if you see one, fix the manifest before continuing.

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add Cargo.toml .gitignore LICENSE-MIT LICENSE-APACHE README.md
# ai_term/ removal is a tracked deletion if it was tracked; if not, it just disappears.
git status   # confirm staging looks right
git commit -m "chore: bootstrap Cargo workspace, dual license, and project README"
```

---

## Task 1: vibeflow-protocol crate skeleton + State enum (TDD)

**Files:**
- Create: `crates/vibeflow-protocol/Cargo.toml`
- Create: `crates/vibeflow-protocol/src/lib.rs`

- [ ] **Step 1: Create the crate manifest**

Write `crates/vibeflow-protocol/Cargo.toml`:

```toml
[package]
name = "vibeflow-protocol"
description = "OSC 1338 protocol — open standard for AI-tool state signalling in terminals"
categories = ["command-line-interface", "parser-implementations"]
keywords = ["terminal", "osc", "ai", "vibeflow"]
readme = "README.md"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
authors.workspace = true
rust-version.workspace = true

[lints]
workspace = true

[lib]
path = "src/lib.rs"

# The vibeflow-emit binary lands in Task 8; its [[bin]] entry is added then.

[dev-dependencies]
proptest = "1"
```

**Why `version.workspace = true`:** workspace inheritance — keeps version/license/etc. in one place (the workspace `Cargo.toml`).

- [ ] **Step 2: Stub `lib.rs` so the crate builds**

Write `crates/vibeflow-protocol/src/lib.rs`:

```rust
//! OSC 1338 protocol — vibeflow's open standard for AI-tool state signalling.
//!
//! See `docs/protocol.md` in the workspace root for the canonical wire-format spec.
```

- [ ] **Step 3: Verify it builds**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow-protocol --lib
```

Expected: `Finished` with no errors.

- [ ] **Step 4: Write the failing test for `State`**

Append to `crates/vibeflow-protocol/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_as_str_round_trips_via_from_str() {
        for s in ["active", "working", "waiting", "done"] {
            let parsed: State = s.parse().expect("known state must parse");
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[test]
    fn state_unknown_string_is_an_error() {
        let err = "frobnicating".parse::<State>().unwrap_err();
        assert!(matches!(err, ParseError::UnknownState(ref s) if s == "frobnicating"));
    }
}
```

Run:

```bash
cargo test -p vibeflow-protocol
```

Expected: compilation fails — `cannot find type State`, `cannot find type ParseError`. Good: red phase.

- [ ] **Step 5: Implement `State` and a stub `ParseError`**

Insert *above* the `#[cfg(test)] mod tests` block in `crates/vibeflow-protocol/src/lib.rs`:

```rust
/// Per-tab AI-tool state, as carried in the `state` parameter of OSC 1338.
///
/// Variants are listed in order of "loudness" of the visual indicator:
/// `Active` is the default with no special styling; `Waiting` is the headline
/// state that pulses amber on the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    /// The default — nothing special is happening on this tab.
    Active,
    /// A tool is running / generating; tab shows a steady blue stripe.
    Working,
    /// A tool is waiting for user input; tab pulses amber. The headline state.
    Waiting,
    /// A tool just finished a task; usually a transient state that flips back to `Active`.
    Done,
}

impl State {
    /// The wire string for this state, as it appears in OSC 1338.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Working => "working",
            State::Waiting => "waiting",
            State::Done => "done",
        }
    }
}

impl std::str::FromStr for State {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(State::Active),
            "working" => Ok(State::Working),
            "waiting" => Ok(State::Waiting),
            "done" => Ok(State::Done),
            _ => Err(ParseError::UnknownState(s.to_owned())),
        }
    }
}

/// Errors produced when parsing OSC 1338 frames or state strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Bytes are not an OSC 1338 sequence (wrong prefix or wrong identifier).
    NotOurOsc,
    /// The required `state` key was absent.
    MissingState,
    /// The `state` value was not one of the four known variants.
    UnknownState(String),
    /// The sequence was structurally malformed (e.g., no terminator).
    Malformed(&'static str),
    /// The sequence exceeded the 4 KiB cap.
    TooLong,
    /// A percent-encoded byte was malformed.
    BadEncoding,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NotOurOsc => f.write_str("not an OSC 1338 sequence"),
            ParseError::MissingState => f.write_str("missing required `state` key"),
            ParseError::UnknownState(s) => write!(f, "unknown state value: {s}"),
            ParseError::Malformed(why) => write!(f, "malformed sequence: {why}"),
            ParseError::TooLong => f.write_str("sequence exceeds 4 KiB"),
            ParseError::BadEncoding => f.write_str("invalid percent encoding"),
        }
    }
}

impl std::error::Error for ParseError {}
```

**Why `pub fn as_str(self)` not `(&self)`:** `State` is `Copy` (cheap, four fixnum variants). Taking it by value avoids a pointless borrow. **Why `to_owned()` not `to_string()`:** clippy nudges you toward `to_owned()` for `&str → String`; both work, this is convention.

- [ ] **Step 6: Run the tests — they pass**

```bash
cargo test -p vibeflow-protocol
```

Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow-protocol/Cargo.toml crates/vibeflow-protocol/src/lib.rs
git commit -m "feat(protocol): add State enum and ParseError"
```

---

## Task 2: Frame struct (TDD)

**Files:**
- Modify: `crates/vibeflow-protocol/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `crates/vibeflow-protocol/src/lib.rs`:

```rust
    #[test]
    fn frame_new_has_state_only() {
        let f = Frame::new(State::Working);
        assert_eq!(f.state, State::Working);
        assert_eq!(f.tool, None);
        assert_eq!(f.project, None);
    }

    #[test]
    fn frame_with_tool_and_project_builds_correctly() {
        let f = Frame::new(State::Waiting)
            .with_tool("claude")
            .with_project("vibeflow");
        assert_eq!(f.state, State::Waiting);
        assert_eq!(f.tool.as_deref(), Some("claude"));
        assert_eq!(f.project.as_deref(), Some("vibeflow"));
    }
```

Run `cargo test -p vibeflow-protocol`. Expected: compile error — `Frame` not found.

- [ ] **Step 2: Implement `Frame`**

Insert *above* the `mod tests` block:

```rust
/// A single OSC 1338 frame's contents.
///
/// Construct with [`Frame::new`] and chain [`Frame::with_tool`] / [`Frame::with_project`]:
///
/// ```
/// use vibeflow_protocol::{Frame, State};
/// let f = Frame::new(State::Waiting).with_tool("claude").with_project("vibeflow");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub state: State,
    pub tool: Option<String>,
    pub project: Option<String>,
}

impl Frame {
    /// A new frame with only the required `state` field set.
    #[must_use]
    pub fn new(state: State) -> Self {
        Self { state, tool: None, project: None }
    }

    /// Set the optional `tool` field. Returns `self` for chaining.
    #[must_use]
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }

    /// Set the optional `project` field. Returns `self` for chaining.
    #[must_use]
    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }
}
```

**Why builder methods consume `self`:** the consuming-builder pattern lets you chain calls naturally (`Frame::new(s).with_tool("x")`). Each method takes ownership, mutates, returns ownership. No borrow-checker drama, no `&mut` lifetime juggling — this is the idiomatic Rust shape for ergonomic short-lived builders. **Why `impl Into<String>`:** lets callers pass `&str` *or* `String` without worrying.

- [ ] **Step 3: Run tests — pass**

```bash
cargo test -p vibeflow-protocol
```

Expected: `test result: ok. 4 passed; 0 failed`.

- [ ] **Step 4: Commit**

```bash
git add crates/vibeflow-protocol/src/lib.rs
git commit -m "feat(protocol): add Frame struct with consuming builder"
```

---

## Task 3: Percent encode/decode (TDD)

**Files:**
- Modify: `crates/vibeflow-protocol/src/lib.rs`

OSC 1338 uses `;` as the parameter separator and `=` as the key/value separator, so values containing those — plus control bytes, `%` itself, and any non-ASCII bytes — must be percent-encoded.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block:

```rust
    #[test]
    fn percent_encode_passes_safe_ascii_through() {
        assert_eq!(percent_encode("hello-world_42"), "hello-world_42");
    }

    #[test]
    fn percent_encode_escapes_specials() {
        assert_eq!(percent_encode("a;b=c"), "a%3Bb%3Dc");
        assert_eq!(percent_encode("100%"), "100%25");
    }

    #[test]
    fn percent_encode_escapes_non_ascii_as_utf8_bytes() {
        // "café" → c, a, f, é (0xC3 0xA9 in UTF-8)
        assert_eq!(percent_encode("café"), "caf%C3%A9");
    }

    #[test]
    fn percent_decode_roundtrips_arbitrary_strings() {
        for s in ["", "plain", "a;b=c", "café", "100%", "tab\there"] {
            let encoded = percent_encode(s);
            let decoded = percent_decode(&encoded).expect("round-trip");
            assert_eq!(decoded, s);
        }
    }

    #[test]
    fn percent_decode_rejects_truncated_escape() {
        assert_eq!(percent_decode("foo%2"), Err(ParseError::BadEncoding));
        assert_eq!(percent_decode("foo%"), Err(ParseError::BadEncoding));
    }

    #[test]
    fn percent_decode_rejects_non_hex_digits() {
        assert_eq!(percent_decode("foo%ZZ"), Err(ParseError::BadEncoding));
    }
```

Run `cargo test -p vibeflow-protocol`. Expected: compile errors — functions not found.

- [ ] **Step 2: Implement encode/decode**

Insert above `mod tests`:

```rust
/// Returns true for bytes that must be percent-encoded in an OSC 1338 value:
/// control bytes (0x00–0x1F, 0x7F), `;`, `=`, `%`, and any non-ASCII byte.
#[inline]
#[allow(dead_code)] // removed in Task 4 once `Frame::to_bytes` calls `percent_encode`
fn needs_encoding(b: u8) -> bool {
    b < 0x20 || b == 0x7f || b == b';' || b == b'=' || b == b'%' || b > 0x7f
}

#[must_use]
#[allow(dead_code)] // removed in Task 4 once `Frame::to_bytes` calls this
pub(crate) fn percent_encode(s: &str) -> String {
    let bytes = s.as_bytes();
    // Fast path: nothing to encode.
    if !bytes.iter().copied().any(needs_encoding) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if needs_encoding(b) {
            // %XX, uppercase hex (RFC 3986 convention).
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        } else {
            out.push(b as char);
        }
    }
    out
}

#[allow(dead_code)] // removed in Task 5 once `parse` calls this
pub(crate) fn percent_decode(s: &str) -> Result<String, ParseError> {
    let bytes = s.as_bytes();
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(ParseError::BadEncoding);
            }
            let hi = hex_value(bytes[i + 1])?;
            let lo = hex_value(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ParseError::BadEncoding)
}

#[inline]
#[allow(dead_code)] // removed in Task 4 (transitively used by `percent_encode`)
fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => unreachable!("hex_nibble: caller masked to 4 bits"),
    }
}

#[inline]
#[allow(dead_code)] // removed in Task 5 (transitively used by `percent_decode`)
fn hex_value(b: u8) -> Result<u8, ParseError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(ParseError::BadEncoding),
    }
}
```

**Why `pub(crate)`:** the encode/decode helpers are internal; making them `pub(crate)` lets the test module call them but keeps them out of the public API surface.

**Why `#[allow(dead_code)]` (with cleanup notes):** at this point in the plan, no production code in `lib` calls these helpers — only the tests do. `cargo clippy --all-targets -- -D warnings` builds the lib target separately, which sees them as unreachable from anything `pub`/used. The `#[allow]` attributes silence the warning for now; each one notes the task that should remove it as a real caller appears. This keeps each commit individually clippy-clean (matters once the CI workflow lands in Task 16).

- [ ] **Step 3: Run tests — pass**

```bash
cargo test -p vibeflow-protocol
```

Expected: `test result: ok. 10 passed; 0 failed`.

- [ ] **Step 4: Commit**

```bash
git add crates/vibeflow-protocol/src/lib.rs
git commit -m "feat(protocol): add percent encode/decode helpers"
```

---

## Task 4: `Frame::to_bytes` (TDD)

**Files:**
- Modify: `crates/vibeflow-protocol/src/lib.rs`

- [ ] **Step 1: Add tests**

Append to `mod tests`:

```rust
    #[test]
    fn to_bytes_minimal_frame_is_state_only() {
        let bytes = Frame::new(State::Waiting).to_bytes();
        assert_eq!(bytes, b"\x1b]1338;state=waiting\x07");
    }

    #[test]
    fn to_bytes_with_tool_and_project() {
        let bytes = Frame::new(State::Working)
            .with_tool("claude")
            .with_project("vibeflow")
            .to_bytes();
        assert_eq!(
            bytes,
            b"\x1b]1338;state=working;tool=claude;project=vibeflow\x07"
        );
    }

    #[test]
    fn to_bytes_percent_encodes_special_characters_in_values() {
        let bytes = Frame::new(State::Active)
            .with_tool("a;b=c")
            .to_bytes();
        assert_eq!(bytes, b"\x1b]1338;state=active;tool=a%3Bb%3Dc\x07");
    }
```

Run `cargo test -p vibeflow-protocol`. Expected: compile error — `to_bytes` not found on `Frame`.

- [ ] **Step 2: Add wire-format constants and `to_bytes`**

Insert near the top of `lib.rs`, after the doc-comment header:

```rust
/// `ESC` byte (start of an OSC sequence).
pub const ESC: u8 = 0x1B;
/// `BEL` byte (one of two valid OSC terminators).
pub const BEL: u8 = 0x07;
/// String-terminator (the second valid terminator) is `ESC \` — two bytes.
pub const ST: [u8; 2] = [ESC, b'\\'];
/// OSC 1338 sequences over this length are dropped on the floor.
pub const MAX_FRAME_LEN: usize = 4096;
/// The OSC identifier we own.
pub const OSC_ID: &str = "1338";
```

Then add to the `impl Frame` block:

```rust
    /// Serialise this frame as the bytes of an OSC 1338 sequence terminated by `BEL`.
    ///
    /// (BEL terminator chosen over ST because it's simpler and is what xterm/iTerm/most
    /// terminals emit themselves. Either is acceptable per the spec.)
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = String::with_capacity(64);
        s.push(ESC as char);
        s.push(']');
        s.push_str(OSC_ID);
        s.push(';');
        s.push_str("state=");
        s.push_str(self.state.as_str());
        if let Some(tool) = &self.tool {
            s.push_str(";tool=");
            s.push_str(&percent_encode(tool));
        }
        if let Some(project) = &self.project {
            s.push_str(";project=");
            s.push_str(&percent_encode(project));
        }
        s.push(BEL as char);
        s.into_bytes()
    }
```

**Also in Step 2:** now that `to_bytes` calls `percent_encode` (which transitively uses `needs_encoding` and `hex_nibble`), the `#[allow(dead_code)]` attributes on those three items added in Task 3 are no longer needed. Remove them — delete each `#[allow(dead_code)]` line from `percent_encode`, `needs_encoding`, and `hex_nibble` (keep the lines on `percent_decode` and `hex_value` — those become live in Task 5). After removal, the relevant signatures should look like:

```rust
#[inline]
fn needs_encoding(b: u8) -> bool { ... }

#[must_use]
pub(crate) fn percent_encode(s: &str) -> String { ... }

#[inline]
fn hex_nibble(n: u8) -> char { ... }
```

- [ ] **Step 3: Run tests — pass**

```bash
cargo test -p vibeflow-protocol
```

Expected: `test result: ok. 13 passed; 0 failed`.

- [ ] **Step 4: Commit**

```bash
git add crates/vibeflow-protocol/src/lib.rs
git commit -m "feat(protocol): add Frame::to_bytes and wire-format constants"
```

---

## Task 5: `parse` (TDD)

**Files:**
- Modify: `crates/vibeflow-protocol/src/lib.rs`

This is the most subtle piece. The parser accepts a complete OSC 1338 frame (caller's responsibility — Stage 2's `OscDispatcher` handles streaming). It supports both `BEL` and `ST` terminators, requires `state`, ignores unknown keys for forward compatibility, and never panics.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests`:

```rust
    #[test]
    fn parse_minimal_bel_terminated() {
        let f = parse(b"\x1b]1338;state=waiting\x07").unwrap();
        assert_eq!(f, Frame::new(State::Waiting));
    }

    #[test]
    fn parse_minimal_st_terminated() {
        let f = parse(b"\x1b]1338;state=active\x1b\\").unwrap();
        assert_eq!(f, Frame::new(State::Active));
    }

    #[test]
    fn parse_full_frame_with_all_keys() {
        let f = parse(b"\x1b]1338;state=working;tool=claude;project=vibeflow\x07").unwrap();
        assert_eq!(
            f,
            Frame::new(State::Working).with_tool("claude").with_project("vibeflow")
        );
    }

    #[test]
    fn parse_decodes_percent_escapes_in_values() {
        let f = parse(b"\x1b]1338;state=active;tool=a%3Bb%3Dc\x07").unwrap();
        assert_eq!(f, Frame::new(State::Active).with_tool("a;b=c"));
    }

    #[test]
    fn parse_ignores_unknown_keys_for_forward_compat() {
        let f = parse(b"\x1b]1338;state=waiting;newfield=hello;tool=claude\x07").unwrap();
        assert_eq!(f, Frame::new(State::Waiting).with_tool("claude"));
    }

    #[test]
    fn parse_rejects_wrong_prefix() {
        assert_eq!(parse(b"hello\x07"), Err(ParseError::NotOurOsc));
        assert_eq!(parse(b"\x1b]133;state=waiting\x07"), Err(ParseError::NotOurOsc));
    }

    #[test]
    fn parse_requires_state_key() {
        assert_eq!(parse(b"\x1b]1338;tool=claude\x07"), Err(ParseError::MissingState));
    }

    #[test]
    fn parse_rejects_unknown_state_value() {
        match parse(b"\x1b]1338;state=zonking\x07") {
            Err(ParseError::UnknownState(ref s)) if s == "zonking" => {}
            other => panic!("expected UnknownState, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_missing_terminator() {
        assert!(matches!(
            parse(b"\x1b]1338;state=waiting"),
            Err(ParseError::Malformed(_))
        ));
    }

    #[test]
    fn parse_rejects_oversized_input() {
        let mut big = Vec::with_capacity(MAX_FRAME_LEN + 100);
        big.extend_from_slice(b"\x1b]1338;state=waiting;tool=");
        big.extend(std::iter::repeat(b'x').take(MAX_FRAME_LEN));
        big.push(BEL);
        assert_eq!(parse(&big), Err(ParseError::TooLong));
    }
```

Run `cargo test -p vibeflow-protocol`. Expected: compile error — `parse` not found.

- [ ] **Step 2: Implement `parse`**

Insert above `mod tests`:

```rust
/// Parse a complete OSC 1338 frame from the byte slice.
///
/// The caller is responsible for delivering exactly one framed sequence — the
/// streaming `OscDispatcher` in the vibeflow binary slices bytes between
/// `ESC ]` and the next `BEL` / `ST` terminator before calling this.
///
/// # Errors
/// See [`ParseError`].
pub fn parse(bytes: &[u8]) -> Result<Frame, ParseError> {
    if bytes.len() > MAX_FRAME_LEN {
        return Err(ParseError::TooLong);
    }

    // Strip the OSC introducer: `ESC ]`.
    let rest = bytes
        .strip_prefix(&[ESC, b']'])
        .ok_or(ParseError::NotOurOsc)?;

    // Find and strip the terminator (BEL or ST).
    let body = strip_terminator(rest)?;

    // The body is `1338;k1=v1;k2=v2…` — must be valid UTF-8 (per spec).
    let body = std::str::from_utf8(body).map_err(|_| ParseError::Malformed("non-UTF-8 body"))?;

    let mut parts = body.split(';');
    let id = parts.next().ok_or(ParseError::Malformed("empty body"))?;
    if id != OSC_ID {
        return Err(ParseError::NotOurOsc);
    }

    let mut state: Option<State> = None;
    let mut tool: Option<String> = None;
    let mut project: Option<String> = None;

    for part in parts {
        // Split on the *first* `=` only — values may contain `=` if percent-encoded
        // would have escaped it, but a literal `=` in a malformed frame should
        // still parse cleanly to "key" + "value-with-equals".
        let Some((key, value)) = part.split_once('=') else { continue };
        match key {
            "state" => {
                let decoded = percent_decode(value)?;
                state = Some(decoded.parse()?);
            }
            "tool" => tool = Some(percent_decode(value)?),
            "project" => project = Some(percent_decode(value)?),
            _ => { /* unknown key — ignore for forward compatibility */ }
        }
    }

    let state = state.ok_or(ParseError::MissingState)?;
    Ok(Frame { state, tool, project })
}

/// Locate either `BEL` or `ESC \` and return the body slice (everything before it).
fn strip_terminator(rest: &[u8]) -> Result<&[u8], ParseError> {
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            BEL => return Ok(&rest[..i]),
            b if b == ESC && rest.get(i + 1).copied() == Some(b'\\') => {
                return Ok(&rest[..i]);
            }
            _ => i += 1,
        }
    }
    Err(ParseError::Malformed("no terminator"))
}
```

**Why `let Some((key, value)) = part.split_once('=') else { continue };`:** `let-else` is the idiomatic Rust way to early-return from a pattern match without a giant `if let` block. **Why `?` after `parse()`:** `decoded.parse::<State>()` returns `Result<State, ParseError>` because we set `type Err = ParseError` on the `FromStr` impl, so `?` propagates a `ParseError::UnknownState` up.

**Also in Step 2:** now that `parse` calls `percent_decode` (which transitively uses `hex_value`), the `#[allow(dead_code)]` attributes on those two items added in Task 3 are no longer needed. Delete the `#[allow(dead_code)]` line above each. After removal:

```rust
pub(crate) fn percent_decode(s: &str) -> Result<String, ParseError> { ... }

#[inline]
fn hex_value(b: u8) -> Result<u8, ParseError> { ... }
```

This makes the lib clippy-clean with no `#[allow]` attributes left over from Task 3.

- [ ] **Step 3: Run tests — pass**

```bash
cargo test -p vibeflow-protocol
```

Expected: `test result: ok. 23 passed; 0 failed`.

- [ ] **Step 4: Commit**

```bash
git add crates/vibeflow-protocol/src/lib.rs
git commit -m "feat(protocol): add parse for OSC 1338 frames"
```

---

## Task 6: Round-trip property test

**Files:**
- Modify: `crates/vibeflow-protocol/src/lib.rs`

A property test asserts: for *any* well-formed `Frame`, `parse(frame.to_bytes()) == Ok(frame)`. This catches encode/decode asymmetries we'd never think to hand-write.

- [ ] **Step 1: Add the property test**

Append to `mod tests`:

```rust
    use proptest::prelude::*;

    fn arb_state() -> impl Strategy<Value = State> {
        prop_oneof![
            Just(State::Active),
            Just(State::Working),
            Just(State::Waiting),
            Just(State::Done),
        ]
    }

    // Any UTF-8 string up to 100 chars (any non-newline scalar — proptest's
    // regex `.` excludes \n by default, which is fine; we cover `\t` etc.
    // explicitly in the unit tests).
    //
    // Why `string_regex` not a bare `".{0,100}"`: a `&str` literal does NOT
    // implement `Strategy<Value=String>`. The proptest! macro accepts string
    // literals after `in` because the macro converts them to a regex strategy
    // — but in plain function-form code we have to call `string_regex`
    // explicitly. The `.unwrap()` is fine because the regex is a literal.
    //
    // Why 100: worst-case encoding is 4 UTF-8 bytes per char × 3 chars per
    // percent-encoded byte = 12 chars per char in the wire form, so two such
    // values plus the rest of the frame fit comfortably under MAX_FRAME_LEN.
    fn arb_value() -> impl Strategy<Value = String> {
        proptest::string::string_regex(".{0,100}").unwrap()
    }

    fn arb_frame() -> impl Strategy<Value = Frame> {
        (arb_state(), proptest::option::of(arb_value()), proptest::option::of(arb_value())).prop_map(
            |(state, tool, project)| Frame { state, tool, project },
        )
    }

    proptest! {
        #[test]
        fn frame_to_bytes_then_parse_roundtrips(frame in arb_frame()) {
            let bytes = frame.to_bytes();
            let parsed = parse(&bytes).expect("round-trip should always parse");
            prop_assert_eq!(parsed, frame);
        }
    }
```

- [ ] **Step 2: Run tests — pass**

```bash
cargo test -p vibeflow-protocol
```

Expected: all previous tests still pass plus `frame_to_bytes_then_parse_roundtrips` passes 256 cases (proptest default).

If a test fails: proptest reports the minimal failing input. That's a real bug in encode or decode — fix it before continuing.

- [ ] **Step 3: Commit**

```bash
git add crates/vibeflow-protocol/Cargo.toml crates/vibeflow-protocol/src/lib.rs
git commit -m "test(protocol): add round-trip property test for Frame"
```

---

## Task 7: `emit` and `emit_state` — stdout writers

**Files:**
- Modify: `crates/vibeflow-protocol/src/lib.rs`

- [ ] **Step 1: Write the test**

Append to `mod tests`:

```rust
    #[test]
    fn emit_writes_to_provided_writer() {
        // emit_to is the seam we test against; emit() and emit_state() wrap it.
        let mut buf = Vec::<u8>::new();
        let f = Frame::new(State::Working).with_tool("claude");
        emit_to(&mut buf, &f).expect("write should succeed");
        assert_eq!(buf, b"\x1b]1338;state=working;tool=claude\x07");
    }
```

Run `cargo test -p vibeflow-protocol`. Expected: compile error — `emit_to` not found.

- [ ] **Step 2: Add `emit_to`, `emit`, `emit_state`**

First, add a `use std::io::Write;` line right below the `//!` doc-comment header at the top of `lib.rs`. Without `Write` in scope, the trait methods `write_all` (in `emit_to`) and `flush` (in `emit`) won't resolve.

```rust
//! OSC 1338 protocol — vibeflow's open standard for AI-tool state signalling.
//!
//! See `docs/protocol.md` in the workspace root for the canonical wire-format spec.

use std::io::Write;
```

Then insert above `mod tests`:

```rust
/// Write the OSC 1338 byte sequence for `frame` to `writer`. Use this when you
/// need to write somewhere other than stdout (tests, files, sockets).
///
/// # Errors
/// Propagates any [`std::io::Error`] from the underlying writer.
pub fn emit_to<W: Write>(writer: &mut W, frame: &Frame) -> std::io::Result<()> {
    writer.write_all(&frame.to_bytes())
}

/// Write `frame` to stdout and flush.
///
/// # Errors
/// Returns the underlying I/O error if stdout cannot be written.
pub fn emit(frame: &Frame) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    emit_to(&mut handle, frame)?;
    handle.flush()
}

/// Convenience for `emit(&Frame::new(state))`.
///
/// # Errors
/// Returns the underlying I/O error if stdout cannot be written.
pub fn emit_state(state: State) -> std::io::Result<()> {
    emit(&Frame::new(state))
}
```

**Why a separate `emit_to`:** depending directly on `std::io::stdout()` makes `emit` un-testable. Splitting out a writer-generic seam is the standard Rust pattern; the public `emit` becomes a thin wrapper. **Why `lock()` then `flush()`:** locking stdout once for the whole operation is faster and prevents interleaving with other writers. Flushing matters because OSC sequences are short — without flush, a buffered stdout might not deliver the bytes until much later, breaking the snappy "tab updates as soon as Claude says it's waiting" feel.

- [ ] **Step 3: Run tests — pass**

```bash
cargo test -p vibeflow-protocol
```

Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add crates/vibeflow-protocol/src/lib.rs
git commit -m "feat(protocol): add emit/emit_state/emit_to writers"
```

---

## Task 8: `vibeflow-emit` CLI binary

**Files:**
- Create: `crates/vibeflow-protocol/src/bin/vibeflow-emit.rs`
- Modify: `crates/vibeflow-protocol/Cargo.toml`

- [ ] **Step 1: Write the binary**

Create `crates/vibeflow-protocol/src/bin/vibeflow-emit.rs`:

```rust
//! `vibeflow-emit` — tiny CLI for emitting one OSC 1338 frame to stdout.
//!
//! Usage:
//!     vibeflow-emit <state> [--tool=<name>] [--project=<name>]
//!
//! `<state>` is one of: active, working, waiting, done.

use std::process::ExitCode;
use vibeflow_protocol::{emit, Frame, State};

fn print_usage(out: &mut impl std::io::Write) {
    let _ = writeln!(
        out,
        "usage: vibeflow-emit <state> [--tool=<name>] [--project=<name>]\n\
         \n\
         <state>: one of active, working, waiting, done\n\
         \n\
         examples:\n  \
         vibeflow-emit waiting --tool=claude\n  \
         vibeflow-emit working --tool=codex --project=vibeflow"
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help") {
        let mut out = std::io::stderr();
        print_usage(&mut out);
        return if args.is_empty() { ExitCode::from(2) } else { ExitCode::SUCCESS };
    }

    let state: State = match args[0].parse() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("vibeflow-emit: unknown state {:?}", args[0]);
            print_usage(&mut std::io::stderr());
            return ExitCode::from(2);
        }
    };

    let mut frame = Frame::new(state);
    for arg in &args[1..] {
        if let Some(v) = arg.strip_prefix("--tool=") {
            frame.tool = Some(v.to_owned());
        } else if let Some(v) = arg.strip_prefix("--project=") {
            frame.project = Some(v.to_owned());
        } else {
            eprintln!("vibeflow-emit: unexpected argument {arg:?}");
            print_usage(&mut std::io::stderr());
            return ExitCode::from(2);
        }
    }

    if let Err(e) = emit(&frame) {
        eprintln!("vibeflow-emit: write failed: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
```

- [ ] **Step 2: Add the `[[bin]]` block to the manifest**

Edit `crates/vibeflow-protocol/Cargo.toml`. Replace the comment placeholder

```toml
# The vibeflow-emit binary lands in Task 8; its [[bin]] entry is added then.
```

with:

```toml
[[bin]]
name = "vibeflow-emit"
path = "src/bin/vibeflow-emit.rs"
```

- [ ] **Step 3: Build and run**

```bash
cd /path/to/vibeflow
cargo build -p vibeflow-protocol --bin vibeflow-emit
./target/debug/vibeflow-emit waiting --tool=claude | xxd | head
```

Expected output (the actual bytes of the emitted frame):

```
00000000: 1b5d 3133 3338 3b73 7461 7465 3d77 6169  .]1338;state=wai
00000010: 7469 6e67 3b74 6f6f 6c3d 636c 6175 6465  ting;tool=claude
00000020: 07                                       .
```

Try the error paths:

```bash
./target/debug/vibeflow-emit                  # no args
./target/debug/vibeflow-emit zonk             # bad state
./target/debug/vibeflow-emit --help
```

Expected: usage text on stderr; exit codes 2, 2, 0.

- [ ] **Step 4: Commit**

```bash
git add crates/vibeflow-protocol/Cargo.toml crates/vibeflow-protocol/src/bin/vibeflow-emit.rs
git commit -m "feat(protocol): add vibeflow-emit CLI binary"
```

---

## Task 9: Fuzz harness for `parse`

**Files:**
- Create: `crates/vibeflow-protocol/fuzz/Cargo.toml`
- Create: `crates/vibeflow-protocol/fuzz/fuzz_targets/parse.rs`
- Create: `crates/vibeflow-protocol/fuzz/.gitignore`

`cargo-fuzz` requires nightly Rust. Install once:

```bash
cargo install cargo-fuzz
rustup toolchain install nightly
```

The fuzz crate is excluded from the parent workspace (Task 0 step 2) so stable users never need to install nightly.

- [ ] **Step 1: Initialise the fuzz crate manifest**

Write `crates/vibeflow-protocol/fuzz/Cargo.toml`:

```toml
[workspace]

[package]
name = "vibeflow-protocol-fuzz"
version = "0.0.0"
publish = false
edition = "2021"

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"

[dependencies.vibeflow-protocol]
path = ".."

[[bin]]
name = "parse"
path = "fuzz_targets/parse.rs"
test = false
doc = false
bench = false
```

**Why the empty `[workspace]` table:** marks this crate as its own workspace root, so cargo doesn't try to associate it with the parent workspace at `../../..`. The parent workspace already excludes `crates/vibeflow-protocol/fuzz` via its `exclude = […]` list, but cargo's discovery still walks up looking for a workspace, and if it finds the parent's `Cargo.toml`, it errors with "current package believes it's in a workspace when it's not". This empty `[workspace]` short-circuits that walk. (`cargo fuzz init` does the same thing automatically.)

- [ ] **Step 2: Write the fuzz target**

Create `crates/vibeflow-protocol/fuzz/fuzz_targets/parse.rs`:

```rust
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Goal: never panic, never OOM, regardless of input.
    let _ = vibeflow_protocol::parse(data);
});
```

- [ ] **Step 3: Add fuzz `.gitignore`**

Create `crates/vibeflow-protocol/fuzz/.gitignore`:

```gitignore
target/
corpus/
artifacts/
coverage/
```

- [ ] **Step 4: Run the fuzzer for 60 seconds**

```bash
cd /path/to/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: many iterations, "Done 60 seconds: ..." line, no crashes / no panics. If it finds a panic, it stops with a reproducer in `fuzz/artifacts/parse/` — that's a real bug; reproduce with `cargo +nightly fuzz run parse fuzz/artifacts/parse/<file>`, then fix.

- [ ] **Step 5: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow-protocol/fuzz/Cargo.toml crates/vibeflow-protocol/fuzz/fuzz_targets/parse.rs crates/vibeflow-protocol/fuzz/.gitignore
git commit -m "test(protocol): add cargo-fuzz harness for parse"
```

---

## Task 10: npm package — skeleton + types

**Files:**
- Create: `bindings/npm/package.json`
- Create: `bindings/npm/tsconfig.json`
- Create: `bindings/npm/src/index.ts`
- Create: `bindings/npm/.gitignore`
- Create: `bindings/npm/.npmignore`
- Create: `bindings/npm/README.md`

- [ ] **Step 1: Write `package.json`**

Create `bindings/npm/package.json`:

```json
{
  "name": "@vibeflow/protocol",
  "version": "0.1.0",
  "description": "OSC 1338 protocol — open standard for AI-tool state signalling in terminals (TypeScript binding)",
  "license": "MIT OR Apache-2.0",
  "author": "Brian Hengen <bhengen@gmail.com>",
  "homepage": "https://github.com/bjhengen/vibeflow",
  "repository": {
    "type": "git",
    "url": "https://github.com/bjhengen/vibeflow.git",
    "directory": "bindings/npm"
  },
  "main": "./dist/src/index.js",
  "types": "./dist/src/index.d.ts",
  "exports": {
    ".": {
      "types": "./dist/src/index.d.ts",
      "default": "./dist/src/index.js"
    }
  },
  "files": [
    "dist/src",
    "README.md"
  ],
  "engines": {
    "node": ">=18"
  },
  "scripts": {
    "build": "tsc",
    "test": "tsc && node --test ./dist/test/index.test.js",
    "clean": "rm -rf dist"
  },
  "devDependencies": {
    "typescript": "^5.4.0",
    "@types/node": "^20.0.0"
  },
  "keywords": ["terminal", "osc", "ai", "vibeflow"]
}
```

**Why CommonJS (no `"type": "module"`):** Node's ESM under TypeScript requires `.js` import extensions in source files (despite the file actually being `.ts`), which is unintuitive boilerplate. CJS lets us write `import { x } from "../src/index"` and have it Just Work. The `default` field in `exports` is the CJS entry; we can flip to ESM later if a consumer needs it. **Why `dist/src` not just `dist` in the `files` array:** tsc with `rootDir: "./"` outputs to `dist/src/...` and `dist/test/...`; we don't ship the test directory.

- [ ] **Step 2: Write `tsconfig.json`**

Create `bindings/npm/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "CommonJS",
    "moduleResolution": "node",
    "outDir": "./dist",
    "rootDir": "./",
    "declaration": true,
    "declarationMap": true,
    "sourceMap": true,
    "strict": true,
    "noImplicitAny": true,
    "noUncheckedIndexedAccess": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "lib": ["ES2022"],
    "types": ["node"]
  },
  "include": ["src/**/*", "test/**/*"],
  "exclude": ["dist", "node_modules"]
}
```

- [ ] **Step 3: Write the type declarations**

Create `bindings/npm/src/index.ts`:

```typescript
/**
 * @vibeflow/protocol — OSC 1338 protocol binding for TypeScript.
 *
 * See https://github.com/bjhengen/vibeflow/blob/main/docs/protocol.md
 * for the canonical wire-format spec.
 */

export type State = "active" | "working" | "waiting" | "done";

export interface Frame {
  state: State;
  tool?: string;
  project?: string;
}

const ESC = "\x1b";
const BEL = "\x07";

/** Sequences over this length are rejected by `parse`. */
export const MAX_FRAME_LEN = 4096;
/** The OSC identifier this binding owns. */
export const OSC_ID = "1338";
```

- [ ] **Step 4: Add `.gitignore` and `.npmignore`**

Create `bindings/npm/.gitignore`:

```gitignore
node_modules/
dist/
*.tsbuildinfo
```

Create `bindings/npm/.npmignore`:

```gitignore
src/
test/
tsconfig.json
*.tsbuildinfo
.gitignore
node_modules/
```

- [ ] **Step 5: Add a stub README (filled out fully in Task 13)**

Create `bindings/npm/README.md`:

```markdown
# @vibeflow/protocol

TypeScript binding for the OSC 1338 protocol — vibeflow's open standard for AI-tool state signalling in terminals.

(See `docs/protocol.md` in the repository root for the canonical wire-format spec. Detailed npm-side usage docs land in Task 13 of the implementation plan.)
```

- [ ] **Step 6: Install + build to verify**

```bash
cd /path/to/vibeflow/bindings/npm
npm install
npm run build
ls -la dist/src/
```

Expected: `dist/src/` contains `index.js`, `index.d.ts`, `index.js.map`, `index.d.ts.map`. No errors. (Tests aren't in the tree yet, so `dist/test/` doesn't exist.)

- [ ] **Step 7: Commit**

```bash
cd /path/to/vibeflow
git add bindings/npm/package.json bindings/npm/tsconfig.json bindings/npm/src/index.ts bindings/npm/.gitignore bindings/npm/.npmignore bindings/npm/README.md
# package-lock.json IS committed for libraries with binaries / scripts:
git add bindings/npm/package-lock.json
git commit -m "chore(npm): scaffold @vibeflow/protocol package"
```

---

## Task 11: npm package — `toBytes`, `emit`, `emitState`

**Files:**
- Modify: `bindings/npm/src/index.ts`
- Create: `bindings/npm/test/index.test.ts`

- [ ] **Step 1: Add the failing test for `toBytes`**

Create `bindings/npm/test/index.test.ts`:

```typescript
import { test } from "node:test";
import assert from "node:assert/strict";

import { Frame, toBytes } from "../src/index";

test("toBytes: minimal frame is state only", () => {
  const f: Frame = { state: "waiting" };
  assert.equal(toBytes(f), "\x1b]1338;state=waiting\x07");
});

test("toBytes: full frame with tool and project", () => {
  const f: Frame = { state: "working", tool: "claude", project: "vibeflow" };
  assert.equal(
    toBytes(f),
    "\x1b]1338;state=working;tool=claude;project=vibeflow\x07",
  );
});

test("toBytes: percent-encodes specials in values", () => {
  const f: Frame = { state: "active", tool: "a;b=c" };
  assert.equal(toBytes(f), "\x1b]1338;state=active;tool=a%3Bb%3Dc\x07");
});

test("toBytes: percent-encodes non-ASCII as UTF-8 bytes", () => {
  const f: Frame = { state: "active", tool: "café" };
  assert.equal(toBytes(f), "\x1b]1338;state=active;tool=caf%C3%A9\x07");
});
```

Run:

```bash
cd /path/to/vibeflow/bindings/npm
npm test
```

Expected: TS compile errors — `toBytes` not exported.

(Adjust `tsconfig.json` if the `test` script fails to find tests under the rootDir — the `test/` path is included via the `include` glob. If `node --test` doesn't find compiled `.js` files under `dist/test/`, add `dist/test/index.test.js` to the script path explicitly. Build first, then test.)

- [ ] **Step 2: Implement `toBytes`, `emit`, `emitState`, plus internal encode/decode**

Append to `bindings/npm/src/index.ts`:

```typescript
const NEEDS_ENCODING = (b: number): boolean =>
  b < 0x20 ||
  b === 0x7f ||
  b === 0x3b /* ; */ ||
  b === 0x3d /* = */ ||
  b === 0x25 /* % */ ||
  b > 0x7f;

/** Internal helper — exported for tests / advanced use. */
export function percentEncode(s: string): string {
  const bytes = new TextEncoder().encode(s);
  let out = "";
  for (const b of bytes) {
    if (NEEDS_ENCODING(b)) {
      out += "%" + b.toString(16).toUpperCase().padStart(2, "0");
    } else {
      out += String.fromCharCode(b);
    }
  }
  return out;
}

/** Internal helper — exported for tests / advanced use. */
export function percentDecode(s: string): string {
  // Operate on UTF-8 bytes, not UTF-16 code units. Two reasons:
  // (1) `s.charCodeAt(i)` returns 0..65535 — Uint8Array truncates past 255,
  //     so any non-ASCII char that slipped through unencoded would be mangled.
  // (2) The Rust reference parser treats unencoded non-ASCII as byte-passthrough;
  //     this matches that behaviour.
  const inputBytes = new TextEncoder().encode(s);
  const out: number[] = [];
  let i = 0;
  while (i < inputBytes.length) {
    if (inputBytes[i] === 0x25 /* % */) {
      if (i + 2 >= inputBytes.length) {
        throw new Error("vibeflow-protocol: bad percent encoding");
      }
      const hex = String.fromCharCode(inputBytes[i + 1]!, inputBytes[i + 2]!);
      if (!/^[0-9a-fA-F]{2}$/.test(hex)) {
        throw new Error("vibeflow-protocol: bad percent encoding");
      }
      out.push(parseInt(hex, 16));
      i += 3;
    } else {
      out.push(inputBytes[i]!);
      i += 1;
    }
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(out));
  } catch {
    throw new Error("vibeflow-protocol: invalid UTF-8 after percent-decode");
  }
}

/**
 * Serialise a frame into its OSC 1338 byte sequence (BEL-terminated).
 * Returns a string — write it to stdout (or any byte sink) verbatim.
 */
export function toBytes(frame: Frame): string {
  let s = `${ESC}]${OSC_ID};state=${frame.state}`;
  if (frame.tool != null) s += `;tool=${percentEncode(frame.tool)}`;
  if (frame.project != null) s += `;project=${percentEncode(frame.project)}`;
  s += BEL;
  return s;
}

/** Write `frame`'s OSC 1338 sequence to `process.stdout`. */
export function emit(frame: Frame): void {
  process.stdout.write(toBytes(frame));
}

/** Convenience for `emit({ state })`. */
export function emitState(state: State): void {
  emit({ state });
}
```

- [ ] **Step 3: Run tests — pass**

```bash
npm test
```

Expected: `tests 4`, `pass 4`, `fail 0`.

- [ ] **Step 4: Commit**

```bash
cd /path/to/vibeflow
git add bindings/npm/src/index.ts bindings/npm/test/index.test.ts
git commit -m "feat(npm): add toBytes, emit, emitState"
```

---

## Task 12: npm package — `parse`

**Files:**
- Modify: `bindings/npm/src/index.ts`
- Modify: `bindings/npm/test/index.test.ts`

- [ ] **Step 1: Add failing tests**

First, update the import line at the top of `bindings/npm/test/index.test.ts` to add `MAX_FRAME_LEN` and `parse`. The line currently reads:

```typescript
import { Frame, toBytes } from "../src/index";
```

Change it to:

```typescript
import { Frame, MAX_FRAME_LEN, parse, toBytes } from "../src/index";
```

Then append these tests to the end of the same file:

```typescript
test("parse: minimal BEL-terminated frame", () => {
  const f = parse("\x1b]1338;state=waiting\x07");
  assert.deepEqual(f, { state: "waiting" });
});

test("parse: minimal ST-terminated frame", () => {
  const f = parse("\x1b]1338;state=active\x1b\\");
  assert.deepEqual(f, { state: "active" });
});

test("parse: full frame with all keys", () => {
  const f = parse("\x1b]1338;state=working;tool=claude;project=vibeflow\x07");
  assert.deepEqual(f, { state: "working", tool: "claude", project: "vibeflow" });
});

test("parse: decodes percent-encoded values", () => {
  const f = parse("\x1b]1338;state=active;tool=a%3Bb%3Dc\x07");
  assert.deepEqual(f, { state: "active", tool: "a;b=c" });
});

test("parse: ignores unknown keys", () => {
  const f = parse("\x1b]1338;state=waiting;newkey=hello;tool=claude\x07");
  assert.deepEqual(f, { state: "waiting", tool: "claude" });
});

test("parse: rejects wrong prefix", () => {
  assert.throws(() => parse("hello\x07"), /not an OSC/);
  assert.throws(() => parse("\x1b]133;state=waiting\x07"), /not OSC 1338/);
});

test("parse: requires state key", () => {
  assert.throws(() => parse("\x1b]1338;tool=claude\x07"), /missing state/);
});

test("parse: rejects unknown state value", () => {
  assert.throws(() => parse("\x1b]1338;state=zonking\x07"), /unknown state/);
});

test("parse: rejects oversize input", () => {
  const big = "\x1b]1338;state=waiting;tool=" + "x".repeat(MAX_FRAME_LEN) + "\x07";
  assert.throws(() => parse(big), /too long/);
});

test("parse: rejects missing terminator", () => {
  assert.throws(() => parse("\x1b]1338;state=waiting"), /no terminator/);
});

test("round-trip: any well-formed frame survives toBytes → parse", () => {
  const frames: Frame[] = [
    { state: "active" },
    { state: "working", tool: "claude" },
    { state: "waiting", tool: "claude", project: "vibeflow" },
    { state: "done", tool: "a;b=c", project: "x=y" },
    { state: "working", tool: "café" },
  ];
  for (const f of frames) {
    assert.deepEqual(parse(toBytes(f)), f);
  }
});
```

Run `npm test`. Expected: TS errors — `parse` not exported / `MAX_FRAME_LEN` cannot be found at runtime, depending on how far compilation gets.

- [ ] **Step 2: Implement `parse`**

Append to `bindings/npm/src/index.ts`:

```typescript
/**
 * Parse a complete OSC 1338 frame. Caller is responsible for delivering
 * exactly one framed sequence (an in-terminal stream parser would chunk
 * between `ESC ]` and the next `BEL` / `ESC \`).
 *
 * Throws on any malformed input.
 */
export function parse(input: string): Frame {
  // Match Rust's byte-based 4 KiB cap: `input.length` is UTF-16 code units,
  // which under-reports byte count for non-ASCII inputs.
  if (new TextEncoder().encode(input).length > MAX_FRAME_LEN) {
    throw new Error("vibeflow-protocol: frame too long");
  }
  if (!input.startsWith(`${ESC}]`)) {
    throw new Error("vibeflow-protocol: not an OSC sequence");
  }
  let body = input.slice(2);

  // Find terminator: BEL or ESC \.
  let bodyEnd = -1;
  for (let i = 0; i < body.length; i++) {
    if (body[i] === BEL) {
      bodyEnd = i;
      break;
    }
    if (body[i] === ESC && body[i + 1] === "\\") {
      bodyEnd = i;
      break;
    }
  }
  if (bodyEnd < 0) {
    throw new Error("vibeflow-protocol: no terminator");
  }
  body = body.slice(0, bodyEnd);

  const parts = body.split(";");
  if (parts[0] !== OSC_ID) {
    throw new Error("vibeflow-protocol: not OSC 1338");
  }

  const result: Partial<Frame> = {};
  for (let i = 1; i < parts.length; i++) {
    const part = parts[i] ?? "";
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    const key = part.slice(0, eq);
    const value = part.slice(eq + 1);
    switch (key) {
      case "state": {
        const decoded = percentDecode(value);
        if (
          decoded !== "active" &&
          decoded !== "working" &&
          decoded !== "waiting" &&
          decoded !== "done"
        ) {
          throw new Error(`vibeflow-protocol: unknown state ${JSON.stringify(decoded)}`);
        }
        result.state = decoded;
        break;
      }
      case "tool":
        result.tool = percentDecode(value);
        break;
      case "project":
        result.project = percentDecode(value);
        break;
      // unknown keys: forward-compat ignore
    }
  }

  if (!result.state) {
    throw new Error("vibeflow-protocol: missing state");
  }
  // Build the result without `tool`/`project` keys when undefined (cleaner deepEqual).
  const out: Frame = { state: result.state };
  if (result.tool !== undefined) out.tool = result.tool;
  if (result.project !== undefined) out.project = result.project;
  return out;
}
```

- [ ] **Step 3: Run tests — pass**

```bash
npm test
```

Expected: `pass 15` total (4 from Task 11 plus 11 here).

- [ ] **Step 4: Commit**

```bash
cd /path/to/vibeflow
git add bindings/npm/src/index.ts bindings/npm/test/index.test.ts
git commit -m "feat(npm): add parse for OSC 1338 frames"
```

---

## Task 13: npm package README (full version)

**Files:**
- Modify: `bindings/npm/README.md`

- [ ] **Step 1: Replace the stub with the full README**

Overwrite `bindings/npm/README.md`:

````markdown
# @vibeflow/protocol

TypeScript binding for the **OSC 1338 protocol** — an open standard for AI-tool state signalling in terminals.

When an AI tool (Claude Code, Codex, Aider, …) emits an OSC 1338 sequence, a compliant terminal (e.g. [vibeflow](https://github.com/bjhengen/vibeflow)) updates that tab's visual indicator: amber pulse for `waiting`, blue for `working`, gray for `idle`. The protocol is open so anything that emits the bytes — a wrapper script, a Node CLI, a hooked LSP — Just Works.

## Install

```bash
npm install @vibeflow/protocol
```

## Quick start

```ts
import { emitState } from "@vibeflow/protocol";

emitState("working");
// ... do work ...
emitState("waiting");
```

## API

```ts
type State = "active" | "working" | "waiting" | "done";

interface Frame {
  state: State;
  tool?: string;       // e.g. "claude", "codex"
  project?: string;    // e.g. "my-app"
}

function toBytes(frame: Frame): string;
function emit(frame: Frame): void;          // writes to process.stdout
function emitState(state: State): void;     // shorthand for emit({ state })
function parse(input: string): Frame;       // throws on malformed input
```

The full wire-format specification lives in [`docs/protocol.md`](https://github.com/bjhengen/vibeflow/blob/main/docs/protocol.md) in the vibeflow repository.

## When emitted bytes do nothing

In any terminal that doesn't recognise OSC 1338, the bytes are silently consumed and produce no output. So it's safe to call `emitState` from a tool that may or may not run inside vibeflow.

## License

Dual-licensed under MIT or Apache-2.0 at your option.
````

- [ ] **Step 2: Commit**

```bash
git add bindings/npm/README.md
git commit -m "docs(npm): write full README for @vibeflow/protocol"
```

---

## Task 14: `docs/protocol.md` — canonical wire-format spec

**Files:**
- Create: `docs/protocol.md`

- [ ] **Step 1: Write the spec document**

Create `/path/to/vibeflow/docs/protocol.md`:

````markdown
# OSC 1338 — vibeflow's AI-tool state signalling protocol

**Status:** stable for v0.1. Additions (new keys, new states) must be backwards compatible — old consumers ignore unknown keys.

**Owner:** vibeflow project. The string `1338` is the OSC identifier this protocol owns; it has no meaning to other terminals.

## Wire format

```
ESC ] 1338 ; key=value [; key=value ]* ( BEL | ST )
```

- `ESC` = `0x1B` (the C0 ESC character).
- `BEL` = `0x07`. `ST` = `ESC \` (string terminator, two bytes).
- Keys: UTF-8, alphanumeric and `_` only.
- Values: UTF-8, percent-encoded if they contain `;`, `=`, `%`, control bytes (< 0x20 or 0x7F), or any non-ASCII byte. Encoding uses uppercase hex (e.g. `%3B` for `;`).
- The full sequence (including `ESC ]`, the `1338`, all keys/values, separators, and the terminator) MUST NOT exceed **4 KiB**. Over-long sequences are dropped on the floor by compliant parsers; parsing resumes at the next `ESC`.
- Unrecognised keys MUST be ignored. This is the forward-compatibility contract.
- Unrecognised values for known keys: implementation-defined. The reference parsers (`vibeflow-protocol`, `@vibeflow/protocol`) currently raise an error for unknown `state` values; vibeflow's dispatcher logs at debug level and ignores the frame.

## Keys (v0.1)

| Key | Required | Purpose |
|---|---|---|
| `state` | yes | One of `active`, `working`, `waiting`, `done`. |
| `tool` | no | Free-form name of the emitting tool (e.g. `claude`, `codex`). Used for tab grouping/display. |
| `project` | no | Free-form name of the project being worked on. Surfaces in tab subtitle when present. |

## States (v0.1)

| State | Visual (in vibeflow) | Meaning |
|---|---|---|
| `active` | no special styling | Default; tool is present but not in any other notable state. |
| `working` | steady blue stripe + tinted subtitle | Tool is running / generating. |
| `waiting` | amber stripe with soft pulse | Tool is waiting for user input. The headline state. |
| `done` | brief flash, returns to `active` | Tool just finished a task; transient. |

## Examples

```
\x1b]1338;state=waiting\x07
\x1b]1338;state=working;tool=claude\x07
\x1b]1338;state=waiting;tool=claude;project=vibeflow\x07
\x1b]1338;state=active;tool=a%3Bb%3Dc\x1b\\
```

## Three-tier integration

vibeflow integrates AI tools at three levels of fidelity, each strictly better than the next:

1. **Native:** the tool calls a binding (this crate, `@vibeflow/protocol`, etc.) directly.
2. **Wrapper:** a thin shim spawned around the tool watches its output and emits OSC 1338 on its behalf.
3. **Heuristic:** vibeflow itself watches process names + output silence and infers state when no explicit signal arrives.

Tier 1 is the goal; tiers 2 and 3 ensure the experience is never broken on day one.

## Reference implementations

- Rust: [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol)
- TypeScript: [`@vibeflow/protocol`](https://www.npmjs.com/package/@vibeflow/protocol)
- Shell helper: `vibeflow-emit` (a binary in the `vibeflow-protocol` crate)

## Versioning

This document is the contract. Breaking changes (renaming or removing existing keys, changing existing state semantics) bump a major-version protocol identifier (`1338` → some future identifier); they will not happen casually. Additive changes (new optional keys, new state values) are guaranteed safe for old consumers.
````

- [ ] **Step 2: Commit**

```bash
git add docs/protocol.md
git commit -m "docs: write canonical OSC 1338 wire-format spec"
```

---

## Task 15: `vibeflow-protocol` crate README

**Files:**
- Create: `crates/vibeflow-protocol/README.md`

- [ ] **Step 1: Write the crate README**

Create `crates/vibeflow-protocol/README.md`:

````markdown
# vibeflow-protocol

Reference implementation of **OSC 1338**, vibeflow's open standard for AI-tool state signalling in terminals.

When an AI tool emits an OSC 1338 sequence, a compliant terminal (e.g. [vibeflow](https://github.com/bjhengen/vibeflow)) updates that tab's visual indicator: amber pulse for `waiting`, blue for `working`. The protocol is open so any tool — Rust, JS, shell, Python via stdout — can participate.

This crate is **zero-dependency** and pure-`std`. It also ships a tiny `vibeflow-emit` binary so anything that can `exec` (shell scripts, hooks) can emit too.

## Install

```toml
[dependencies]
vibeflow-protocol = "0.1"
```

## Quick start

```rust
use vibeflow_protocol::{emit, emit_state, Frame, State};

fn main() -> std::io::Result<()> {
    // Simple:
    emit_state(State::Working)?;

    // With detail:
    emit(&Frame::new(State::Waiting)
        .with_tool("claude")
        .with_project("vibeflow"))?;
    Ok(())
}
```

## Parsing

```rust
use vibeflow_protocol::{parse, State};

fn main() {
    let bytes = b"\x1b]1338;state=waiting;tool=claude\x07";
    let frame = parse(bytes).unwrap();
    assert_eq!(frame.state, State::Waiting);
    assert_eq!(frame.tool.as_deref(), Some("claude"));
}
```

## `vibeflow-emit` (CLI)

```bash
$ cargo install vibeflow-protocol
$ vibeflow-emit waiting --tool=claude
$ vibeflow-emit working --tool=codex --project=vibeflow
```

## Wire format

The complete OSC 1338 specification is in [`docs/protocol.md`](https://github.com/bjhengen/vibeflow/blob/main/docs/protocol.md) in the project repository.

## License

Dual-licensed under MIT or Apache-2.0 at your option.
````

- [ ] **Step 2: Commit**

```bash
git add crates/vibeflow-protocol/README.md
git commit -m "docs(protocol): write crates.io README"
```

---

## Task 16: GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write the workflow**

Create `/path/to/vibeflow/.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"

jobs:
  rust:
    name: Rust (stable)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install stable toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt

      - name: Cargo cache
        uses: Swatinem/rust-cache@v2

      - name: cargo fmt --check
        run: cargo fmt --all -- --check

      - name: cargo clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: cargo build
        run: cargo build --workspace --all-targets

      - name: cargo test
        run: cargo test --workspace --all-targets

      - name: cargo doc
        run: cargo doc --workspace --no-deps
        env:
          RUSTDOCFLAGS: "-D warnings"

  fuzz:
    name: Fuzz smoke (60s)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install nightly toolchain
        uses: dtolnay/rust-toolchain@nightly

      - name: Cargo cache
        uses: Swatinem/rust-cache@v2
        with:
          workspaces: "crates/vibeflow-protocol/fuzz"

      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz --locked

      - name: Run parse fuzzer for 60s
        working-directory: crates/vibeflow-protocol
        run: cargo +nightly fuzz run parse -- -max_total_time=60

  npm:
    name: npm (@vibeflow/protocol)
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: bindings/npm
    steps:
      - uses: actions/checkout@v4

      - name: Use Node.js 20
        uses: actions/setup-node@v4
        with:
          node-version: "20"
          cache: "npm"
          cache-dependency-path: bindings/npm/package-lock.json

      - run: npm ci
      - run: npm run build
      - run: npm test
```

- [ ] **Step 2: Verify locally that all the steps the CI runs actually pass**

```bash
cd /path/to/vibeflow

# Rust job equivalents:
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --all-targets
cargo test --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# npm job equivalent:
( cd bindings/npm && npm run build && npm test )
```

Expected: every command exits 0. If `clippy` flags something — fix it (or, for a justified case, add `#[allow(…)]` *with a comment explaining why*). Don't blanket-allow.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: build/test/clippy/fmt/doc + 60s fuzz smoke + npm"
```

---

## Task 17: Cross-reference & link the artefacts

**Files:**
- Modify: `crates/vibeflow-protocol/README.md`
- Modify: `bindings/npm/README.md`
- Modify: `README.md` (workspace root)

Now that `docs/protocol.md` exists, make sure all three READMEs link to it correctly and to each other. (Most links were written speculatively in earlier tasks; this is the verification pass.)

- [ ] **Step 1: Open all three READMEs side-by-side and verify**

```bash
cd /path/to/vibeflow
grep -nH "protocol.md" README.md bindings/npm/README.md crates/vibeflow-protocol/README.md
grep -nH "github.com/bjhengen/vibeflow" README.md bindings/npm/README.md crates/vibeflow-protocol/README.md
```

Expected: every link is well-formed and points to either an in-repo path or `github.com/bjhengen/vibeflow/...`.

- [ ] **Step 2: Update the workspace root README to link to the artefacts**

Edit `/path/to/vibeflow/README.md`. Replace the existing **Repository layout** section with:

```markdown
## Repository layout

- [`crates/vibeflow-protocol/`](crates/vibeflow-protocol/) — Rust reference implementation, published as [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol) on crates.io. Includes the `vibeflow-emit` CLI.
- [`bindings/npm/`](bindings/npm/) — TypeScript reference implementation, published as [`@vibeflow/protocol`](https://www.npmjs.com/package/@vibeflow/protocol) on npm.
- [`docs/protocol.md`](docs/protocol.md) — the canonical OSC 1338 wire-format specification.
- [`docs/superpowers/specs/`](docs/superpowers/specs/) — design specs.
- [`docs/superpowers/plans/`](docs/superpowers/plans/) — implementation plans.

The terminal binary (`crates/vibeflow/`) is not yet built; this repository currently ships only the protocol foundation.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: cross-link README to protocol spec and published artefacts"
```

---

## Task 18: Cross-binding byte-for-byte compatibility check

**Files:**
- Modify: `crates/vibeflow-protocol/src/lib.rs`

Goal: prove the bytes the npm `toBytes` produces parse cleanly on the Rust side, and vice versa. We do this two ways: (a) a hardcoded fixture in a Rust unit test (cheap, deterministic, runs in CI), and (b) a hand-run shell smoke test (catches any toolchain weirdness end-to-end).

- [ ] **Step 1: Add a fixture-based unit test on the Rust side**

Append to the `mod tests` block in `crates/vibeflow-protocol/src/lib.rs`:

```rust
    /// Sanity check: parse a byte sequence byte-identical to what `@vibeflow/protocol`'s
    /// `toBytes` produces. If the npm and Rust formats ever diverge, this test fires first.
    /// The fixture was captured by hand from the npm side and is part of the test contract.
    #[test]
    fn parses_npm_emitted_bytes_byte_for_byte() {
        let bytes = b"\x1b]1338;state=working;tool=codex;project=vibeflow\x07";
        let f = parse(bytes).unwrap();
        assert_eq!(
            f,
            Frame::new(State::Working).with_tool("codex").with_project("vibeflow")
        );
    }

    /// Same idea in the other direction: the Rust `to_bytes` output should be byte-identical
    /// to what an npm caller computing `toBytes` for the same Frame would emit.
    #[test]
    fn rust_to_bytes_matches_npm_fixture() {
        let f = Frame::new(State::Waiting).with_tool("claude").with_project("vibeflow");
        let bytes = f.to_bytes();
        assert_eq!(
            bytes,
            b"\x1b]1338;state=waiting;tool=claude;project=vibeflow\x07"
        );
    }
```

- [ ] **Step 2: Run the tests — pass**

```bash
cd /path/to/vibeflow
cargo test -p vibeflow-protocol
```

Expected: all tests pass, including the two new fixture tests.

- [ ] **Step 3: One hand-run smoke check confirming the npm side actually emits matching bytes**

The package is CJS (Task 10 Step 1), and `tsc` outputs the entry at `dist/src/index.js`, so we use a plain `require` from a synchronous Node one-liner. Make sure `npm run build` has been run first so `dist/src/index.js` exists.

```bash
cd /path/to/vibeflow/bindings/npm
npm run build
node -e "
  const m = require('./dist/src/index.js');
  process.stdout.write(m.toBytes({ state: 'waiting', tool: 'claude', project: 'vibeflow' }));
" | xxd
```

Expected hex output (matching the Rust fixture in step 1's second test):

```
00000000: 1b5d 3133 3338 3b73 7461 7465 3d77 6169  .]1338;state=wai
00000010: 7469 6e67 3b74 6f6f 6c3d 636c 6175 6465  ting;tool=claude
00000020: 3b70 726f 6a65 6374 3d76 6962 6566 6c6f  ;project=vibeflo
00000030: 7707                                     w.
```

If the hex is a single byte off, you have a real format-divergence bug between the bindings — fix it before continuing.

- [ ] **Step 4: Commit**

```bash
cd /path/to/vibeflow
git add crates/vibeflow-protocol/src/lib.rs
git commit -m "test(protocol): cross-binding byte-for-byte compatibility fixtures"
```

---

## Task 19: Final verification + tag

**Files:** none (verification + git tag)

- [ ] **Step 1: Full local CI dry-run**

```bash
cd /path/to/vibeflow
cargo fmt --all -- --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo build --workspace --all-targets && \
  cargo test --workspace --all-targets && \
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps && \
  ( cd bindings/npm && npm run build && npm test ) && \
  echo "ALL GREEN"
```

Expected: trailing line is `ALL GREEN`. Anything else: stop, fix, re-run.

- [ ] **Step 2: 60-second fuzz**

```bash
cd /path/to/vibeflow/crates/vibeflow-protocol
cargo +nightly fuzz run parse -- -max_total_time=60
```

Expected: no crashes, no panics.

- [ ] **Step 3: Tag the milestone**

```bash
cd /path/to/vibeflow
git tag -a stage1-protocol-complete -m "vibeflow-protocol foundation complete (Stage 1 of v0.1)"
git tag --list   # confirm
```

(Don't push the tag yet — wait until after publishing to crates.io / npm, which is a separate decision left to the user.)

After this plan completes, surface the result to the user: report the local CI dry-run result, the tag name, and a one-line "ready for Stage 2 plan" handoff. The user (not the executing agent) decides when to publish to crates.io / npm and when to write the Stage 2 plan.

---

## Spec coverage check

Mapping spec sections → tasks:

| Spec section | Covered by |
|---|---|
| Goals — `OSC 1338` open standard | Tasks 1–7, 14 |
| Components — `vibeflow-protocol` crate | Tasks 1–9, 15 |
| Components — `@vibeflow/protocol` npm package | Tasks 10–13 |
| Components — `vibeflow-emit` binary | Task 8 |
| Protocol — wire format | Tasks 4, 5, 14 |
| Protocol — `State` enum (canonical) | Task 1 |
| Protocol — polyglot bindings | Tasks 1–13 |
| Testing — unit (`vibeflow-protocol::parse`/`emit`) | Tasks 1–7 |
| Testing — fuzz | Task 9 |
| Distribution & licensing — dual MIT/Apache-2.0 | Task 0 |
| Distribution — crates.io & npm publishing | Task 17 (READMEs prepare; actual publish is post-Stage-1) |
| CI — Linux only, build/test/clippy/fmt + 60 s fuzz smoke | Task 16 |

**Out of scope for this plan (deferred to later stages, with rationale):**

- `OscDispatcher` / `AiStateTracker` — Stage 2. They consume the protocol crate; they don't define it.
- PTY plumbing, GUI, renderer, tab bar, input, config, hot-reload, shell hooks — Stages 3+.
- Crates.io / npm publishing — happens after Stage 1 is reviewed and tagged. Not a code task; one-off `cargo publish` + `npm publish` invocations.
- Python binding — explicitly deferred to v0.2 in the spec.
- Headless GPU snapshot tests, Mac/Windows CI — explicitly deferred in the spec.

## Next plan

After this plan finishes:

1. Write **Stage 2 plan: `OscDispatcher` + `AiStateTracker`**. Inputs: this plan's protocol crate, plus the spec sections on streaming OSC parsing, debouncing, heuristic-silence and stale-state timeouts.
2. Write **Stage 3 plan: PTY + reader thread + headless `App`**. Inputs: Stages 1–2.

Each subsequent stage gets its own focused plan, paced for learning Rust and producing demonstrably-working software at every commit.
