//! In-Game Quick-Access Overlay & Hardware Display Plane Subsystem for KntonOS.
//! Provides ultra-low-overhead hardware overlay planes, zero-copy hardware cursor composition,

pub mod overlay_plane;
pub mod keyboard;

pub use overlay_plane::*;
pub use keyboard::*;
