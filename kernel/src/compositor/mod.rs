//! Desktop Shell Compositor & Window Manager for KntonOS.

pub mod dirty;
pub mod cursor;

pub use dirty::{DamageRegion, Rect};
pub use cursor::{CursorState, CURSOR_HEIGHT, CURSOR_WIDTH};
