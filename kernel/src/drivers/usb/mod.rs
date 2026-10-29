//! Universal Serial Bus (USB) Subsystem for KntonOS.

pub mod hid;
pub mod xhci;

pub use hid::{
    ConfigurationDescriptor, DeviceDescriptor, EndpointDescriptor, HidDescriptor,
    HidKeyboardReport, HidMouseReport, InterfaceDescriptor, UsbConfigurationBundle,
    USB_CLASS_HID, USB_PROTOCOL_KEYBOARD, USB_PROTOCOL_MOUSE, USB_SUBCLASS_BOOT,
};
pub use xhci::{
    CommandRing, Dcbaa, DeviceContext, EndpointContext, ErstEntry, EventRing, InputContext,
    InputControlContext, SlotContext, Trb, TrbType, XhciCapabilities, XhciController, XhciError,
    XhciEvent,
};
