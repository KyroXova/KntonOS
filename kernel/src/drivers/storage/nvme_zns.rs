//! NVMe Zoned Namespaces (ZNS) Device Subsystem & Sequential Zone State Machine.

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;
use crate::drivers::storage::nvme::NvmeSubmissionEntry;

// ZNS Command Opcodes & Action Definitions.

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

// Zone Descriptor.

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

// ZNS Namespace & Zone State Machine.

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

// NVMe ZNS Command Submission Builders.

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


// Zone Append Command Queue & Multi-Writer Dispatcher.

/// Status of an asynchronous Zone Append operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneAppendStatus {
    Pending,
    Submitted,
    Completed,
    Failed,
}

/// Asynchronous Zone Append request allowing lock-free concurrent zone writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneAppendRequest {
    pub request_id: u64,
    pub zone_index: u32,
    pub lba_count: u32,
    pub prp1: u64,
    pub prp2: u64,
    pub status: ZoneAppendStatus,
    pub assigned_lba: Option<u64>,
}

impl ZoneAppendRequest {
    pub const fn new(
        request_id: u64,
        zone_index: u32,
        lba_count: u32,
        prp1: u64,
        prp2: u64,
    ) -> Self {
        Self {
            request_id,
            zone_index,
            lba_count,
            prp1,
            prp2,
            status: ZoneAppendStatus::Pending,
            assigned_lba: None,
        }
    }
}

/// Constructs an NVMe Zone Append submission entry (Opcode 0x7D).
pub fn build_zone_append(
    cid: u16,
    nsid: u32,
    zslba: u64,
    nlb: u16,
    prp1: u64,
    prp2: u64,
) -> NvmeSubmissionEntry {
    let mut entry = NvmeSubmissionEntry::new(NVME_ZNS_ZONE_APPEND, cid, nsid);
    entry.prp1 = prp1;
    entry.prp2 = prp2;
    entry.cdw10 = (zslba & 0xFFFF_FFFF) as u32;
    entry.cdw11 = (zslba >> 32) as u32;
    entry.cdw12 = (nlb.saturating_sub(1)) as u32; // 0-based count
    entry
}

/// Queue manager for submitting concurrent Zone Append commands without host-side write pointer serialization.
#[derive(Debug, Default)]
pub struct ZoneAppendQueue {
    pub requests: Vec<ZoneAppendRequest>,
    next_id: u64,
    pub total_appended_lbas: u64,
}

impl ZoneAppendQueue {
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
            next_id: 1,
            total_appended_lbas: 0,
        }
    }

    /// Enqueues a new append request to a target zone.
    pub fn enqueue(
        &mut self,
        zone_index: u32,
        lba_count: u32,
        prp1: u64,
        prp2: u64,
    ) -> u64 {
        let req_id = self.next_id;
        self.next_id += 1;
        self.requests.push(ZoneAppendRequest::new(
            req_id,
            zone_index,
            lba_count,
            prp1,
            prp2,
        ));
        req_id
    }

    /// Submits a batch of pending Zone Append requests to the ZNS namespace,
    /// atomically advancing write pointers and generating NVMe hardware submission entries.
    pub fn submit_batch(
        &mut self,
        zns: &mut ZnsNamespace,
    ) -> Result<Vec<NvmeSubmissionEntry>, &'static str> {
        let mut submissions = Vec::new();

        for req in self.requests.iter_mut() {
            if req.status != ZoneAppendStatus::Pending {
                continue;
            }

            let zone = zns.zones.get_mut(req.zone_index as usize).ok_or("Zone index out of bounds")?;
            if zone.state == ZoneState::Empty {
                zns.open_zone(req.zone_index, false)?;
            }
            let zone = zns.zones.get_mut(req.zone_index as usize).unwrap();
            if !zone.is_open() {
                req.status = ZoneAppendStatus::Failed;
                return Err("Zone is not open for append");
            }
            if (req.lba_count as u64) > zone.remaining_capacity() {
                req.status = ZoneAppendStatus::Failed;
                return Err("Append exceeds zone remaining capacity");
            }

            // The controller atomically assigns the current write pointer
            let assigned = zone.write_pointer;
            zone.write_pointer += req.lba_count as u64;
            if zone.remaining_capacity() == 0 {
                zone.state = ZoneState::Full;
            }

            req.assigned_lba = Some(assigned);
            req.status = ZoneAppendStatus::Completed;
            self.total_appended_lbas += req.lba_count as u64;

            let cid = (req.request_id & 0xFFFF) as u16;
            submissions.push(build_zone_append(
                cid,
                zns.nsid,
                zone.start_lba,
                req.lba_count as u16,
                req.prp1,
                req.prp2,
            ));
        }

        Ok(submissions)
    }

    pub fn get_request(&self, request_id: u64) -> Option<&ZoneAppendRequest> {
        self.requests.iter().find(|r| r.request_id == request_id)
    }

    pub fn pending_count(&self) -> usize {
        self.requests.iter().filter(|r| r.status == ZoneAppendStatus::Pending).count()
    }
}

