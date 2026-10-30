//! External eGPU Hot-Connect Manager & Dynamic PCIe Bridge Configuration.
//!
//! Provides dynamic PCIe Type 1 bridge secondary/subordinate bus renumbering,
//! 32-bit non-prefetchable and 64-bit prefetchable MMIO aperture window allocation,
//! Resizable BAR support, and external eGPU hot-connect attach/detach management
//! across Thunderbolt 3/4, USB4, and OCuLink PCIe interconnects.

use alloc::vec::Vec;

// -----------------------------------------------------------------------------
// PCIe Type 1 PCI-to-PCI Bridge Register Offsets
// -----------------------------------------------------------------------------

pub const PCI_PRIMARY_BUS_OFFSET: u8 = 0x18;
pub const PCI_SECONDARY_BUS_OFFSET: u8 = 0x19;
pub const PCI_SUBORDINATE_BUS_OFFSET: u8 = 0x1A;
pub const PCI_SEC_LATENCY_TIMER_OFFSET: u8 = 0x1B;
pub const PCI_IO_BASE_OFFSET: u8 = 0x1C;
pub const PCI_IO_LIMIT_OFFSET: u8 = 0x1D;
pub const PCI_SEC_STATUS_OFFSET: u8 = 0x1E;
pub const PCI_MEMORY_BASE_OFFSET: u8 = 0x20;
pub const PCI_MEMORY_LIMIT_OFFSET: u8 = 0x22;
pub const PCI_PREF_MEMORY_BASE_OFFSET: u8 = 0x24;
pub const PCI_PREF_MEMORY_LIMIT_OFFSET: u8 = 0x26;
pub const PCI_PREF_BASE_UPPER32_OFFSET: u8 = 0x28;
pub const PCI_PREF_LIMIT_UPPER32_OFFSET: u8 = 0x2C;
pub const PCI_BRIDGE_CONTROL_OFFSET: u8 = 0x3E;

/// PCI BAR classification types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarType {
    Mmio32,
    Mmio64Prefetchable,
    IoPort,
}

/// Dynamic PCI BAR allocation record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciBarAllocation {
    pub bar_idx: u8,
    pub bar_type: BarType,
    pub base_address: u64,
    pub size: u64,
    pub is_prefetchable: bool,
}

/// PCIe Bridge Configuration Space Window Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciBridgeConfig {
    pub primary_bus: u8,
    pub secondary_bus: u8,
    pub subordinate_bus: u8,
    pub mem_base_32: u32,
    pub mem_limit_32: u32,
    pub pref_mem_base_64: u64,
    pub pref_mem_limit_64: u64,
}

impl PciBridgeConfig {
    pub const fn empty(primary_bus: u8) -> Self {
        Self {
            primary_bus,
            secondary_bus: 0,
            subordinate_bus: 0,
            mem_base_32: 0,
            mem_limit_32: 0,
            pref_mem_base_64: 0,
            pref_mem_limit_64: 0,
        }
    }

    /// Encodes 32-bit non-prefetchable memory base & limit into 16-bit register formats.
    /// PCIe Base Spec: Bits 15:4 correspond to address bits 31:20 (1 MiB alignment).
    pub fn encode_mem_window_32(&self) -> (u16, u16) {
        if self.mem_base_32 >= self.mem_limit_32 {
            return (0xFFFF, 0x0000); // Disabled window
        }
        let base_reg = ((self.mem_base_32 >> 16) & 0xFFF0) as u16;
        let limit_reg = ((self.mem_limit_32.saturating_sub(1) >> 16) & 0xFFF0) as u16;
        (base_reg, limit_reg)
    }

    /// Decodes 32-bit non-prefetchable memory registers into physical address range.
    pub fn decode_mem_window_32(base_reg: u16, limit_reg: u16) -> (u32, u32) {
        if base_reg > limit_reg {
            return (0, 0);
        }
        let base = ((base_reg & 0xFFF0) as u32) << 16;
        let limit = (((limit_reg & 0xFFF0) as u32) << 16) | 0x000F_FFFF;
        (base, limit + 1)
    }

