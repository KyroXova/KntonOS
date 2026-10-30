//! In-Game Quick-Access Overlay & Hardware Display Plane Subsystem for KntonOS.
//!
//! Provides ultra-low-overhead hardware overlay planes, zero-copy hardware cursor composition,
//! handheld quick-access menu (QAM) management, and in-game performance sliders.

pub mod overlay_plane;

pub use overlay_plane::*;
