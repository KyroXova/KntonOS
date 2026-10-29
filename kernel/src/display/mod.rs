//! Linear Framebuffer & 2D Desktop Compositor for KntonOS.

pub mod color;
pub mod desktop;
pub mod draw;
pub mod font;
pub mod framebuffer;

pub use color::Color;
pub use desktop::{DesktopShell, WindowRect, TASKBAR_HEIGHT};
pub use draw::{draw_char, draw_rect, draw_rect_outline, draw_string};
pub use font::{FONT_HEIGHT, FONT_WIDTH};
pub use framebuffer::{FramebufferInfo, LinearFramebuffer};
