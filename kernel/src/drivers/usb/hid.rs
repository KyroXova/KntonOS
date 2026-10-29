//! Standard USB Descriptor Parser & HID Boot Protocol Decoder for KntonOS.
//!
//! Implements USB 2.0/3.0 descriptor decoders (Device, Configuration, Interface,
//! Endpoint, HID) and standard HID Boot Protocol parsers for Mouse and Keyboard.

use alloc::vec::Vec;

pub const USB_CLASS_HID: u8 = 0x03;
pub const USB_SUBCLASS_BOOT: u8 = 0x01;
pub const USB_PROTOCOL_KEYBOARD: u8 = 0x01;
pub const USB_PROTOCOL_MOUSE: u8 = 0x02;

/// Standard USB 18-byte Device Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub bcd_usb: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub num_configurations: u8,
}

impl DeviceDescriptor {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 18 || buf[0] < 18 || buf[1] != 1 {
            return None;
        }
        Some(Self {
            length: buf[0],
            descriptor_type: buf[1],
            bcd_usb: u16::from_le_bytes([buf[2], buf[3]]),
            device_class: buf[4],
            device_subclass: buf[5],
            device_protocol: buf[6],
            max_packet_size0: buf[7],
            id_vendor: u16::from_le_bytes([buf[8], buf[9]]),
            id_product: u16::from_le_bytes([buf[10], buf[11]]),
            bcd_device: u16::from_le_bytes([buf[12], buf[13]]),
            i_manufacturer: buf[14],
            i_product: buf[15],
            i_serial_number: buf[16],
            num_configurations: buf[17],
        })
    }
}

/// Standard USB 9-byte Configuration Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigurationDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub total_length: u16,
    pub num_interfaces: u8,
    pub configuration_value: u8,
    pub i_configuration: u8,
    pub attributes: u8,
    pub max_power: u8,
}

impl ConfigurationDescriptor {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 9 || buf[0] < 9 || buf[1] != 2 {
            return None;
        }
        Some(Self {
            length: buf[0],
            descriptor_type: buf[1],
            total_length: u16::from_le_bytes([buf[2], buf[3]]),
            num_interfaces: buf[4],
            configuration_value: buf[5],
            i_configuration: buf[6],
            attributes: buf[7],
            max_power: buf[8],
        })
    }
}

/// Standard USB 9-byte Interface Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub num_endpoints: u8,
    pub interface_class: u8,
    pub interface_subclass: u8,
    pub interface_protocol: u8,
    pub i_interface: u8,
}

impl InterfaceDescriptor {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 9 || buf[0] < 9 || buf[1] != 4 {
            return None;
        }
        Some(Self {
            length: buf[0],
            descriptor_type: buf[1],
            interface_number: buf[2],
            alternate_setting: buf[3],
            num_endpoints: buf[4],
            interface_class: buf[5],
            interface_subclass: buf[6],
            interface_protocol: buf[7],
            i_interface: buf[8],
        })
    }

    pub fn is_hid(&self) -> bool {
        self.interface_class == USB_CLASS_HID
    }

    pub fn is_hid_keyboard(&self) -> bool {
        self.interface_class == USB_CLASS_HID
            && self.interface_subclass == USB_SUBCLASS_BOOT
            && self.interface_protocol == USB_PROTOCOL_KEYBOARD
    }

    pub fn is_hid_mouse(&self) -> bool {
        self.interface_class == USB_CLASS_HID
            && self.interface_subclass == USB_SUBCLASS_BOOT
            && self.interface_protocol == USB_PROTOCOL_MOUSE
    }
}

/// Standard USB 7-byte Endpoint Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub endpoint_address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

