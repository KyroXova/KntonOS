//! Interactive Virtual Terminal (VT) Shell and Key-Event Dispatch Engine for KntonOS.

pub mod input;
pub mod terminal;

pub use input::{KeyAction, KeyBuffer, KeyboardDecoder, KeyEvent, ModifierState, poll_ps2_hardware};
pub use terminal::{TerminalBuffer, TerminalCell, DEFAULT_COLS, DEFAULT_ROWS};
