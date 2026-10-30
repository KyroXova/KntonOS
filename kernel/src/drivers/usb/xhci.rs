//! USB 3.0 Extensible Host Controller Interface (xHCI) Driver & Discovery.

use crate::drivers::pci_scanner::PciDevice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhciError {
    NotXhciDevice,
    MissingBar0,
    InvalidCapabilityLength,
    InvalidVersion,
    MmioAccessFailed,
    RingFull,
    InvalidSlotId,
}

/// Decoded xHCI Host Controller capability registers (read-only interface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XhciCapabilities {
    pub caplength: u8,
    pub hciversion: u16,
    pub max_slots: u8,
    pub max_interrupters: u16,
    pub max_ports: u8,
    pub ac64: bool,
    pub context_size_64: bool,
    pub xecp: u16,
    pub doorbell_offset: u32,
    pub runtime_offset: u32,
}

impl XhciCapabilities {
    /// Parse capabilities from raw 32-bit registers.
    pub fn parse(
        caplength_version: u32,
        hcsparams1: u32,
        hccparams1: u32,
        dboff: u32,
        rtsoff: u32,
    ) -> Result<Self, XhciError> {
        let caplength = (caplength_version & 0xFF) as u8;
        let hciversion = ((caplength_version >> 16) & 0xFFFF) as u16;

        if caplength < 0x20 {
            return Err(XhciError::InvalidCapabilityLength);
        }

        let max_slots = (hcsparams1 & 0xFF) as u8;
        let max_interrupters = ((hcsparams1 >> 8) & 0x7FF) as u16;
        let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;

        let ac64 = (hccparams1 & 0x01) != 0;
        let context_size_64 = (hccparams1 & 0x04) != 0;
        let xecp = ((hccparams1 >> 16) & 0xFFFF) as u16;

        Ok(Self {
            caplength,
            hciversion,
            max_slots,
            max_interrupters,
            max_ports,
            ac64,
            context_size_64,
            xecp,
            doorbell_offset: dboff & !0x03,
            runtime_offset: rtsoff & !0x1F,
        })
    }
}

/// Transfer Request Block (TRB) Types defined by xHCI specification.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrbType {
    Normal = 1,
    SetupStage = 2,
    DataStage = 3,
    StatusStage = 4,
    Link = 6,
    EnableSlotCommand = 9,
    DisableSlotCommand = 10,
    AddressDeviceCommand = 11,
    ConfigureEndpointCommand = 12,
    EvaluateContextCommand = 13,
    ResetEndpointCommand = 14,
    StopEndpointCommand = 15,
    SetTrDequeuePointerCommand = 16,
    ResetDeviceCommand = 17,
    NoOpCommand = 23,
    TransferEvent = 32,
    CommandCompletionEvent = 33,
    PortStatusChangeEvent = 34,
    Unknown = 0,
}

impl From<u8> for TrbType {
    fn from(val: u8) -> Self {
        match val {
            1 => TrbType::Normal,
            2 => TrbType::SetupStage,
            3 => TrbType::DataStage,
            4 => TrbType::StatusStage,
            6 => TrbType::Link,
            9 => TrbType::EnableSlotCommand,
            10 => TrbType::DisableSlotCommand,
            11 => TrbType::AddressDeviceCommand,
            12 => TrbType::ConfigureEndpointCommand,
            13 => TrbType::EvaluateContextCommand,
            14 => TrbType::ResetEndpointCommand,
            15 => TrbType::StopEndpointCommand,
            16 => TrbType::SetTrDequeuePointerCommand,
            17 => TrbType::ResetDeviceCommand,
            23 => TrbType::NoOpCommand,
            32 => TrbType::TransferEvent,
            33 => TrbType::CommandCompletionEvent,
            34 => TrbType::PortStatusChangeEvent,
            _ => TrbType::Unknown,
        }
    }
}

/// 16-byte Transfer Request Block (TRB) aligned to 16 bytes.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Trb {
    pub parameter: u64,
    pub status: u32,
    pub control: u32,
}

impl Trb {
    pub const fn new(parameter: u64, status: u32, control: u32) -> Self {
        Self { parameter, status, control }
    }

    pub fn trb_type(&self) -> TrbType {
        TrbType::from(((self.control >> 10) & 0x3F) as u8)
    }

    pub fn cycle(&self) -> bool {
        (self.control & 0x01) != 0
    }

    pub fn set_cycle(&mut self, cycle: bool) {
        if cycle {
            self.control |= 0x01;
        } else {
            self.control &= !0x01;
        }
    }

    /// Construct a No-Op Command TRB (TRB type 23).
    pub fn no_op_command(cycle: bool) -> Self {
        let control = ((TrbType::NoOpCommand as u32) << 10) | if cycle { 1 } else { 0 };
        Self::new(0, 0, control)
    }