impl EndpointDescriptor {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 7 || buf[0] < 7 || buf[1] != 5 {
            return None;
        }
        Some(Self {
            length: buf[0],
            descriptor_type: buf[1],
            endpoint_address: buf[2],
            attributes: buf[3],
            max_packet_size: u16::from_le_bytes([buf[4], buf[5]]),
            interval: buf[6],
        })
    }

    pub fn is_in(&self) -> bool {
        (self.endpoint_address & 0x80) != 0
    }

    pub fn endpoint_number(&self) -> u8 {
        self.endpoint_address & 0x0F
    }

    pub fn is_interrupt(&self) -> bool {
        (self.attributes & 0x03) == 0x03
    }
}

/// USB HID Class Descriptor (0x21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidDescriptor {
    pub length: u8,
    pub descriptor_type: u8,
    pub bcd_hid: u16,
    pub country_code: u8,
    pub num_descriptors: u8,
    pub report_desc_type: u8,
    pub report_desc_length: u16,
}

impl HidDescriptor {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 9 || buf[0] < 9 || buf[1] != 0x21 {
            return None;
        }
        Some(Self {
            length: buf[0],
            descriptor_type: buf[1],
            bcd_hid: u16::from_le_bytes([buf[2], buf[3]]),
            country_code: buf[4],
            num_descriptors: buf[5],
            report_desc_type: buf[6],
            report_desc_length: u16::from_le_bytes([buf[7], buf[8]]),
        })
    }
}

/// Parsed USB Configuration Descriptor Bundle.
#[derive(Debug, Clone)]
pub struct UsbConfigurationBundle {
    pub config: ConfigurationDescriptor,
    pub interfaces: Vec<InterfaceDescriptor>,
    pub endpoints: Vec<EndpointDescriptor>,
    pub hid: Option<HidDescriptor>,
}

impl UsbConfigurationBundle {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        let config = ConfigurationDescriptor::parse(buf)?;
        let mut interfaces = Vec::new();
        let mut endpoints = Vec::new();
        let mut hid = None;

        let total = (config.total_length as usize).min(buf.len());
        let mut offset = config.length as usize;

        while offset + 2 <= total {
            let len = buf[offset] as usize;
            if len == 0 || offset + len > total {
                break;
            }
            let desc_type = buf[offset + 1];
            match desc_type {
                4 => {
                    if let Some(iface) = InterfaceDescriptor::parse(&buf[offset..offset + len]) {
                        interfaces.push(iface);
                    }
                }
                5 => {
                    if let Some(ep) = EndpointDescriptor::parse(&buf[offset..offset + len]) {
                        endpoints.push(ep);
                    }
                }
                0x21 => {
                    if let Some(h) = HidDescriptor::parse(&buf[offset..offset + len]) {
                        hid = Some(h);
                    }
                }
                _ => {}
            }
            offset += len;
        }

        Some(Self {
            config,
            interfaces,
            endpoints,
            hid,
        })
    }
}

/// Standard HID Boot Mouse Report (3 buttons, 8-bit dX, 8-bit dY, optional wheel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HidMouseReport {
    pub buttons: u8,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
    pub dx: i8,
    pub dy: i8,
    pub wheel: i8,
}

impl HidMouseReport {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 3 {
            return None;
        }
        let buttons = buf[0];
        let left_button = (buttons & 0x01) != 0;
        let right_button = (buttons & 0x02) != 0;
        let middle_button = (buttons & 0x04) != 0;
        let dx = buf[1] as i8;
        let dy = buf[2] as i8;
        let wheel = if buf.len() >= 4 { buf[3] as i8 } else { 0 };

        Some(Self {
            buttons,
            left_button,
            right_button,
            middle_button,
            dx,
            dy,
            wheel,
        })
    }
}

/// Standard HID Boot Keyboard Report (modifier mask + 6-key rollover array).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HidKeyboardReport {
    pub modifiers: u8,
    pub lctrl: bool,
    pub lshift: bool,
    pub lalt: bool,
    pub lgui: bool,
    pub rctrl: bool,
    pub rshift: bool,
    pub ralt: bool,
    pub rgui: bool,
    pub keys: [u8; 6],
}

