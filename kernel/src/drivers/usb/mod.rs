//! Universal Serial Bus (USB) Subsystem for KntonOS.

pub mod xhci;

pub use xhci::{XhciCapabilities, XhciController, XhciError};