// Active Zone Capacity Tracker & Governor.

/// Active zone capacity tracker ensuring active/open zone limits and capacity monitoring.
#[derive(Debug, Default, Clone, Copy)]
pub struct ActiveZoneCapacityTracker;

impl ActiveZoneCapacityTracker {
    pub const fn new() -> Self {
        Self
    }

    /// Computes the total active capacity across all active zones in the namespace.
    pub fn total_active_capacity(&self, zns: &ZnsNamespace) -> u64 {
        zns.zones
            .iter()
            .filter(|z| z.is_active())
            .map(|z| z.capacity_lbas)
            .sum()
    }

    /// Computes total writeable remaining LBAs across all active zones.
    pub fn total_remaining_capacity(&self, zns: &ZnsNamespace) -> u64 {
        zns.zones
            .iter()
            .filter(|z| z.is_active())
            .map(|z| z.remaining_capacity())
            .sum()
    }

    /// Collects list of active zone indices.
    pub fn active_zone_indices(&self, zns: &ZnsNamespace) -> Vec<u32> {
        zns.zones
            .iter()
            .filter(|z| z.is_active())
            .map(|z| z.zone_index)
            .collect()
    }

    /// Checks if a write or append of `lba_count` is permitted in the designated zone.
    pub fn can_write(&self, zns: &ZnsNamespace, zone_index: u32, lba_count: u64) -> bool {
        if let Some(zone) = zns.zones.get(zone_index as usize) {
            match zone.state {
                ZoneState::Full | ZoneState::ReadOnly | ZoneState::Offline => false,
                ZoneState::Empty => {
                    zns.active_zone_count() < zns.max_active_zones
                        && zns.open_zone_count() < zns.max_open_zones
                        && lba_count <= zone.capacity_lbas
                }
                ZoneState::Closed => {
                    zns.open_zone_count() < zns.max_open_zones
                        && lba_count <= zone.remaining_capacity()
                }
                ZoneState::ImplicitlyOpened | ZoneState::ExplicitlyOpened => {
                    lba_count <= zone.remaining_capacity()
                }
            }
        } else {
            false
        }
    }

    /// Selects the best existing open zone with sufficient capacity to satisfy an append,
    /// or opens a new empty zone if allowed.
    pub fn find_best_zone_for_append(&self, zns: &ZnsNamespace, lba_count: u64) -> Option<u32> {
        // First try already opened zones with enough capacity
        for zone in &zns.zones {
            if zone.is_open() && zone.remaining_capacity() >= lba_count {
                return Some(zone.zone_index);
            }
        }
        // Next try empty zones if active and open limits permit
        if zns.active_zone_count() < zns.max_active_zones && zns.open_zone_count() < zns.max_open_zones {
            for zone in &zns.zones {
                if zone.is_empty() && zone.capacity_lbas >= lba_count {
                    return Some(zone.zone_index);
                }
            }
        }
        None
    }
}


// Subsystem Verification Self-Test.

