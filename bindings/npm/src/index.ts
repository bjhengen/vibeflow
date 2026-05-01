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