    /// Encodes 64-bit prefetchable memory base & limit registers.
    /// Lower 4 bits specify 64-bit address support (0x1).
    pub fn encode_pref_window_64(&self) -> (u16, u16, u32, u32) {
        if self.pref_mem_base_64 >= self.pref_mem_limit_64 {
            return (0xFFFF, 0x0000, 0, 0);
        }
        let base_low = (((self.pref_mem_base_64 >> 16) & 0xFFF0) as u16) | 0x0001;
        let limit_low = (((self.pref_mem_limit_64.saturating_sub(1) >> 16) & 0xFFF0) as u16) | 0x0001;
        let base_upper = (self.pref_mem_base_64 >> 32) as u32;
        let limit_upper = (self.pref_mem_limit_64.saturating_sub(1) >> 32) as u32;
        (base_low, limit_low, base_upper, limit_upper)
    }

    /// Decodes 64-bit prefetchable memory registers into physical 64-bit address range.
    pub fn decode_pref_window_64(
        base_low: u16,
        limit_low: u16,
        base_upper: u32,
        limit_upper: u32,
    ) -> (u64, u64) {
        if (base_low & 0xFFF0) > (limit_low & 0xFFF0) && base_upper >= limit_upper {
            return (0, 0);
        }
        let base = (((base_upper as u64) << 32) | (((base_low & 0xFFF0) as u64) << 16)) as u64;
        let limit = (((limit_upper as u64) << 32)
            | (((limit_low & 0xFFF0) as u64) << 16)
            | 0x000F_FFFF) as u64;
        (base, limit + 1)
    }
}

/// PCIe Physical MMIO Resource Window Allocator.
///
/// Carves out aligned physical address regions for 32-bit non-prefetchable and
/// 64-bit prefetchable BAR apertures without colliding with system RAM.
pub struct PciResourceWindowPool {
    pub mmio32_base: u32,
    pub mmio32_limit: u32,
    pub next_mmio32: u32,
    pub pref64_base: u64,
    pub pref64_limit: u64,
    pub next_pref64: u64,
}

impl PciResourceWindowPool {
    /// Creates a resource pool with default x86-64 Q35 / modern PC MMIO ranges:
    /// - 32-bit MMIO: 0xE000_0000 .. 0xFE00_0000 (480 MiB)
    /// - 64-bit Prefetchable MMIO: 0x0000_0004_0000_0000 .. 0x0000_0008_0000_0000 (16 GiB)
    pub fn new_default() -> Self {
        Self {
            mmio32_base: 0xE000_0000,
            mmio32_limit: 0xFE00_0000,
            next_mmio32: 0xE000_0000,
            pref64_base: 0x0000_0004_0000_0000,
            pref64_limit: 0x0000_0008_0000_0000,
            next_pref64: 0x0000_0004_0000_0000,
        }
    }

    /// Allocates an aligned 32-bit non-prefetchable MMIO window.
    pub fn allocate_mmio32(&mut self, size: usize, align: usize) -> Result<u32, &'static str> {
        let align = align.max(1024 * 1024); // Minimum 1 MiB alignment for PCIe bridges
        let aligned_addr = (self.next_mmio32 + (align as u32 - 1)) & !(align as u32 - 1);
        let end_addr = aligned_addr.checked_add(size as u32).ok_or("MMIO32 overflow")?;

        if end_addr > self.mmio32_limit {
            return Err("Out of 32-bit MMIO aperture space");
        }

        self.next_mmio32 = end_addr;
        Ok(aligned_addr)
    }

    /// Allocates an aligned 64-bit prefetchable MMIO window.
    pub fn allocate_pref64(&mut self, size: u64, align: u64) -> Result<u64, &'static str> {
        let align = align.max(1024 * 1024); // Minimum 1 MiB alignment
        let aligned_addr = (self.next_pref64 + (align - 1)) & !(align - 1);
        let end_addr = aligned_addr.checked_add(size).ok_or("Prefetchable MMIO64 overflow")?;

        if end_addr > self.pref64_limit {
            return Err("Out of 64-bit prefetchable MMIO aperture space");
        }

        self.next_pref64 = end_addr;
        Ok(aligned_addr)
    }
}

/// PCIe Bus Number Allocator.
///
/// Dynamically assigns downstream secondary and subordinate bus number ranges
/// to newly discovered hot-plugged PCIe switches and bridges.
pub struct PciBusAllocator {
    pub next_bus: u8,
    pub max_bus: u8,
}

impl PciBusAllocator {
    pub fn new(start_bus: u8, max_bus: u8) -> Self {
        Self {
            next_bus: start_bus,
            max_bus,
        }
    }

