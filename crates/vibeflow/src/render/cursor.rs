//! `CursorBlink` — 500 ms-period blink-state oracle. The renderer asks
//! `visible(now)` when building the active tab's cell instances; if `false`,
//! the cursor cell is drawn with the un-inverted (regular) fg/bg.

#![allow(dead_code)] // first user is `Renderer` in Stage 7 Task 7
