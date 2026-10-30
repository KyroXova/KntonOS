//! NVMe Zoned Namespaces (ZNS) Device Subsystem & Sequential Zone State Machine.
//!
//! Implements modern NVMe Zoned Namespaces (ZNS TP 4053 / NVMe 2.0 Command Set):
//! - Sequential Write Required (SWR) zone geometry layout
//! - Standard ZNS Zone State Machine (Empty, Implicitly Opened, Explicitly Opened, Closed, Full, Read Only, Offline)
//! - Active and Open zone capacity limit tracking and resource governance
//! - Zone Management Send and Receive command entry construction

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;
use crate::drivers::storage::nvme::NvmeSubmissionEntry;

// -----------------------------------------------------------------------------
// ZNS Command Opcodes & Action Definitions
// -----------------------------------------------------------------------------

/// NVMe ZNS Command Opcodes.
pub const NVME_ZNS_MANAGEMENT_SEND: u8 = 0x79;
pub const NVME_ZNS_MANAGEMENT_RECEIVE: u8 = 0x7A;
pub const NVME_ZNS_ZONE_APPEND: u8 = 0x7D;

/// Zone Management Send Actions (ZSA).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneSendAction {
    Close = 0x01,
    Finish = 0x02,
    Open = 0x03,
    ResetWritePointer = 0x04,
    Offline = 0x05,
    SetZoneAlternativeState = 0x10,
}

/// ZNS Zone Types.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneType {
    Reserved = 0x00,
    SequentialWriteRequired = 0x02,
}

/// Standard ZNS Zone State Machine definitions (NVMe ZNS specification).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneState {
    Empty = 0x01,
    ImplicitlyOpened = 0x02,
    ExplicitlyOpened = 0x03,
    Closed = 0x04,
    ReadOnly = 0x0D,
    Full = 0x0E,
    Offline = 0x0F,
}

// -----------------------------------------------------------------------------
// Zone Descriptor
// -----------------------------------------------------------------------------

/// Descriptor tracking physical geometry, write pointer, and runtime state of a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneDescriptor {
    pub zone_index: u32,
    pub zone_type: ZoneType,
    pub state: ZoneState,
    pub start_lba: u64,
    pub zone_size_lbas: u64,
    pub capacity_lbas: u64,
    pub write_pointer: u64,
}

impl ZoneDescriptor {
    pub const fn new(
        zone_index: u32,
        start_lba: u64,
        zone_size_lbas: u64,
        capacity_lbas: u64,
    ) -> Self {
        Self {
            zone_index,
            zone_type: ZoneType::SequentialWriteRequired,
            state: ZoneState::Empty,
            start_lba,
            zone_size_lbas,
            capacity_lbas,
            write_pointer: start_lba,
        }
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.state == ZoneState::Empty
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.state == ZoneState::Full
    }

    #[inline(always)]
    pub fn is_open(&self) -> bool {
        self.state == ZoneState::ImplicitlyOpened || self.state == ZoneState::ExplicitlyOpened
    }

    #[inline(always)]
    pub fn is_active(&self) -> bool {
        match self.state {
            ZoneState::ImplicitlyOpened | ZoneState::ExplicitlyOpened | ZoneState::Closed => true,
            _ => false,
        }
    }

    /// Number of writeable LBAs remaining in this zone before reaching capacity.
    #[inline(always)]
    pub fn remaining_capacity(&self) -> u64 {
        let max_lba = self.start_lba + self.capacity_lbas;
        if self.write_pointer >= max_lba {
            0
        } else {
            max_lba - self.write_pointer
        }
    }
}

// -----------------------------------------------------------------------------
// ZNS Namespace & Zone State Machine
// -----------------------------------------------------------------------------

/// In-kernel NVMe Zoned Namespace device abstraction.
#[derive(Debug, Clone)]
pub struct ZnsNamespace {
    pub nsid: u32,
    pub total_lbas: u64,
    pub zone_size_lbas: u64,
    pub zone_capacity_lbas: u64,
    pub max_active_zones: u32,
    pub max_open_zones: u32,
    pub zones: Vec<ZoneDescriptor>,
}

impl ZnsNamespace {
    /// Creates a new ZNS namespace partitioned into sequential zones.
    pub fn new(
        nsid: u32,
        total_lbas: u64,
        zone_size_lbas: u64,
        zone_capacity_lbas: u64,
        max_active_zones: u32,
        max_open_zones: u32,
    ) -> Self {
        let num_zones = (total_lbas / zone_size_lbas) as usize;
        let mut zones = Vec::with_capacity(num_zones);
        for i in 0..num_zones {
            let start_lba = (i as u64) * zone_size_lbas;
            zones.push(ZoneDescriptor::new(
                i as u32,
                start_lba,
                zone_size_lbas,
                zone_capacity_lbas,
            ));
        }

        Self {
            nsid,
            total_lbas,
            zone_size_lbas,
            zone_capacity_lbas,
            max_active_zones,
            max_open_zones,
            zones,
        }
    }

    pub fn zone_count(&self) -> usize {
        self.zones.len()
    }

    pub fn active_zone_count(&self) -> u32 {
        self.zones.iter().filter(|z| z.is_active()).count() as u32
    }

    pub fn open_zone_count(&self) -> u32 {
        self.zones.iter().filter(|z| z.is_open()).count() as u32
    }