impl HidKeyboardReport {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 8 {
            return None;
        }
        let modifiers = buf[0];
        let lctrl = (modifiers & 0x01) != 0;
        let lshift = (modifiers & 0x02) != 0;
        let lalt = (modifiers & 0x04) != 0;
        let lgui = (modifiers & 0x08) != 0;
        let rctrl = (modifiers & 0x10) != 0;
        let rshift = (modifiers & 0x20) != 0;
        let ralt = (modifiers & 0x40) != 0;
        let rgui = (modifiers & 0x80) != 0;

        let mut keys = [0u8; 6];
        keys.copy_from_slice(&buf[2..8]);

        Some(Self {
            modifiers,
            lctrl,
            lshift,
            lalt,
            lgui,
            rctrl,
            rshift,
            ralt,
            rgui,
            keys,
        })
    }

    pub fn is_shift(&self) -> bool {
        self.lshift || self.rshift
    }

    pub fn is_ctrl(&self) -> bool {
        self.lctrl || self.rctrl
    }

    pub fn is_alt(&self) -> bool {
        self.lalt || self.ralt
    }

    /// Convert active key presses in this report into key events.
    pub fn to_key_events(&self) -> Vec<crate::shell::KeyEvent> {
        let mut events = Vec::new();
        let shift = self.is_shift();
        let ctrl = self.is_ctrl();
        let alt = self.is_alt();

        for &keycode in self.keys.iter() {
            if keycode != 0 {
                events.push(crate::shell::KeyEvent {
                    char_code: Self::hid_keycode_to_char(keycode, shift),
                    scancode: keycode,
                    action: crate::shell::KeyAction::Press,
                    shift,
                    ctrl,
                    alt,
                });
            }
        }
        events
    }

    /// Converts standard USB HID keycodes to ASCII character.
    pub fn hid_keycode_to_char(keycode: u8, shift: bool) -> Option<char> {
        match keycode {
            0x04..=0x1D => {
                let base = if shift { b'A' } else { b'a' };
                Some((base + (keycode - 0x04)) as char)
            }
            0x1E => Some(if shift { '!' } else { '1' }),
            0x1F => Some(if shift { '@' } else { '2' }),
            0x20 => Some(if shift { '#' } else { '3' }),
            0x21 => Some(if shift { '$' } else { '4' }),
            0x22 => Some(if shift { '%' } else { '5' }),
            0x23 => Some(if shift { '^' } else { '6' }),
            0x24 => Some(if shift { '&' } else { '7' }),
            0x25 => Some(if shift { '*' } else { '8' }),
            0x26 => Some(if shift { '(' } else { '9' }),
            0x27 => Some(if shift { ')' } else { '0' }),
            0x28 => Some('\n'),  // Enter
            0x2A => Some('\x08'), // Backspace
            0x2B => Some('\t'),   // Tab
            0x2C => Some(' '),    // Space
            0x2D => Some(if shift { '_' } else { '-' }),
            0x2E => Some(if shift { '+' } else { '=' }),
            0x2F => Some(if shift { '{' } else { '[' }),
            0x30 => Some(if shift { '}' } else { ']' }),
            0x31 => Some(if shift { '|' } else { '\\' }),
            0x33 => Some(if shift { ':' } else { ';' }),
            0x34 => Some(if shift { '"' } else { '\'' }),
            0x35 => Some(if shift { '~' } else { '`' }),
            0x36 => Some(if shift { '<' } else { ',' }),
            0x37 => Some(if shift { '>' } else { '.' }),
            0x38 => Some(if shift { '?' } else { '/' }),
            _ => None,
        }
    }
}

/// Route a decoded USB HID mouse report into the desktop compositor cursor.
pub fn dispatch_mouse_report(report: &HidMouseReport) {
    crate::compositor::update_cursor(report.dx as i32, report.dy as i32, report.buttons);
}

/// Route a decoded USB HID keyboard report into the virtual terminal shell.
pub fn dispatch_keyboard_report(report: &HidKeyboardReport) {
    for event in report.to_key_events() {
        crate::shell::push_key_event(event);
    }
}

