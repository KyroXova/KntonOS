//! Page-fault and EPT-backed MMIO memory range watchpoint interceptor.


/// MMIO access type intercepted by the watchpoint engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmioAccessType {
    Read,
    Write,
    Execute,
}

/// Record of an intercepted MMIO access event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioInterceptEvent {
    pub watchpoint_id: u32,
    pub instruction_pointer: usize,
    pub accessed_address: usize,
    pub access_type: MmioAccessType,
    pub access_size: usize,
    pub write_value: u64,
}

impl MmioInterceptEvent {
    pub const fn new(
        watchpoint_id: u32,
        instruction_pointer: usize,
        accessed_address: usize,
        access_type: MmioAccessType,
        access_size: usize,
        write_value: u64,
    ) -> Self {
        Self {
            watchpoint_id,
            instruction_pointer,
            accessed_address,
            access_type,
            access_size,
            write_value,
        }
    }
}

/// Definition of a monitored MMIO memory range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioWatchRange {
    pub id: u32,
    pub base_addr: usize,
    pub size: usize,
    pub trap_reads: bool,
    pub trap_writes: bool,
    pub active: bool,
}

impl MmioWatchRange {
    pub const fn new(id: u32, base_addr: usize, size: usize, trap_reads: bool, trap_writes: bool) -> Self {
        Self {
            id,
            base_addr,
            size,
            trap_reads,
            trap_writes,
            active: true,
        }
    }

    #[inline(always)]
    pub fn contains(&self, addr: usize) -> bool {
        self.active && addr >= self.base_addr && addr < self.base_addr + self.size
    }
}

pub const MAX_MMIO_WATCHPOINTS: usize = 16;
pub const MAX_INTERCEPT_LOGS: usize = 32;

/// Page-fault / EPT backed MMIO range interception engine.
pub struct MmioRangeInterceptor {
    ranges: [Option<MmioWatchRange>; MAX_MMIO_WATCHPOINTS],
    range_count: usize,
    intercept_log: [Option<MmioInterceptEvent>; MAX_INTERCEPT_LOGS],
    log_count: usize,
    total_traps: u64,
}

impl MmioRangeInterceptor {
    pub const fn new() -> Self {
        Self {
            ranges: [None; MAX_MMIO_WATCHPOINTS],
            range_count: 0,
            intercept_log: [None; MAX_INTERCEPT_LOGS],
            log_count: 0,
            total_traps: 0,
        }
    }

    /// Registers a new MMIO address range for trapping.
    pub fn register_range(
        &mut self,
        id: u32,
        base_addr: usize,
        size: usize,
        trap_reads: bool,
        trap_writes: bool,
    ) -> Result<(), &'static str> {
        if self.range_count >= MAX_MMIO_WATCHPOINTS {
            return Err("MMIO watchpoint table full");
        }
        for slot in self.ranges.iter_mut() {
            if slot.is_none() {
                *slot = Some(MmioWatchRange::new(id, base_addr, size, trap_reads, trap_writes));
                self.range_count += 1;
                return Ok(());
            }
        }
        Err("No free watchpoint slot")
    }

    /// Removes an MMIO watchpoint range.
    pub fn unregister_range(&mut self, id: u32) -> bool {
        for slot in self.ranges.iter_mut() {
            if let Some(r) = slot {
                if r.id == id {
                    *slot = None;
                    self.range_count = self.range_count.saturating_sub(1);
                    return true;
                }
            }
        }
        false
    }

    /// Evaluates a #PF or EPT violation against active MMIO watchpoints, returning an intercept event.
    pub fn handle_fault(
        &mut self,
        fault_addr: usize,
        rip: usize,
        is_write: bool,
        access_size: usize,
        value: u64,
    ) -> Option<MmioInterceptEvent> {
        let access_type = if is_write {
            MmioAccessType::Write
        } else {
            MmioAccessType::Read
        };

        for r in self.ranges.iter().flatten() {
            if r.contains(fault_addr) {
                let should_trap = match access_type {
                    MmioAccessType::Write => r.trap_writes,
                    MmioAccessType::Read => r.trap_reads,
                    MmioAccessType::Execute => false,
                };

                if should_trap {
                    let event = MmioInterceptEvent::new(
                        r.id,
                        rip,
                        fault_addr,
                        access_type,
                        access_size,
                        value,
                    );
                    self.record_event(event);
                    return Some(event);
                }
            }
        }

        None
    }

    fn record_event(&mut self, event: MmioInterceptEvent) {
        self.total_traps += 1;
        if self.log_count < MAX_INTERCEPT_LOGS {
            self.intercept_log[self.log_count] = Some(event);
            self.log_count += 1;
        } else {
            // Shift log array left to record newest entry
            for i in 1..MAX_INTERCEPT_LOGS {
                self.intercept_log[i - 1] = self.intercept_log[i];
            }
            self.intercept_log[MAX_INTERCEPT_LOGS - 1] = Some(event);
        }
    }

    #[inline(always)]
    pub const fn total_traps(&self) -> u64 {
        self.total_traps
    }

    #[inline(always)]
    pub const fn logged_events_count(&self) -> usize {
        self.log_count
    }

    pub fn last_event(&self) -> Option<MmioInterceptEvent> {
        if self.log_count > 0 {
            self.intercept_log[self.log_count - 1]
        } else {
            None
        }
    }
}