    /// Transitions a zone to Explicitly Opened or Implicitly Opened.
    pub fn open_zone(&mut self, zone_index: u32, explicit: bool) -> Result<(), &'static str> {
        let current_state = self.zones.get(zone_index as usize).ok_or("Zone index out of bounds")?.state;
        match current_state {
            ZoneState::Full => return Err("Cannot open a Full zone"),
            ZoneState::ReadOnly | ZoneState::Offline => return Err("Zone is in ReadOnly or Offline state"),
            ZoneState::ExplicitlyOpened | ZoneState::ImplicitlyOpened => {
                if explicit && current_state == ZoneState::ImplicitlyOpened {
                    self.zones[zone_index as usize].state = ZoneState::ExplicitlyOpened;
                }
                Ok(())
            }
            ZoneState::Empty | ZoneState::Closed => {
                if current_state == ZoneState::Empty && self.active_zone_count() >= self.max_active_zones {
                    return Err("Maximum active zone limit exceeded");
                }
                if self.open_zone_count() >= self.max_open_zones {
                    return Err("Maximum open zone limit exceeded");
                }
                self.zones[zone_index as usize].state = if explicit {
                    ZoneState::ExplicitlyOpened
                } else {
                    ZoneState::ImplicitlyOpened
                };
                Ok(())
            }
        }
    }

    /// Transitions an open zone to Closed.
    pub fn close_zone(&mut self, zone_index: u32) -> Result<(), &'static str> {
        let zone = self.zones.get_mut(zone_index as usize).ok_or("Zone index out of bounds")?;
        match zone.state {
            ZoneState::ImplicitlyOpened | ZoneState::ExplicitlyOpened => {
                zone.state = ZoneState::Closed;
                Ok(())
            }
            ZoneState::Closed => Ok(()),
            _ => Err("Zone is not in an open state to close"),
        }
    }

    /// Transitions an open or closed zone to Full by advancing write pointer to capacity.
    pub fn finish_zone(&mut self, zone_index: u32) -> Result<(), &'static str> {
        let zone = self.zones.get_mut(zone_index as usize).ok_or("Zone index out of bounds")?;
        match zone.state {
            ZoneState::Empty | ZoneState::ImplicitlyOpened | ZoneState::ExplicitlyOpened | ZoneState::Closed => {
                zone.write_pointer = zone.start_lba + zone.capacity_lbas;
                zone.state = ZoneState::Full;
                Ok(())
            }
            ZoneState::Full => Ok(()),
            _ => Err("Zone cannot be finished from current state"),
        }
    }

    /// Resets the write pointer of a zone back to start_lba, transitioning to Empty.
    pub fn reset_zone(&mut self, zone_index: u32) -> Result<(), &'static str> {
        let zone = self.zones.get_mut(zone_index as usize).ok_or("Zone index out of bounds")?;
        match zone.state {
            ZoneState::ReadOnly | ZoneState::Offline => Err("Cannot reset ReadOnly or Offline zone"),
            _ => {
                zone.write_pointer = zone.start_lba;
                zone.state = ZoneState::Empty;
                Ok(())
            }
        }
    }

    /// Transitions a zone to Offline.
    pub fn offline_zone(&mut self, zone_index: u32) -> Result<(), &'static str> {
        let zone = self.zones.get_mut(zone_index as usize).ok_or("Zone index out of bounds")?;
        zone.state = ZoneState::Offline;
        Ok(())
    }

    /// Advances the write pointer during a sequential write operation.
    pub fn write_sequential(&mut self, zone_index: u32, lba_count: u64) -> Result<u64, &'static str> {
        let zone = self.zones.get_mut(zone_index as usize).ok_or("Zone index out of bounds")?;
        if zone.state == ZoneState::Empty {
            self.open_zone(zone_index, false)?;
        }
        let zone = self.zones.get_mut(zone_index as usize).unwrap();
        if !zone.is_open() {
            return Err("Zone must be in an open state to write");
        }
        if lba_count > zone.remaining_capacity() {
            return Err("Write exceeds zone remaining capacity");
        }

        let assigned_lba = zone.write_pointer;
        zone.write_pointer += lba_count;
        if zone.remaining_capacity() == 0 {
            zone.state = ZoneState::Full;
        }
        Ok(assigned_lba)
    }
}

// -----------------------------------------------------------------------------
// NVMe ZNS Command Submission Builders
// -----------------------------------------------------------------------------

/// Constructs an NVMe Zone Management Send submission entry.
pub fn build_zone_management_send(
    cid: u16,
    nsid: u32,
    slba: u64,
    action: ZoneSendAction,
    select_all: bool,
) -> NvmeSubmissionEntry {
    let mut entry = NvmeSubmissionEntry::new(NVME_ZNS_MANAGEMENT_SEND, cid, nsid);
    entry.cdw10 = (slba & 0xFFFF_FFFF) as u32;
    entry.cdw11 = (slba >> 32) as u32;
    let mut cdw13 = action as u32;
    if select_all {
        cdw13 |= 1 << 8;
    }
    entry.cdw13 = cdw13;
    entry
}

/// Constructs an NVMe Zone Management Receive submission entry (Report Zones).
pub fn build_zone_management_receive(
    cid: u16,
    nsid: u32,
    slba: u64,
    prp1: u64,
    prp2: u64,
    report_opts: u8,
) -> NvmeSubmissionEntry {
    let mut entry = NvmeSubmissionEntry::new(NVME_ZNS_MANAGEMENT_RECEIVE, cid, nsid);
    entry.prp1 = prp1;
    entry.prp2 = prp2;
    entry.cdw10 = (slba & 0xFFFF_FFFF) as u32;
    entry.cdw11 = (slba >> 32) as u32;
    entry.cdw13 = report_opts as u32;
    entry
}
