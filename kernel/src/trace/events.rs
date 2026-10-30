//! Kernel Trace Record Structures, Event Type Enumeration, & Binary Serialization.
//! Provides zero-allocation, fixed-size 32-byte trace record framing with high-resolution

/// Well-known kernel profiling event classifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TraceEventType {
    /// Scheduler thread context switch (prev_tid -> next_tid).
    SchedContextSwitch = 1,
    /// Syscall entry and dispatch (syscall_id, arg0).
    SyscallEntry = 2,
    /// Syscall exit and return value (syscall_id, duration_ns).
    SyscallExit = 3,
    /// Page fault (#PF) exception (fault_vaddr, error_code).
    PageFault = 4,
    /// IPC shared-memory ring packet transfer (channel_id, bytes).
    IpcTransfer = 5,
    /// Hardware IRQ or Local APIC timer interrupt duration (vector, duration_ns).
    IrqDuration = 6,
    /// Dynamic power governor PL1 TDP or frequency adjustment.
    PowerGovernorStep = 7,
    /// Transparent Huge Page 2MB collapse or split event.
    ThpCollapseSplit = 8,
    /// Custom diagnostic mark or breakpoint.
    DiagnosticMark = 9,
}

impl TraceEventType {
    pub const fn from_u16(val: u16) -> Option<Self> {
        match val {
            1 => Some(Self::SchedContextSwitch),
            2 => Some(Self::SyscallEntry),
            3 => Some(Self::SyscallExit),
            4 => Some(Self::PageFault),
            5 => Some(Self::IpcTransfer),
            6 => Some(Self::IrqDuration),
            7 => Some(Self::PowerGovernorStep),
            8 => Some(Self::ThpCollapseSplit),
            9 => Some(Self::DiagnosticMark),
            _ => None,
        }
    }
}

/// Bitmask representation for dynamic event filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceEventMask(pub u32);

impl TraceEventMask {
    pub const ALL: Self = Self(0xFFFF_FFFF);
    pub const NONE: Self = Self(0);

    pub const fn from_event(event: TraceEventType) -> Self {
        Self(1 << (event as u16))
    }

    #[inline(always)]
    pub fn contains(&self, event: TraceEventType) -> bool {
        (self.0 & (1 << (event as u16))) != 0
    }

    #[inline(always)]
    pub fn enable(&mut self, event: TraceEventType) {
        self.0 |= 1 << (event as u16);
    }

    #[inline(always)]
    pub fn disable(&mut self, event: TraceEventType) {
        self.0 &= !(1 << (event as u16));
    }
}

/// Magic sentinel for trace record validation.
pub const TRACE_RECORD_MAGIC: u16 = 0x5452; // "TR"

/// Fixed-size 32-byte zero-allocation kernel trace event record.
/// Designed to pack cleanly into L1 cache lines (two records per 64-byte line)
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRecord {
    /// Validation magic (`TRACE_RECORD_MAGIC`).
    pub magic: u16,
    /// Event type identifier (`TraceEventType`).
    pub event_type: u16,
    /// Logical CPU core ID emitting this event.
    pub core_id: u16,
    /// Flags and event-specific status bits.
    pub flags: u16,
    /// High-resolution timestamp (nanoseconds since boot or raw TSC).
    pub timestamp_ns: u64,
    /// Event-specific primary payload (e.g. TID, address, syscall number).
    pub payload_a: u64,
    /// Event-specific secondary payload (e.g. duration, error code, bytes).
    pub payload_b: u64,
}

impl TraceRecord {
    pub const SIZE: usize = core::mem::size_of::<Self>(); // Exactly 32 bytes

    pub const fn empty() -> Self {
        Self {
            magic: 0,
            event_type: 0,
            core_id: 0,
            flags: 0,
            timestamp_ns: 0,
            payload_a: 0,
            payload_b: 0,
        }
    }

    pub const fn new(
        event_type: TraceEventType,
        core_id: u16,
        flags: u16,
        timestamp_ns: u64,
        payload_a: u64,
        payload_b: u64,
    ) -> Self {
        Self {
            magic: TRACE_RECORD_MAGIC,
            event_type: event_type as u16,
            core_id,
            flags,
            timestamp_ns,
            payload_a,
            payload_b,
        }
    }

    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        self.magic == TRACE_RECORD_MAGIC
    }

    #[inline(always)]
    pub fn event(&self) -> Option<TraceEventType> {
        TraceEventType::from_u16(self.event_type)
    }

    /// Serializes the record into a 32-byte destination slice.
    pub fn serialize(&self, dest: &mut [u8]) -> bool {
        if dest.len() < Self::SIZE {
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                self as *const Self as *const u8,
                dest.as_mut_ptr(),
                Self::SIZE,
            );
        }
        true
    }

    /// Deserializes a record from a 32-byte slice.
    pub fn deserialize(src: &[u8]) -> Option<Self> {
        if src.len() < Self::SIZE {
            return None;
        }
        let record = unsafe { core::ptr::read_unaligned(src.as_ptr() as *const Self) };
        if record.is_valid() {
            Some(record)
        } else {
            None
        }
    }
}

/// Unit test verifying trace record structures, size invariants, and binary serialization.
pub fn test_trace_record_serialization() -> bool {
    // Assert 32-byte size invariant
    if TraceRecord::SIZE != 32 {
        return false;
    }

    let record = TraceRecord::new(
        TraceEventType::SchedContextSwitch,
        3,      // Core 3
        0x01,   // Flags
        123_456_789,
        4,      // prev_tid = 4
        7,      // next_tid = 7
    );

    if !record.is_valid() || record.event() != Some(TraceEventType::SchedContextSwitch) {
        return false;
    }

    let mut raw_buf = [0u8; 32];
    if !record.serialize(&mut raw_buf) {
        return false;
    }

    let parsed = match TraceRecord::deserialize(&raw_buf) {
        Some(r) => r,
        None => return false,
    };

    if parsed != record {
        return false;
    }
    if parsed.core_id != 3 || parsed.payload_a != 4 || parsed.payload_b != 7 {
        return false;
    }

    // Verify corrupt magic rejection
    raw_buf[0] = 0;
    if TraceRecord::deserialize(&raw_buf).is_some() {
        return false;
    }

    // Verify event mask
    let mut mask = TraceEventMask::NONE;
    mask.enable(TraceEventType::PageFault);
    mask.enable(TraceEventType::IpcTransfer);
    if !mask.contains(TraceEventType::PageFault) || !mask.contains(TraceEventType::IpcTransfer) {
        return false;
    }
    if mask.contains(TraceEventType::SchedContextSwitch) {
        return false;
    }

    true
}
