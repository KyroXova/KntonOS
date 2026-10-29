//! Desktop Shell Compositor & Window Manager for KntonOS.

pub mod dirty;
pub mod cursor;
pub mod window;
pub mod compositor;

pub use dirty::{DamageRegion, Rect};
pub use cursor::{CursorState, CURSOR_HEIGHT, CURSOR_WIDTH};
pub use window::{Window, BORDER_WIDTH, BUTTON_WIDTH, TITLEBAR_HEIGHT};
pub use compositor::{SceneCompositor, DEFAULT_TASKBAR_HEIGHT};
