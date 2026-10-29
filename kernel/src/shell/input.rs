//! PS/2 Keyboard Scancode Set 1/2 Decoder and Key Event Buffer for KntonOS.

use alloc::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub char_code: Option<char>,
    pub scancode: u8,
    pub action: KeyAction,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ModifierState {
    pub lshift: bool,
    pub rshift: bool,
    pub lctrl: bool,
    pub rctrl: bool,
    pub lalt: bool,
    pub ralt: bool,
    pub caps_lock: bool,
}

impl ModifierState {
    pub const fn new() -> Self {
        Self {
            lshift: false,
            rshift: false,
            lctrl: false,
            rctrl: false,
            lalt: false,
            ralt: false,
            caps_lock: false,
        }
    }

    #[inline(always)]
    pub fn is_shift(&self) -> bool {
        self.lshift || self.rshift
    }

    #[inline(always)]
    pub fn is_ctrl(&self) -> bool {
        self.lctrl || self.rctrl
    }

    #[inline(always)]
    pub fn is_alt(&self) -> bool {
        self.lalt || self.ralt
    }
}

pub const DEFAULT_KEY_BUFFER_CAPACITY: usize = 256;

/// Ring buffer storing decoded key events and ASCII characters.
#[derive(Debug, Clone)]
pub struct KeyBuffer {
    events: VecDeque<KeyEvent>,
    chars: VecDeque<char>,
    capacity: usize,
}

impl KeyBuffer {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_KEY_BUFFER_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            chars: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push_event(&mut self, event: KeyEvent) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);

        if event.action == KeyAction::Press {
            if let Some(c) = event.char_code {
                if self.chars.len() >= self.capacity {
                    self.chars.pop_front();
                }
                self.chars.push_back(c);
            }
        }
    }

    pub fn pop_event(&mut self) -> Option<KeyEvent> {
        self.events.pop_front()
    }

    pub fn pop_char(&mut self) -> Option<char> {
        self.chars.pop_front()
    }

    pub fn has_chars(&self) -> bool {
        !self.chars.is_empty()
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.chars.clear();
    }
}

/// PS/2 keyboard scancode set 1 and 2 stateful decoder.
pub struct KeyboardDecoder {
    pub modifiers: ModifierState,
    pub buffer: KeyBuffer,
    is_extended: bool,
    is_set2_release: bool,
}

impl KeyboardDecoder {
    pub fn new() -> Self {
        Self {
            modifiers: ModifierState::new(),
            buffer: KeyBuffer::new(),
            is_extended: false,
            is_set2_release: false,
        }
    }

    /// Process incoming scancode using Scancode Set 1 (PC standard).
    pub fn process_scancode_set1(&mut self, scancode: u8) -> Option<KeyEvent> {
        // Extended scancode prefix
        if scancode == 0xE0 {
            self.is_extended = true;
            return None;
        }

        let is_release = (scancode & 0x80) != 0;
        let make_code = scancode & 0x7F;
        let extended = self.is_extended;
        self.is_extended = false;

        // Track modifiers
        match (make_code, extended) {
            (0x2A, false) => self.modifiers.lshift = !is_release,
            (0x36, false) => self.modifiers.rshift = !is_release,
            (0x1D, false) => self.modifiers.lctrl = !is_release,
            (0x1D, true) => self.modifiers.rctrl = !is_release,
            (0x38, false) => self.modifiers.lalt = !is_release,
            (0x38, true) => self.modifiers.ralt = !is_release,
            (0x3A, false) if !is_release => {
                self.modifiers.caps_lock = !self.modifiers.caps_lock;
            }
            _ => {}
        }

        let action = if is_release {
            KeyAction::Release
        } else {
            KeyAction::Press
        };

        let effective_shift = self.modifiers.is_shift();
        let char_code = if action == KeyAction::Press {
            scancode_set1_to_ascii(make_code, effective_shift, self.modifiers.caps_lock)
        } else {
            None
        };

        let event = KeyEvent {
            char_code,
            scancode,
            action,
            shift: effective_shift,
            ctrl: self.modifiers.is_ctrl(),
            alt: self.modifiers.is_alt(),
        };

        self.buffer.push_event(event);
        Some(event)
    }

    /// Process incoming scancode using Scancode Set 2.
    pub fn process_scancode_set2(&mut self, scancode: u8) -> Option<KeyEvent> {
        if scancode == 0xE0 {
            self.is_extended = true;
            return None;
        }

        if scancode == 0xF0 {
            self.is_set2_release = true;
            return None;
        }

        let is_release = self.is_set2_release;
        self.is_set2_release = false;
        let extended = self.is_extended;
        self.is_extended = false;

        // Map set 2 scancode to set 1 make code
        let make_code_opt = map_set2_to_set1(scancode, extended);
        let make_code = match make_code_opt {
            Some(mc) => mc,
            None => return None,
        };

        // Synthesize set 1 scancode
        let s1 = if is_release {
            make_code | 0x80
        } else {
            make_code
        };

        self.process_scancode_set1(s1)
    }

    /// Push an ASCII char directly to the buffer (e.g. from serial terminal or virtual keyboard).
    pub fn push_char(&mut self, c: char) {
        let event = KeyEvent {
            char_code: Some(c),
            scancode: 0,
            action: KeyAction::Press,
            shift: self.modifiers.is_shift(),
            ctrl: self.modifiers.is_ctrl(),
            alt: self.modifiers.is_alt(),
        };
        self.buffer.push_event(event);
    }

    /// Read next ASCII char from buffer.
    pub fn read_char(&mut self) -> Option<char> {
        self.buffer.pop_char()
    }
}

