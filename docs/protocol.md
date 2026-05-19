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
- Unrecognised values for known keys: implementation-defined. The reference parsers (`vibeflow-protocol` on crates.io and npm) currently raise an error for unknown `state` values; vibeflow's dispatcher logs at debug level and ignores the frame.

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

1. **Native:** the tool calls a binding (this crate, the `vibeflow-protocol` npm package, etc.) directly.
2. **Wrapper:** a thin shim spawned around the tool watches its output and emits OSC 1338 on its behalf.
3. **Heuristic:** vibeflow itself watches process names + output silence and infers state when no explicit signal arrives.

Tier 1 is the goal; tiers 2 and 3 ensure the experience is never broken on day one.

## Reference implementations

- Rust: [`vibeflow-protocol`](https://crates.io/crates/vibeflow-protocol)
- TypeScript: [`vibeflow-protocol`](https://www.npmjs.com/package/vibeflow-protocol)
- Shell helper: `vibeflow-emit` (a binary in the `vibeflow-protocol` crate)

## Versioning

This document is the contract. Breaking changes (renaming or removing existing keys, changing existing state semantics) bump a major-version protocol identifier (`1338` → some future identifier); they will not happen casually. Additive changes (new optional keys, new state values) are guaranteed safe for old consumers.
