//! winit `ApplicationHandler` integration: the [`WindowApp`] type owns the
//! `Window`, the [`crate::render::Renderer`], and the [`crate::app::App`].
//! Drives polling, ticking, and event routing on the main thread.
