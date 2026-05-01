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
 *
 * Returns a JavaScript string — that's what Node's `process.stdout.write` and
 * most other terminal-bound sinks consume. The OSC bytes (ESC=0x1B, BEL=0x07)
 * are valid single-byte ASCII control characters, and the rest of the payload
 * is ASCII (percent-encoding handles non-ASCII), so the string round-trips
 * unchanged through UTF-8 encoding. Pass through `Buffer.from(..., 'binary')`
 * if you need a `Buffer` for non-default-encoding writes.
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

  // Strict undefined check — not falsy. All current State values are non-empty
  // strings, but `if (!result.state)` would also fire for any future "" sentinel.
  if (result.state === undefined) {
    throw new Error("vibeflow-protocol: missing state");
  }
  // Build the result without `tool`/`project` keys when undefined (cleaner deepEqual).
  const out: Frame = { state: result.state };
  if (result.tool !== undefined) out.tool = result.tool;
  if (result.project !== undefined) out.project = result.project;
  return out;
}
