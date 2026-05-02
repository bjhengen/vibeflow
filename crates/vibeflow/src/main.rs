//! Headless demo binary: spawn one shell, poll for state changes, print them.
//!
//! Stage 4 replaces this with the winit event loop and the wgpu renderer.
//! For Stage 3 it just exercises the PTY → dispatcher → tracker pipeline so
//! integration tests have something to compile against.

fn main() {
    eprintln!("vibeflow Stage 3: headless demo not yet implemented");
}