    /// Construct an Enable Slot Command TRB (TRB type 9).
    pub fn enable_slot_command(slot_type: u8, cycle: bool) -> Self {
        let control = ((slot_type as u32 & 0x1F) << 16)
            | ((TrbType::EnableSlotCommand as u32) << 10)
            | if cycle { 1 } else { 0 };
        Self::new(0, 0, control)
    }

    /// Construct an Address Device Command TRB (TRB type 11).
    pub fn address_device_command(input_context_phys: u64, slot_id: u8, bsr: bool, cycle: bool) -> Self {
        let control = ((slot_id as u32) << 24)
            | (if bsr { 1 << 9 } else { 0 })
            | ((TrbType::AddressDeviceCommand as u32) << 10)
            | if cycle { 1 } else { 0 };
        Self::new(input_context_phys, 0, control)
    }

    /// Construct a Configure Endpoint Command TRB (TRB type 12).
    pub fn configure_endpoint_command(
        input_context_phys: u64,
        slot_id: u8,
        deconfigure: bool,
        cycle: bool,
    ) -> Self {
        let control = ((slot_id as u32) << 24)
            | (if deconfigure { 1 << 9 } else { 0 })
            | ((TrbType::ConfigureEndpointCommand as u32) << 10)
            | if cycle { 1 } else { 0 };
        Self::new(input_context_phys, 0, control)
    }

    /// Construct a Link TRB (TRB type 6) pointing to the ring base.
    pub fn link(ring_phys: u64, toggle_cycle: bool, cycle: bool) -> Self {
        let control = (if toggle_cycle { 1 << 1 } else { 0 })
            | ((TrbType::Link as u32) << 10)
            | if cycle { 1 } else { 0 };
        Self::new(ring_phys, 0, control)
    }

    /// Decode an event TRB into high-level event variants.
    pub fn decode_event(&self) -> Option<XhciEvent> {
        let t = ((self.control >> 10) & 0x3F) as u8;
        let completion_code = ((self.status >> 24) & 0xFF) as u8;
        let slot_id = ((self.control >> 24) & 0xFF) as u8;

        match t {
            32 => {
                let remaining_len = self.status & 0x00FF_FFFF;
                let endpoint_id = ((self.control >> 16) & 0x1F) as u8;
                Some(XhciEvent::Transfer {
                    trb_ptr: self.parameter,
                    remaining_len,
                    completion_code,
                    slot_id,
                    endpoint_id,
                })
            }
            33 => {
                let param = self.status & 0x00FF_FFFF;
                Some(XhciEvent::CommandCompletion {
                    cmd_trb_ptr: self.parameter,
                    param,
                    completion_code,
                    slot_id,
                })
            }
            34 => {
                let port_id = ((self.parameter >> 24) & 0xFF) as u8;
                Some(XhciEvent::PortStatusChange {
                    port_id,
                    completion_code,
                })
            }
            _ => Some(XhciEvent::Other {
                trb_type: t,
                completion_code,
                slot_id,
            }),
        }
    }
}

/// Decoded Event TRB representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhciEvent {
    Transfer {
        trb_ptr: u64,
        remaining_len: u32,
        completion_code: u8,
        slot_id: u8,
        endpoint_id: u8,
    },
    CommandCompletion {
        cmd_trb_ptr: u64,
        param: u32,
        completion_code: u8,
        slot_id: u8,
    },
    PortStatusChange {
        port_id: u8,
        completion_code: u8,
    },
    Other {
        trb_type: u8,
        completion_code: u8,
        slot_id: u8,
    },
}

/// Event Ring Segment Table (ERST) entry (16 bytes, 64-byte aligned).
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ErstEntry {
    pub ring_segment_base_address: u64,
    pub ring_segment_size: u32,
    pub reserved: u32,
}

impl ErstEntry {
    pub const fn new(base_addr: u64, size: u32) -> Self {
        Self {
            ring_segment_base_address: base_addr,
            ring_segment_size: size,
            reserved: 0,
        }
    }
}

pub const MAX_SLOTS_SUPPORTED: usize = 32;

/// Device Context Base Address Array (DCBAA).
#[repr(C, align(64))]
#[derive(Debug, Clone)]
pub struct Dcbaa {
    pub entries: [u64; MAX_SLOTS_SUPPORTED + 1],
}

impl Dcbaa {
    pub const fn new() -> Self {
        Self {
            entries: [0u64; MAX_SLOTS_SUPPORTED + 1],
        }
    }

    pub fn set_device_context(&mut self, slot_id: usize, phys_addr: u64) -> Result<(), XhciError> {
        if slot_id > MAX_SLOTS_SUPPORTED {
            return Err(XhciError::InvalidSlotId);
        }
        self.entries[slot_id] = phys_addr;
        Ok(())
    }

