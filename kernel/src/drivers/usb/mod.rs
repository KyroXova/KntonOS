//! Universal Serial Bus (USB) Subsystem for KntonOS.

pub mod hid;
pub mod xhci;

pub use hid::{
    dispatch_keyboard_report, dispatch_mouse_report, ConfigurationDescriptor, DeviceDescriptor,
    EndpointDescriptor, HidDescriptor, HidKeyboardReport, HidMouseReport, InterfaceDescriptor,
    UsbConfigurationBundle, USB_CLASS_HID, USB_PROTOCOL_KEYBOARD, USB_PROTOCOL_MOUSE,
    USB_SUBCLASS_BOOT,
};
pub use xhci::{
    CommandRing, Dcbaa, DeviceContext, EndpointContext, ErstEntry, EventRing, InputContext,
    InputControlContext, SlotContext, Trb, TrbType, XhciCapabilities, XhciController, XhciError,
    XhciEvent,
};

use crate::serial_write_str;

/// Comprehensive verification self-test suite for USB 3.0 xHCI & HID Input Pipeline.
pub fn test_usb_hid_subsystem() -> bool {
    serial_write_str("[KNT-USB-TEST] Running USB 3.0 xHCI & HID input pipeline verification suite...\r\n");

    // 1. Verify xHCI Command Ring TRB creation & cycling
    let mut cmd_ring = CommandRing::new();
    let trb_noop = Trb::no_op_command(true);
    if trb_noop.trb_type() != TrbType::NoOpCommand || !trb_noop.cycle() {
        serial_write_str("[KNT-PANIC] xHCI NoOpCommand TRB generation failed!\r\n");
        return false;
    }
    let trb_slot = Trb::enable_slot_command(0, true);
    if trb_slot.trb_type() != TrbType::EnableSlotCommand {
        serial_write_str("[KNT-PANIC] xHCI EnableSlotCommand TRB generation failed!\r\n");
        return false;
    }
    let trb_addr = Trb::address_device_command(0x1000, 1, false, true);
    if trb_addr.trb_type() != TrbType::AddressDeviceCommand || trb_addr.parameter != 0x1000 {
        serial_write_str("[KNT-PANIC] xHCI AddressDeviceCommand TRB generation failed!\r\n");
        return false;
    }
    let trb_cfg = Trb::configure_endpoint_command(0x2000, 1, false, true);
    if trb_cfg.trb_type() != TrbType::ConfigureEndpointCommand || trb_cfg.parameter != 0x2000 {
        serial_write_str("[KNT-PANIC] xHCI ConfigureEndpointCommand TRB generation failed!\r\n");
        return false;
    }
    let _ = cmd_ring.push(trb_noop);
    let _ = cmd_ring.push(trb_slot);
    let _ = cmd_ring.push(trb_addr);
    let _ = cmd_ring.push(trb_cfg);

    // 2. Verify xHCI Event Ring polling & decoding
    let mut event_ring = EventRing::new();
    // Simulate Command Completion Event (completion code 1 = Success, slot_id 1)
    let comp_status = 1u32 << 24;
    let comp_control = (1u32 << 24) | ((TrbType::CommandCompletionEvent as u32) << 10) | 1;
    let comp_trb = Trb::new(0x1000, comp_status, comp_control);
    event_ring.push_event_synthetic(comp_trb);

    match event_ring.poll_event() {
        Some(XhciEvent::CommandCompletion { cmd_trb_ptr, completion_code, slot_id, .. }) => {
            if cmd_trb_ptr != 0x1000 || completion_code != 1 || slot_id != 1 {
                serial_write_str("[KNT-PANIC] xHCI CommandCompletionEvent decoding mismatch!\r\n");
                return false;
            }
        }
        _ => {
            serial_write_str("[KNT-PANIC] xHCI CommandCompletionEvent polling failed!\r\n");
            return false;
        }
    }

    // Simulate Port Status Change Event (port 2, completion code 1)
    let port_param = 2u64 << 24;
    let port_status = 1u32 << 24;
    let port_control = ((TrbType::PortStatusChangeEvent as u32) << 10) | 1;
    let port_trb = Trb::new(port_param, port_status, port_control);
    event_ring.push_event_synthetic(port_trb);

    match event_ring.poll_event() {
        Some(XhciEvent::PortStatusChange { port_id, completion_code }) => {
            if port_id != 2 || completion_code != 1 {
                serial_write_str("[KNT-PANIC] xHCI PortStatusChangeEvent decoding mismatch!\r\n");
                return false;
            }
        }
        _ => {
            serial_write_str("[KNT-PANIC] xHCI PortStatusChangeEvent polling failed!\r\n");
            return false;
        }
    }

    // 3. Verify DCBAA, Slot Context, Endpoint Context structures
    let mut dcbaa = Dcbaa::new();
    let _ = dcbaa.set_device_context(1, 0x8000_1000);
    if dcbaa.get_device_context(1) != 0x8000_1000 {
        serial_write_str("[KNT-PANIC] xHCI DCBAA device context mapping failed!\r\n");
        return false;
    }

    let slot = SlotContext::new(4, 2, 1);
    if slot.speed() != 4 || slot.context_entries() != 2 || slot.root_port() != 1 {
        serial_write_str("[KNT-PANIC] xHCI SlotContext bitfield decoding mismatch!\r\n");
        return false;
    }

    let ep0 = EndpointContext::new_control_ep0(64, 0x9000);
    if ep0.max_packet_size() != 64 || ep0.ep_type() != 4 {
        serial_write_str("[KNT-PANIC] xHCI EndpointContext control EP0 mismatch!\r\n");
        return false;
    }

    let ep_intr = EndpointContext::new_interrupt_in(8, 4, 0xA000);
    if ep_intr.max_packet_size() != 8 || ep_intr.ep_type() != 7 {
        serial_write_str("[KNT-PANIC] xHCI EndpointContext interrupt IN mismatch!\r\n");
        return false;
    }

    serial_write_str("  * xHCI command/event ring and context structures verified\r\n");
    serial_write_str("[KNT-USB-XHCI-PASS] xHCI command/event ring and context structures verified\r\n");

    // 4. Verify USB standard descriptors and HID parser
    let raw_dev_desc: [u8; 18] = [
        18, 1, 0x00, 0x02, 0, 0, 0, 64, 0x6B, 0x12, 0x01, 0x00, 0x00, 0x01, 1, 2, 0, 1,
    ];
    let dev_desc = match DeviceDescriptor::parse(&raw_dev_desc) {
        Some(d) => d,
        None => {
            serial_write_str("[KNT-PANIC] USB DeviceDescriptor parsing failed!\r\n");
            return false;
        }
    };
    if dev_desc.id_vendor != 0x126B || dev_desc.id_product != 0x0001 || dev_desc.max_packet_size0 != 64 {
        serial_write_str("[KNT-PANIC] USB DeviceDescriptor field mismatch!\r\n");
        return false;
    }

    // Synthetic HID Configuration Bundle (Config: 9 bytes, Interface: 9 bytes, HID: 9 bytes, Endpoint: 7 bytes = 34 bytes)
    let raw_cfg_bundle: [u8; 34] = [
        // Config Descriptor (9 bytes)
        9, 2, 34, 0, 1, 1, 0, 0xA0, 50,
        // Interface Descriptor (9 bytes): HID Boot Mouse
        9, 4, 0, 0, 1, USB_CLASS_HID, USB_SUBCLASS_BOOT, USB_PROTOCOL_MOUSE, 0,
        // HID Descriptor (9 bytes)
        9, 0x21, 0x11, 0x01, 0, 1, 0x22, 52, 0,
        // Endpoint Descriptor (7 bytes): EP1 IN, Interrupt, 8 bytes, interval 10
        7, 5, 0x81, 0x03, 8, 0, 10,
    ];
    let bundle = match UsbConfigurationBundle::parse(&raw_cfg_bundle) {
        Some(b) => b,
        None => {
            serial_write_str("[KNT-PANIC] USB UsbConfigurationBundle parsing failed!\r\n");
            return false;
        }
    };
    if bundle.interfaces.len() != 1 || !bundle.interfaces[0].is_hid_mouse() {
        serial_write_str("[KNT-PANIC] USB HID mouse interface identification failed!\r\n");
        return false;
    }
    if bundle.endpoints.len() != 1 || !bundle.endpoints[0].is_in() || !bundle.endpoints[0].is_interrupt() {
        serial_write_str("[KNT-PANIC] USB HID interrupt endpoint identification failed!\r\n");
        return false;
    }

    serial_write_str("  * USB descriptors and HID configuration bundle parser verified\r\n");
    serial_write_str("[KNT-USB-HID-DESC-PASS] USB descriptors and HID configuration bundle parser verified\r\n");

    // 5. Verify USB HID mouse report loopback into Desktop Compositor cursor
    let initial_cursor_pos = crate::compositor::with_compositor(|comp| (comp.cursor.x, comp.cursor.y))
        .unwrap_or((0, 0));

    let mouse_report = HidMouseReport {
        buttons: 0x01,
        left_button: true,
        right_button: false,
        middle_button: false,
        dx: 20,
        dy: -10,
        wheel: 0,
    };
    dispatch_mouse_report(&mouse_report);

    let (new_cursor_x, new_cursor_y, new_buttons) = crate::compositor::with_compositor(|comp| {
        (comp.cursor.x, comp.cursor.y, comp.cursor.buttons)
    }).unwrap_or((0, 0, 0));

    if new_cursor_x != initial_cursor_pos.0 + 20
        || new_cursor_y != initial_cursor_pos.1.saturating_sub(10)
        || (new_buttons & 0x01) == 0
    {
        serial_write_str("[KNT-PANIC] Desktop Compositor cursor update from USB mouse failed!\r\n");
        return false;
    }

    serial_write_str("  * USB HID mouse motion and button events routed to compositor cursor\r\n");
    serial_write_str("[KNT-USB-HID-MOUSE-PASS] USB HID mouse motion and button events routed to compositor cursor\r\n");

    // 6. Verify USB HID keyboard report loopback into Virtual Terminal Shell
    let kbd_report = HidKeyboardReport {
        modifiers: 0,
        lctrl: false,
        lshift: false,
        lalt: false,
        lgui: false,
        rctrl: false,
        rshift: false,
        ralt: false,
        rgui: false,
        keys: [0x11, 0, 0, 0, 0, 0], // 0x11 is 'n'
    };
    dispatch_keyboard_report(&kbd_report);

    let char_received = {
        let guard = crate::shell::SHELL.lock();
        if let Some(ref sh) = *guard {
            sh.current_line.ends_with('n')
        } else {
            false
        }
    };
    if !char_received {
        serial_write_str("[KNT-PANIC] Virtual Terminal Shell input from USB keyboard failed!\r\n");
        return false;
    }

    serial_write_str("  * USB HID keyboard scan events routed to virtual terminal shell\r\n");
    serial_write_str("[KNT-USB-HID-KBD-PASS] USB HID keyboard scan events routed to virtual terminal shell\r\n");

    serial_write_str("[KNT-USB-HID-PASS] All USB 3.0 xHCI and HID input pipeline checks passed\r\n");
    true
}
