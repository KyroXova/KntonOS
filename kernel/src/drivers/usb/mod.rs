//! Universal Serial Bus (USB) Subsystem for KntonOS.

pub mod xhci;

pub use xhci::{
    CommandRing, Dcbaa, DeviceContext, EndpointContext, ErstEntry, EventRing, InputContext,
    InputControlContext, SlotContext, Trb, TrbType, XhciCapabilities, XhciController, XhciError,
    XhciEvent,
};