    pub fn get_device_context(&self, slot_id: usize) -> u64 {
        if slot_id <= MAX_SLOTS_SUPPORTED {
            self.entries[slot_id]
        } else {
            0
        }
    }
}

/// Standard 32-byte Slot Context structure.
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotContext {
    pub dword0: u32,
    pub dword1: u32,
    pub dword2: u32,
    pub dword3: u32,
    pub reserved: [u32; 4],
}

impl SlotContext {
    pub fn new(speed: u8, context_entries: u8, root_port: u8) -> Self {
        let dword0 = ((speed as u32 & 0x0F) << 20) | ((context_entries as u32 & 0x1F) << 27);
        let dword1 = (root_port as u32 & 0xFF) << 16;
        Self {
            dword0,
            dword1,
            dword2: 0,
            dword3: 0,
            reserved: [0; 4],
        }
    }

    pub fn speed(&self) -> u8 {
        ((self.dword0 >> 20) & 0x0F) as u8
    }

    pub fn context_entries(&self) -> u8 {
        ((self.dword0 >> 27) & 0x1F) as u8
    }

    pub fn root_port(&self) -> u8 {
        ((self.dword1 >> 16) & 0xFF) as u8
    }

    pub fn usb_device_address(&self) -> u8 {
        (self.dword3 & 0xFF) as u8
    }

    pub fn slot_state(&self) -> u8 {
        ((self.dword3 >> 27) & 0x1F) as u8
    }
}

/// Standard 32-byte Endpoint Context structure.
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EndpointContext {
    pub dword0: u32,
    pub dword1: u32,
    pub tr_dequeue_lo: u32,
    pub tr_dequeue_hi: u32,
    pub dword4: u32,
    pub reserved: [u32; 3],
}

impl EndpointContext {
    pub fn new_control_ep0(max_packet_size: u16, ring_phys: u64) -> Self {
        let ep_type = 4u32; // Control Bidirectional
        let cerr = 3u32;     // 3 retry errors
        let dword1 = (cerr << 1) | (ep_type << 3) | ((max_packet_size as u32) << 16);
        let tr_dequeue_lo = (ring_phys as u32) | 1; // DCS = 1
        let tr_dequeue_hi = (ring_phys >> 32) as u32;
        let dword4 = 8;     // average TRB length 8 bytes

        Self {
            dword0: 0,
            dword1,
            tr_dequeue_lo,
            tr_dequeue_hi,
            dword4,
            reserved: [0; 3],
        }
    }

    pub fn new_interrupt_in(max_packet_size: u16, interval: u8, ring_phys: u64) -> Self {
        let ep_type = 7u32; // Interrupt In
        let cerr = 3u32;
        let dword0 = (interval as u32) << 16;
        let dword1 = (cerr << 1) | (ep_type << 3) | ((max_packet_size as u32) << 16);
        let tr_dequeue_lo = (ring_phys as u32) | 1; // DCS = 1
        let tr_dequeue_hi = (ring_phys >> 32) as u32;
        let dword4 = max_packet_size as u32;

        Self {
            dword0,
            dword1,
            tr_dequeue_lo,
            tr_dequeue_hi,
            dword4,
            reserved: [0; 3],
        }
    }

    pub fn ep_state(&self) -> u8 {
        (self.dword0 & 0x07) as u8
    }

    pub fn ep_type(&self) -> u8 {
        ((self.dword1 >> 3) & 0x07) as u8
    }

    pub fn max_packet_size(&self) -> u16 {
        ((self.dword1 >> 16) & 0xFFFF) as u16
    }
}

/// Input Control Context (32 bytes).
#[repr(C, align(32))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputControlContext {
    pub drop_flags: u32,
    pub add_flags: u32,
    pub reserved: [u32; 6],
}

/// Device Context (Slot Context + 31 Endpoint Contexts).
#[repr(C, align(64))]
#[derive(Debug, Clone, Default)]
pub struct DeviceContext {
    pub slot: SlotContext,
    pub endpoints: [EndpointContext; 31],
}

/// Input Context (Input Control Context + Slot Context + 31 Endpoint Contexts).
#[repr(C, align(64))]
#[derive(Debug, Clone, Default)]
pub struct InputContext {
    pub control: InputControlContext,
    pub slot: SlotContext,
    pub endpoints: [EndpointContext; 31],
}

pub const COMMAND_RING_SIZE: usize = 32;

/// xHCI Command Ring buffer with cycle tracking.
pub struct CommandRing {
    pub trbs: [Trb; COMMAND_RING_SIZE],
    pub enqueue_index: usize,
    pub cycle_state: bool,
}

