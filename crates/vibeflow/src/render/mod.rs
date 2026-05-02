//! GPU rendering primitives. Stage 4 ships a minimal [`Renderer`] that opens a
//! wgpu surface on a [`winit::window::Window`] and clears it to a solid color.
//! Stage 5 layers the cell grid on top; Stage 6 adds the tab bar.
