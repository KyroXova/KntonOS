//! Wayland Protocol Wire Encoding/Decoding & Core Interface Objects.
//!
//! Provides binary serialization for Wayland IPC wire format (object ID, opcode, size,
//! and strongly-typed arguments) and core Wayland interface object tables.

use alloc::vec::Vec;

/// Standard Wayland wire protocol message header:
/// - 32-bit sender object ID
/// - 16-bit opcode
/// - 16-bit message size in bytes (header + payload, 4-byte aligned)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WlMessageHeader {
    pub object_id: u32,
    pub opcode: u16,
    pub size: u16,
}

impl WlMessageHeader {
    pub const SIZE: usize = 8;

    pub const fn new(object_id: u32, opcode: u16, size: u16) -> Self {
        Self {
            object_id,
            opcode,
            size,
        }
    }

    /// Decodes a header from raw 8-byte slice.
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let object_id = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let opcode = u16::from_ne_bytes([buf[4], buf[5]]);
        let size = u16::from_ne_bytes([buf[6], buf[7]]);
        if (size as usize) < Self::SIZE || (size % 4) != 0 {
            return None;
        }
        Some(Self {
            object_id,
            opcode,
            size,
        })
    }

    /// Encodes header into raw 8-byte slice.
    pub fn encode(&self, buf: &mut [u8]) -> bool {
        if buf.len() < Self::SIZE {
            return false;
        }
        buf[0..4].copy_from_slice(&self.object_id.to_ne_bytes());
        buf[4..6].copy_from_slice(&self.opcode.to_ne_bytes());
        buf[6..8].copy_from_slice(&self.size.to_ne_bytes());
        true
    }
}

/// Strongly-typed argument in Wayland wire protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WlArg {
    Int(i32),
    Uint(u32),
    Fixed(i32), // 24.8 signed fixed-point
    String(Vec<u8>),
    Object(u32),
    NewId(u32),
    Array(Vec<u8>),
    Fd(i32),
}

/// Wayland wire message decoder.
pub struct WlDecoder<'a> {
    buf: &'a [u8],
    cursor: usize,
}

impl<'a> WlDecoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, cursor: 0 }
    }

    pub fn decode_header(&mut self) -> Option<WlMessageHeader> {
        if self.cursor + WlMessageHeader::SIZE > self.buf.len() {
            return None;
        }
        let hdr = WlMessageHeader::decode(&self.buf[self.cursor..self.cursor + WlMessageHeader::SIZE])?;
        self.cursor += WlMessageHeader::SIZE;
        Some(hdr)
    }

    pub fn read_u32(&mut self) -> Option<u32> {
        if self.cursor + 4 > self.buf.len() {
            return None;
        }
        let val = u32::from_ne_bytes([
            self.buf[self.cursor],
            self.buf[self.cursor + 1],
            self.buf[self.cursor + 2],
            self.buf[self.cursor + 3],
        ]);
        self.cursor += 4;
        Some(val)
    }

    pub fn read_i32(&mut self) -> Option<i32> {
        self.read_u32().map(|u| u as i32)
    }

    pub fn read_string(&mut self) -> Option<Vec<u8>> {
        let len = self.read_u32()? as usize;
        if self.cursor + len > self.buf.len() {
            return None;
        }
        let raw = &self.buf[self.cursor..self.cursor + len];
        // Strip trailing null byte if present
        let clean_len = if len > 0 && raw[len - 1] == 0 { len - 1 } else { len };
        let mut s = Vec::with_capacity(clean_len);
        s.extend_from_slice(&raw[..clean_len]);

        // Wayland strings are padded to 4-byte boundaries
        let padded = (len + 3) & !3;
        self.cursor += padded;
        Some(s)
    }
}

/// Wayland wire message encoder.
pub struct WlEncoder {
    buf: Vec<u8>,
}

impl WlEncoder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn finish_message(&mut self, object_id: u32, opcode: u16) {
        let size = self.buf.len() as u16;
        let hdr = WlMessageHeader::new(object_id, opcode, size);
        hdr.encode(&mut self.buf[0..8]);
    }

    pub fn start_message(&mut self, object_id: u32, opcode: u16) {
        self.buf.clear();
        let hdr = WlMessageHeader::new(object_id, opcode, 8);
        self.buf.resize(8, 0);
        hdr.encode(&mut self.buf[0..8]);
    }

    pub fn write_u32(&mut self, val: u32) {
        self.buf.extend_from_slice(&val.to_ne_bytes());
    }

    pub fn write_i32(&mut self, val: i32) {
        self.buf.extend_from_slice(&val.to_ne_bytes());
    }

    pub fn write_string(&mut self, s: &[u8]) {
        let len_with_null = (s.len() + 1) as u32;
        self.write_u32(len_with_null);
        self.buf.extend_from_slice(s);
        self.buf.push(0); // Null terminator
        while (self.buf.len() % 4) != 0 {
            self.buf.push(0); // 4-byte alignment padding
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
}

/// Well-known core Wayland interface types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WlInterfaceType {
    WlDisplay,
    WlRegistry,
    WlCompositor,
    WlSurface,
    WlShm,
    WlShmPool,
    WlBuffer,
    WlSubcompositor,
    WlSubsurface,
    WlCallback,
}

