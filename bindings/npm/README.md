# vibeflow-protocol

TypeScript binding for the **OSC 1338 protocol** — an open standard for AI-tool state signalling in terminals.

When an AI tool (Claude Code, Codex, Aider, …) emits an OSC 1338 sequence, a compliant terminal (e.g. [vibeflow](https://github.com/bjhengen/vibeflow)) updates that tab's visual indicator: amber pulse for `waiting`, blue for `working`. The protocol is open so anything that emits the bytes — a wrapper script, a Node CLI, a hooked LSP — Just Works.

## Install

```bash
npm install vibeflow-protocol
```

## Quick start

```ts
import { emitState } from "vibeflow-protocol";

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

## Emitting from a hook (captured stdout)

`emit()` writes to `process.stdout`. Some hosts capture a tool's stdout — notably
Claude Code hooks — so those bytes never reach the terminal and the indicator never
updates. In that case, write the frame to the controlling terminal (`/dev/tty`)
directly, the way the `vibeflow-emit` CLI does:

```ts
import { openSync, writeSync } from "node:fs";
import { toBytes } from "vibeflow-protocol";

writeSync(openSync("/dev/tty", "w"), toBytes({ state: "waiting" }));
```

## When emitted bytes do nothing

In any terminal that doesn't recognise OSC 1338, the bytes are silently consumed and produce no output. So it's safe to call `emitState` from a tool that may or may not run inside vibeflow.

## License

Dual-licensed under MIT or Apache-2.0 at your option.
