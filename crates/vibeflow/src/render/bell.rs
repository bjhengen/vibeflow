//! `BellFlash` — 200 ms white-tint fade triggered by the VTE BEL action
//! (`0x07`). The renderer reads `tint_alpha(now)` per frame and overlays a
//! full-window white rect (via `TabBarPipeline`) when the alpha is > 0.

#![allow(dead_code)] // first user is `Renderer` in Stage 7 Task 8