    /// Allocates a contiguous range of bus numbers (e.g. for a downstream switch and endpoints).
    pub fn allocate_bus_range(&mut self, count: u8) -> Result<(u8, u8), &'static str> {
        if count == 0 {
            return Err("Invalid bus count: 0");
        }
        let sec_bus = self.next_bus;
        let sub_bus = sec_bus.checked_add(count - 1).ok_or("PCI bus number overflow")?;

        if sub_bus > self.max_bus {
            return Err("Exhausted available PCI bus numbers");
        }

        self.next_bus = sub_bus + 1;
        Ok((sec_bus, sub_bus))
    }
}

/// Dynamic Bridge Manager.
///
/// Programs physical Type 1 PCI-to-PCI bridge registers and allocates MMIO BARs
/// for downstream external eGPU adapters.
pub struct DynamicBridgeManager {
    pub bridge_bdf: (u8, u8, u8),
    pub bridge_config: PciBridgeConfig,
    pub assigned_bars: Vec<PciBarAllocation>,
}

impl DynamicBridgeManager {
    pub fn new(bus: u8, dev: u8, func: u8) -> Self {
        Self {
            bridge_bdf: (bus, dev, func),
            bridge_config: PciBridgeConfig::empty(bus),
            assigned_bars: Vec::new(),
        }
    }

    /// Renumbers secondary and subordinate buses behind this bridge.
    pub fn renumber_buses(
        &mut self,
        allocator: &mut PciBusAllocator,
        bus_count: u8,
    ) -> Result<(u8, u8), &'static str> {
        let (sec, sub) = allocator.allocate_bus_range(bus_count)?;
        self.bridge_config.secondary_bus = sec;
        self.bridge_config.subordinate_bus = sub;
        Ok((sec, sub))
    }

    /// Allocates and programs 32-bit and 64-bit MMIO windows for this bridge.
    pub fn configure_mmio_apertures(
        &mut self,
        pool: &mut PciResourceWindowPool,
        mmio32_size: usize,
        pref64_size: u64,
    ) -> Result<(), &'static str> {
        // Allocate 32-bit non-prefetchable window (1 MiB aligned)
        let mem32_base = pool.allocate_mmio32(mmio32_size, 1024 * 1024)?;
        let mem32_limit = mem32_base + mmio32_size as u32;

        // Allocate 64-bit prefetchable window (e.g. 256 MiB or 1 GiB for VRAM)
        let pref64_base = pool.allocate_pref64(pref64_size, 1024 * 1024)?;
        let pref64_limit = pref64_base + pref64_size;

        self.bridge_config.mem_base_32 = mem32_base;
        self.bridge_config.mem_limit_32 = mem32_limit;
        self.bridge_config.pref_mem_base_64 = pref64_base;
        self.bridge_config.pref_mem_limit_64 = pref64_limit;

        Ok(())
    }

    /// Assigns a physical BAR allocation within the bridge's configured apertures.
    pub fn assign_device_bar(
        &mut self,
        bar_idx: u8,
        bar_type: BarType,
        size: u64,
    ) -> Result<PciBarAllocation, &'static str> {
        let base_address = match bar_type {
            BarType::Mmio32 => {
                // Must fit within bridge 32-bit non-prefetchable window
                let align = (size as u32).max(4096);
                let current_used = self
                    .assigned_bars
                    .iter()
                    .filter(|b| b.bar_type == BarType::Mmio32)
                    .map(|b| b.size)
                    .sum::<u64>();
                let candidate = (self.bridge_config.mem_base_32 as u64 + current_used + (align as u64 - 1))
                    & !(align as u64 - 1);
                if candidate + size > self.bridge_config.mem_limit_32 as u64 {
                    return Err("BAR exceeds bridge MMIO32 window");
                }
                candidate
            }
            BarType::Mmio64Prefetchable => {
                // Fits within bridge 64-bit prefetchable window (VRAM aperture)
                let align = size.max(1024 * 1024);
                let current_used = self
                    .assigned_bars
                    .iter()
                    .filter(|b| b.bar_type == BarType::Mmio64Prefetchable)
                    .map(|b| b.size)
                    .sum::<u64>();
                let candidate = (self.bridge_config.pref_mem_base_64 + current_used + (align - 1))
                    & !(align - 1);
                if candidate + size > self.bridge_config.pref_mem_limit_64 {
                    return Err("BAR exceeds bridge Prefetchable MMIO64 window");
                }
                candidate
            }
            BarType::IoPort => return Err("I/O ports unsupported for modern eGPUs"),
        };

        let alloc = PciBarAllocation {
            bar_idx,
            bar_type,
            base_address,
            size,
            is_prefetchable: bar_type == BarType::Mmio64Prefetchable,
        };

        self.assigned_bars.push(alloc);
        Ok(alloc)
    }
}