/// Polls PS/2 hardware ports (0x64 status and 0x60 data) for incoming byte.
pub fn poll_ps2_hardware() -> Option<u8> {
    unsafe {
        let status = crate::inb(0x64);
        if (status & 0x01) != 0 {
            Some(crate::inb(0x60))
        } else {
            None
        }
    }
}

fn scancode_set1_to_ascii(make_code: u8, shift: bool, caps_lock: bool) -> Option<char> {
    let alpha_shift = shift ^ caps_lock;

    let c = match make_code {
        0x01 => return None, // ESC
        0x02 => if shift { '!' } else { '1' },
        0x03 => if shift { '@' } else { '2' },
        0x04 => if shift { '#' } else { '3' },
        0x05 => if shift { '$' } else { '4' },
        0x06 => if shift { '%' } else { '5' },
        0x07 => if shift { '^' } else { '6' },
        0x08 => if shift { '&' } else { '7' },
        0x09 => if shift { '*' } else { '8' },
        0x0A => if shift { '(' } else { '9' },
        0x0B => if shift { ')' } else { '0' },
        0x0C => if shift { '_' } else { '-' },
        0x0D => if shift { '+' } else { '=' },
        0x0E => '\x08', // Backspace
        0x0F => '\t',   // Tab
        0x10 => if alpha_shift { 'Q' } else { 'q' },
        0x11 => if alpha_shift { 'W' } else { 'w' },
        0x12 => if alpha_shift { 'E' } else { 'e' },
        0x13 => if alpha_shift { 'R' } else { 'r' },
        0x14 => if alpha_shift { 'T' } else { 't' },
        0x15 => if alpha_shift { 'Y' } else { 'y' },
        0x16 => if alpha_shift { 'U' } else { 'u' },
        0x17 => if alpha_shift { 'I' } else { 'i' },
        0x18 => if alpha_shift { 'O' } else { 'o' },
        0x19 => if alpha_shift { 'P' } else { 'p' },
        0x1A => if shift { '{' } else { '[' },
        0x1B => if shift { '}' } else { ']' },
        0x1C => '\n', // Enter
        0x1E => if alpha_shift { 'A' } else { 'a' },
        0x1F => if alpha_shift { 'S' } else { 's' },
        0x20 => if alpha_shift { 'D' } else { 'd' },
        0x21 => if alpha_shift { 'F' } else { 'f' },
        0x22 => if alpha_shift { 'G' } else { 'g' },
        0x23 => if alpha_shift { 'H' } else { 'h' },
        0x24 => if alpha_shift { 'J' } else { 'j' },
        0x25 => if alpha_shift { 'K' } else { 'k' },
        0x26 => if alpha_shift { 'L' } else { 'l' },
        0x27 => if shift { ':' } else { ';' },
        0x28 => if shift { '"' } else { '\'' },
        0x29 => if shift { '~' } else { '`' },
        0x2B => if shift { '|' } else { '\\' },
        0x2C => if alpha_shift { 'Z' } else { 'z' },
        0x2D => if alpha_shift { 'X' } else { 'x' },
        0x2E => if alpha_shift { 'C' } else { 'c' },
        0x2F => if alpha_shift { 'V' } else { 'v' },
        0x30 => if alpha_shift { 'B' } else { 'b' },
        0x31 => if alpha_shift { 'N' } else { 'n' },
        0x32 => if alpha_shift { 'M' } else { 'm' },
        0x33 => if shift { '<' } else { ',' },
        0x34 => if shift { '>' } else { '.' },
        0x35 => if shift { '?' } else { '/' },
        0x39 => ' ', // Space
        _ => return None,
    };

    Some(c)
}

fn map_set2_to_set1(scancode: u8, _extended: bool) -> Option<u8> {
    match scancode {
        0x76 => Some(0x01), // ESC
        0x16 => Some(0x02), // 1
        0x1E => Some(0x03), // 2
        0x26 => Some(0x04), // 3
        0x25 => Some(0x05), // 4
        0x2E => Some(0x06), // 5
        0x36 => Some(0x07), // 6
        0x3D => Some(0x08), // 7
        0x3E => Some(0x09), // 8
        0x46 => Some(0x0A), // 9
        0x45 => Some(0x0B), // 0
        0x4E => Some(0x0C), // -
        0x55 => Some(0x0D), // =
        0x66 => Some(0x0E), // Backspace
        0x0D => Some(0x0F), // Tab
        0x15 => Some(0x10), // Q
        0x1D => Some(0x11), // W
        0x24 => Some(0x12), // E
        0x2D => Some(0x13), // R
        0x2C => Some(0x14), // T
        0x35 => Some(0x15), // Y
        0x3C => Some(0x16), // U
        0x43 => Some(0x17), // I
        0x44 => Some(0x18), // O
        0x4D => Some(0x19), // P
        0x5A => Some(0x1C), // Enter
        0x1C => Some(0x1E), // A
        0x1B => Some(0x1F), // S
        0x23 => Some(0x20), // D
        0x2B => Some(0x21), // F
        0x34 => Some(0x22), // G
        0x33 => Some(0x23), // H
        0x3B => Some(0x24), // J
        0x42 => Some(0x25), // K
        0x4B => Some(0x26), // L
        0x12 => Some(0x2A), // Left Shift
        0x1A => Some(0x2C), // Z
        0x22 => Some(0x2D), // X
        0x21 => Some(0x2E), // C
        0x2A => Some(0x2F), // V
        0x32 => Some(0x30), // B
        0x31 => Some(0x31), // N
        0x3A => Some(0x32), // M
        0x59 => Some(0x36), // Right Shift
        0x14 => Some(0x1D), // Ctrl
        0x11 => Some(0x38), // Alt
        0x29 => Some(0x39), // Space
        0x58 => Some(0x3A), // Caps Lock
        _ => None,
    }
}