impl CommandRing {
    pub fn new() -> Self {
        let mut ring = Self {
            trbs: [Trb::default(); COMMAND_RING_SIZE],
            enqueue_index: 0,
            cycle_state: true,
        };
        ring.trbs[COMMAND_RING_SIZE - 1] = Trb::link(0, true, false);
        ring
    }

    pub fn push(&mut self, mut trb: Trb) -> Result<usize, XhciError> {
        if self.enqueue_index >= COMMAND_RING_SIZE - 1 {
            let mut link_trb = self.trbs[COMMAND_RING_SIZE - 1];
            link_trb.set_cycle(self.cycle_state);
            self.trbs[COMMAND_RING_SIZE - 1] = link_trb;

            self.enqueue_index = 0;
            self.cycle_state = !self.cycle_state;
        }

        trb.set_cycle(self.cycle_state);
        let idx = self.enqueue_index;
        self.trbs[idx] = trb;
        self.enqueue_index += 1;
        Ok(idx)
    }
}

pub const EVENT_RING_SIZE: usize = 32;

/// xHCI Event Ring buffer with consumer dequeue tracking.
pub struct EventRing {
    pub trbs: [Trb; EVENT_RING_SIZE],
    pub dequeue_index: usize,
    pub cycle_state: bool,
}

impl EventRing {
    pub fn new() -> Self {
        Self {
            trbs: [Trb::default(); EVENT_RING_SIZE],
            dequeue_index: 0,
            cycle_state: true,
        }
    }

    pub fn poll_event(&mut self) -> Option<XhciEvent> {
        let trb = &self.trbs[self.dequeue_index];
        if trb.cycle() != self.cycle_state {
            return None;
        }

        let event = trb.decode_event();
        self.dequeue_index += 1;
        if self.dequeue_index >= EVENT_RING_SIZE {
            self.dequeue_index = 0;
            self.cycle_state = !self.cycle_state;
        }

        event
    }

    pub fn push_event_synthetic(&mut self, mut trb: Trb) {
        trb.set_cycle(self.cycle_state);
        let idx = self.dequeue_index;
        self.trbs[idx] = trb;
    }
}

/// Represents an enumerated USB 3.0 xHCI Host Controller instance.
#[derive(Debug, Clone)]
pub struct XhciController {
    pub pci_bus: u8,
    pub pci_slot: u8,
    pub pci_func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub mmio_base: u64,
    pub mmio_size: u64,
    pub op_base: u64,
    pub caps: XhciCapabilities,
}

impl XhciController {
    /// Attempts to instantiate an xHCI controller from an enumerated PCI device.
    pub fn from_pci(dev: &PciDevice) -> Result<Self, XhciError> {
        // Verify class 0x0C (Serial Bus), subclass 0x03 (USB), prog_if 0x30 (xHCI)
        if dev.class_code != 0x0C || dev.subclass != 0x03 || dev.prog_if != 0x30 {
            return Err(XhciError::NotXhciDevice);
        }

        let bar0 = dev.get_bar(0).ok_or(XhciError::MissingBar0)?;
        let mmio_base = bar0.address;
        let mmio_size = bar0.size;

        if mmio_base == 0 {
            return Err(XhciError::MissingBar0);
        }

        // Read capability registers from MMIO (or safely fallback if in virtual mode)
        let caps = unsafe { Self::read_capabilities_mmio(mmio_base) }?;
        let op_base = mmio_base + caps.caplength as u64;

        Ok(Self {
            pci_bus: dev.bus,
            pci_slot: dev.slot,
            pci_func: dev.function,
            vendor_id: dev.vendor_id,
            device_id: dev.device_id,
            mmio_base,
            mmio_size,
            op_base,
            caps,
        })
    }

    /// Read and decode xHCI capability registers from physical MMIO pointer.
    pub unsafe fn read_capabilities_mmio(mmio_base: u64) -> Result<XhciCapabilities, XhciError> {
        let ptr = mmio_base as *const u32;

        let caplength_version = core::ptr::read_volatile(ptr.offset(0));
        let hcsparams1 = core::ptr::read_volatile(ptr.offset(1));
        let hccparams1 = core::ptr::read_volatile(ptr.offset(4));
        let dboff = core::ptr::read_volatile(ptr.offset(5));
        let rtsoff = core::ptr::read_volatile(ptr.offset(6));

        XhciCapabilities::parse(caplength_version, hcsparams1, hccparams1, dboff, rtsoff)
    }

    /// Friendly formatted version string.
    pub fn version_string(&self) -> &'static str {
        match self.caps.hciversion {
            0x0096 => "xHCI 0.96",
            0x0100 => "xHCI 1.0",
            0x0110 => "xHCI 1.1",
            0x0120 => "xHCI 1.2",
            _ => "xHCI 3.x",
        }
    }
}

