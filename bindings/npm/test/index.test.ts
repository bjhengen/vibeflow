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