/// Unit test verifying Page-Fault / EPT backed MMIO range registration, fault filtering, and event logging.
pub fn test_mmio_range_interceptor() -> bool {
    let mut interceptor = MmioRangeInterceptor::new();

    // 1. Register 4KB NVMe MMIO register window at 0xFEBF_0000
    let nvme_base = 0xFEBF_0000;
    let nvme_size = 0x1000;
    if interceptor
        .register_range(1, nvme_base, nvme_size, true, true)
        .is_err()
    {
        return false;
    }

    // 2. Register VirtIO GPU BAR0 window at 0xFEB0_0000 (size 16KB, trap writes only)
    let gpu_base = 0xFEB0_0000;
    let gpu_size = 0x4000;
    if interceptor
        .register_range(2, gpu_base, gpu_size, false, true)
        .is_err()
    {
        return false;
    }

    if interceptor.range_count != 2 {
        return false;
    }

    // 3. Test NVMe Doorbell write fault (offset 0x1000 within base)
    let db_addr = nvme_base + 0x1000 - 4; // last 4 bytes of 4KB
    let rip_1 = 0xFFFFFFFF_8010_2040;
    let evt1 = match interceptor.handle_fault(db_addr, rip_1, true, 4, 0x0000_0001) {
        Some(e) => e,
        None => return false,
    };
    if evt1.watchpoint_id != 1 || evt1.access_type != MmioAccessType::Write || evt1.write_value != 1 {
        return false;
    }

    // 4. Test GPU read fault (should be ignored because trap_reads is false for GPU)
    let gpu_read_addr = gpu_base + 0x10;
    let rip_2 = 0xFFFFFFFF_8010_2080;
    if interceptor.handle_fault(gpu_read_addr, rip_2, false, 4, 0).is_some() {
        return false;
    }

    // 5. Test GPU write fault (should be trapped)
    let gpu_write_addr = gpu_base + 0x20;
    let rip_3 = 0xFFFFFFFF_8010_20C0;
    let evt2 = match interceptor.handle_fault(gpu_write_addr, rip_3, true, 4, 0xDEAD_BEEF) {
        Some(e) => e,
        None => return false,
    };
    if evt2.watchpoint_id != 2 || evt2.access_type != MmioAccessType::Write || evt2.write_value != 0xDEAD_BEEF {
        return false;
    }

    // 6. Test out-of-range fault (unmonitored address)
    if interceptor.handle_fault(0xFED0_0000, 0xFFFFFFFF_8010_2100, true, 4, 0).is_some() {
        return false;
    }

    // Verify statistics and last event
    if interceptor.total_traps() != 2 || interceptor.logged_events_count() != 2 {
        return false;
    }
    let last = match interceptor.last_event() {
        Some(l) => l,
        None => return false,
    };
    if last.accessed_address != gpu_write_addr || last.write_value != 0xDEAD_BEEF {
        return false;
    }

    // Unregister NVMe range
    if !interceptor.unregister_range(1) || interceptor.range_count != 1 {
        return false;
    }

    true
}
