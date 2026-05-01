import { test } from "node:test";
import assert from "node:assert/strict";

import { Frame, MAX_FRAME_LEN, parse, toBytes } from "../src/index";

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