/// Verification self-test suite for Kernel-Direct NVMe ZNS and Hardware Copy Offload Engine.
pub fn test_nvme_zns_subsystem() -> bool {
    use crate::drivers::storage::copy_offload::{
        NvmeCopyOffloadEngine, NvmeCopyRangeDescriptor, NVME_NVM_COPY,
    };

    // 1. Verify ZNS geometry and standard Zone State Machine
    let mut zns = ZnsNamespace::new(1, 524_288, 65_536, 64_000, 4, 2);
    if zns.zone_count() != 8 {
        return false;
    }
    // Initial zone state must be Empty
    if !zns.zones[0].is_empty() || zns.zones[0].write_pointer != 0 || zns.zones[0].remaining_capacity() != 64_000 {
        return false;
    }

    // Open zone 0 explicitly
    if zns.open_zone(0, true).is_err() || zns.zones[0].state != ZoneState::ExplicitlyOpened {
        return false;
    }
    // Open zone 1 implicitly
    if zns.open_zone(1, false).is_err() || zns.zones[1].state != ZoneState::ImplicitlyOpened {
        return false;
    }
    if zns.open_zone_count() != 2 || zns.active_zone_count() != 2 {
        return false;
    }

    // Enforce max open zones limit (limit = 2)
    if zns.open_zone(2, true).is_ok() {
        return false;
    }

    // Close zone 1
    if zns.close_zone(1).is_err() || zns.zones[1].state != ZoneState::Closed {
        return false;
    }
    if zns.open_zone_count() != 1 || zns.active_zone_count() != 2 {
        return false;
    }

    // Now opening zone 2 succeeds
    if zns.open_zone(2, true).is_err() || zns.zones[2].state != ZoneState::ExplicitlyOpened {
        return false;
    }

    // Sequential write to zone 0: write 10,000 LBAs
    let assigned = match zns.write_sequential(0, 10_000) {
        Ok(lba) => lba,
        Err(_) => return false,
    };
    if assigned != 0 || zns.zones[0].write_pointer != 10_000 || zns.zones[0].remaining_capacity() != 54_000 {
        return false;
    }

    // Finish zone 0: write pointer advances to capacity (64,000) and state becomes Full
    if zns.finish_zone(0).is_err() || zns.zones[0].state != ZoneState::Full || zns.zones[0].write_pointer != 64_000 {
        return false;
    }
    if zns.zones[0].remaining_capacity() != 0 {
        return false;
    }

    // Reset zone 0: write pointer resets to start LBA and state returns to Empty
    if zns.reset_zone(0).is_err() || zns.zones[0].state != ZoneState::Empty || zns.zones[0].write_pointer != 0 {
        return false;
    }

    // 2. Verify Zone Append Queue and Concurrent Multi-Writer Dispatch
    let mut append_queue = ZoneAppendQueue::new();
    let id1 = append_queue.enqueue(2, 500, 0x1000_0000, 0);
    let id2 = append_queue.enqueue(2, 1000, 0x1001_0000, 0);
    let id3 = append_queue.enqueue(2, 1500, 0x1002_0000, 0);

    let submissions = match append_queue.submit_batch(&mut zns) {
        Ok(s) => s,
        Err(_) => return false,
    };

    if submissions.len() != 3 || append_queue.total_appended_lbas != 3000 {
        return false;
    }

    // Verify assigned LBAs are strictly monotonic without host lock serialization
    let r1 = match append_queue.get_request(id1) {
        Some(r) => r,
        None => return false,
    };
    let r2 = match append_queue.get_request(id2) {
        Some(r) => r,
        None => return false,
    };
    let r3 = match append_queue.get_request(id3) {
        Some(r) => r,
        None => return false,
    };

    let start_lba_zone2 = zns.zones[2].start_lba;
    if r1.assigned_lba != Some(start_lba_zone2) {
        return false;
    }
    if r2.assigned_lba != Some(start_lba_zone2 + 500) {
        return false;
    }
    if r3.assigned_lba != Some(start_lba_zone2 + 1500) {
        return false;
    }
    if zns.zones[2].write_pointer != start_lba_zone2 + 3000 {
        return false;
    }

    // Verify submission entry hardware encoding
    if submissions[0].opcode() != NVME_ZNS_ZONE_APPEND {
        return false;
    }

    // 3. Verify Active Zone Capacity Tracker & Governor
    let tracker = ActiveZoneCapacityTracker::new();
    let active_cap = tracker.total_active_capacity(&zns);
    if active_cap == 0 || !tracker.can_write(&zns, 2, 1000) {
        return false;
    }
    let best_zone = tracker.find_best_zone_for_append(&zns, 2000);
    if best_zone.is_none() {
        return false;
    }

    // 4. Verify NVMe Simple Copy Command & Hardware Copy Offload Engine
    let mut copy_engine = NvmeCopyOffloadEngine::new(512);

    let ranges = [
        NvmeCopyRangeDescriptor::new(100, 50),
        NvmeCopyRangeDescriptor::new(500, 100),
    ];
    if ranges[0].lba_count() != 50 || ranges[1].lba_count() != 100 {
        return false;
    }

    let copy_cmd = match copy_engine.prepare_copy_job(42, 1, 10_000, &ranges, 0x2000_0000) {
        Ok(cmd) => cmd,
        Err(_) => return false,
    };
    if copy_cmd.opcode() != NVME_NVM_COPY || copy_cmd.cdw10 != 10_000 || copy_cmd.cdw12 != 1 {
        return false;
    }

    // Execute copy offload directly into destination ZNS zone 3
    let copy_ranges = [
        NvmeCopyRangeDescriptor::new(start_lba_zone2, 500),
        NvmeCopyRangeDescriptor::new(start_lba_zone2 + 500, 1000),
    ];
    let dest_slba = match copy_engine.execute_zns_zone_copy_offload(&mut zns, 3, &copy_ranges) {
        Ok(slba) => slba,
        Err(_) => return false,
    };

    let start_lba_zone3 = zns.zones[3].start_lba;
    if dest_slba != start_lba_zone3 || zns.zones[3].write_pointer != start_lba_zone3 + 1500 {
        return false;
    }
    // Telemetry: 1500 LBAs * 512 bytes * 2 (read + write PCIe traffic eliminated)
    let expected_saved = 1500 * 512 * 2;
    if copy_engine.bytes_saved() != expected_saved {
        return false;
    }

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-NVME-ZNS-PASS] NVMe Zoned Namespaces (ZNS) and hardware copy offload engine verified\r\n");

    true
}