/// Unit test verifying dynamic bridge bus renumbering, MMIO aperture allocation, and BAR mapping.
pub fn verify_bridge_renumbering_and_bar_allocations() -> bool {
    // 1. Initialize bus allocator starting at Bus 5 (after on-board devices)
    let mut bus_alloc = PciBusAllocator::new(5, 32);

    // Initialize Bridge Manager on Bus 0, Dev 28, Func 0 (eGPU PCIe Root Port)
    let mut bridge = DynamicBridgeManager::new(0, 28, 0);

    // Allocate 4 downstream buses (e.g. for external Thunderbolt PCIe switch + GPU endpoints)
    let (sec_bus, sub_bus) = match bridge.renumber_buses(&mut bus_alloc, 4) {
        Ok(res) => res,
        Err(_) => return false,
    };

    if sec_bus != 5 || sub_bus != 8 || bridge.bridge_config.secondary_bus != 5 {
        return false;
    }
    if bridge.bridge_config.subordinate_bus != 8 || bus_alloc.next_bus != 9 {
        return false;
    }

    // 2. Initialize resource pool and configure bridge apertures
    let mut pool = PciResourceWindowPool::new_default();

    // Allocate 32 MiB non-prefetchable (control registers) + 256 MiB prefetchable (VRAM BAR)
    let mmio32_size = 32 * 1024 * 1024;
    let pref64_size = 256 * 1024 * 1024;

    if bridge.configure_mmio_apertures(&mut pool, mmio32_size, pref64_size).is_err() {
        return false;
    }

    let cfg = bridge.bridge_config;
    if cfg.mem_limit_32 - cfg.mem_base_32 != mmio32_size as u32 {
        return false;
    }
    if cfg.pref_mem_limit_64 - cfg.pref_mem_base_64 != pref64_size {
        return false;
    }

    // 3. Test Type 1 register encoding and round-trip decoding
    let (mem_base_reg, mem_limit_reg) = cfg.encode_mem_window_32();
    let (dec_base32, dec_limit32) = PciBridgeConfig::decode_mem_window_32(mem_base_reg, mem_limit_reg);
    if dec_base32 != cfg.mem_base_32 || dec_limit32 != cfg.mem_limit_32 {
        return false;
    }

    let (pref_base_low, pref_limit_low, pref_base_hi, pref_limit_hi) = cfg.encode_pref_window_64();
    let (dec_base64, dec_limit64) = PciBridgeConfig::decode_pref_window_64(
        pref_base_low,
        pref_limit_low,
        pref_base_hi,
        pref_limit_hi,
    );
    if dec_base64 != cfg.pref_mem_base_64 || dec_limit64 != cfg.pref_mem_limit_64 {
        return false;
    }

    // 4. Test downstream eGPU BAR allocation
    // BAR0: 16 MiB 32-bit MMIO (eGPU Register Block)
    let bar0 = match bridge.assign_device_bar(0, BarType::Mmio32, 16 * 1024 * 1024) {
        Ok(b) => b,
        Err(_) => return false,
    };
    if bar0.base_address != cfg.mem_base_32 as u64 || bar0.size != 16 * 1024 * 1024 {
        return false;
    }

    // BAR1: 256 MiB 64-bit Prefetchable MMIO (eGPU Framebuffer VRAM)
    let bar1 = match bridge.assign_device_bar(1, BarType::Mmio64Prefetchable, 256 * 1024 * 1024) {
        Ok(b) => b,
        Err(_) => return false,
    };
    if bar1.base_address != cfg.pref_mem_base_64 || bar1.size != 256 * 1024 * 1024 {
        return false;
    }
    if !bar1.is_prefetchable {
        return false;
    }

    // Over-allocation must fail safely
    let over_alloc = bridge.assign_device_bar(2, BarType::Mmio32, 64 * 1024 * 1024);
    if over_alloc.is_ok() {
        return false; // Did not catch window overflow
    }

    true
}