/// Metadata record for an active Wayland object instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WlObjectEntry {
    pub id: u32,
    pub interface: WlInterfaceType,
    pub version: u32,
}

impl WlObjectEntry {
    pub const fn new(id: u32, interface: WlInterfaceType, version: u32) -> Self {
        Self { id, interface, version }
    }
}

pub const MAX_WAYLAND_OBJECTS: usize = 64;

/// Object table mapping 32-bit Wayland IDs to interface definitions.
pub struct WlObjectTable {
    objects: [Option<WlObjectEntry>; MAX_WAYLAND_OBJECTS],
    count: usize,
}

impl WlObjectTable {
    pub const fn new() -> Self {
        Self {
            objects: [None; MAX_WAYLAND_OBJECTS],
            count: 0,
        }
    }

    pub fn insert(&mut self, entry: WlObjectEntry) -> bool {
        for slot in self.objects.iter_mut() {
            if slot.is_none() {
                *slot = Some(entry);
                self.count += 1;
                return true;
            }
        }
        false
    }

    pub fn remove(&mut self, id: u32) -> bool {
        for slot in self.objects.iter_mut() {
            if let Some(obj) = slot {
                if obj.id == id {
                    *slot = None;
                    self.count = self.count.saturating_sub(1);
                    return true;
                }
            }
        }
        false
    }

    pub fn lookup(&self, id: u32) -> Option<WlObjectEntry> {
        for slot in self.objects.iter() {
            if let Some(obj) = slot {
                if obj.id == id {
                    return Some(*obj);
                }
            }
        }
        None
    }

    #[inline(always)]
    pub const fn count(&self) -> usize {
        self.count
    }
}

/// Unit test verifying Wayland wire message encoding/decoding and object table lookups.
pub fn test_wayland_wire_and_object_table() -> bool {
    // 1. Test Wire Encoding
    let mut enc = WlEncoder::new();
    enc.start_message(1, 1); // wl_display.get_registry (new_id = 2)
    enc.write_u32(2);
    enc.finish_message(1, 1);

    let bytes = enc.as_bytes();
    if bytes.len() != 12 {
        return false;
    }

    // 2. Test Wire Decoding
    let mut dec = WlDecoder::new(bytes);
    let hdr = match dec.decode_header() {
        Some(h) => h,
        None => return false,
    };
    if hdr.object_id != 1 || hdr.opcode != 1 || hdr.size != 12 {
        return false;
    }
    let new_id = match dec.read_u32() {
        Some(id) => id,
        None => return false,
    };
    if new_id != 2 {
        return false;
    }

    // 3. Test String Encoding / Decoding
    let mut str_enc = WlEncoder::new();
    str_enc.start_message(2, 0); // wl_registry.global(name=1, interface="wl_compositor", version=4)
    str_enc.write_u32(1);
    str_enc.write_string(b"wl_compositor");
    str_enc.write_u32(4);
    str_enc.finish_message(2, 0);

    let str_bytes = str_enc.as_bytes();
    let mut str_dec = WlDecoder::new(str_bytes);
    let _ = str_dec.decode_header();
    let name = str_dec.read_u32().unwrap_or(0);
    let iface = str_dec.read_string().unwrap_or_default();
    let ver = str_dec.read_u32().unwrap_or(0);

    if name != 1 || iface != b"wl_compositor" || ver != 4 {
        return false;
    }

    // 4. Test Object Table
    let mut table = WlObjectTable::new();
    table.insert(WlObjectEntry::new(1, WlInterfaceType::WlDisplay, 1));
    table.insert(WlObjectEntry::new(2, WlInterfaceType::WlRegistry, 1));
    table.insert(WlObjectEntry::new(3, WlInterfaceType::WlCompositor, 4));

    if table.count() != 3 {
        return false;
    }

    let comp = match table.lookup(3) {
        Some(o) => o,
        None => return false,
    };
    if comp.interface != WlInterfaceType::WlCompositor || comp.version != 4 {
        return false;
    }

    table.remove(2);
    if table.count() != 2 || table.lookup(2).is_some() {
        return false;
    }

    true
}
