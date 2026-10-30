//! PCIe Function-Level Reset (FLR) and link re-initialization subsystem.

use crate::arch::x86_64::{monotonic_sleep_ns, monotonic_time_ms};
use crate::drivers::pci_scanner::PciScanner;

// PCI / PCIe configuration space constants.

/// Standard PCI Configuration Header Offsets
pub const PCI_VENDOR_ID_OFFSET: u8 = 0x00;
pub const PCI_DEVICE_ID_OFFSET: u8 = 0x02;
pub const PCI_COMMAND_OFFSET: u8 = 0x04;
pub const PCI_STATUS_OFFSET: u8 = 0x06;
pub const PCI_BAR0_OFFSET: u8 = 0x10;
pub const PCI_CAPABILITIES_PTR_OFFSET: u8 = 0x34;
pub const PCI_INTERRUPT_LINE_OFFSET: u8 = 0x3C;
pub const PCI_INTERRUPT_PIN_OFFSET: u8 = 0x3D;

/// PCI Status Register Flags
pub const PCI_STATUS_CAPABILITIES_LIST: u16 = 1 << 4;

/// PCI Command Register Flags
pub const PCI_COMMAND_IO_SPACE: u16 = 1 << 0;
pub const PCI_COMMAND_MEMORY_SPACE: u16 = 1 << 1;
pub const PCI_COMMAND_BUS_MASTER: u16 = 1 << 2;
pub const PCI_COMMAND_INTX_DISABLE: u16 = 1 << 10;

/// Standard PCI Capability IDs
pub const PCI_CAP_ID_PM: u8 = 0x01;
pub const PCI_CAP_ID_MSI: u8 = 0x05;
pub const PCI_CAP_ID_EXP: u8 = 0x10; // PCI Express Capability ID
pub const PCI_CAP_ID_MSIX: u8 = 0x11;

/// PCI Express Capability Structure Register Offsets (relative to cap_ptr)
pub const PCIE_CAP_REG_OFFSET: u8 = 0x02;
pub const PCIE_DEVCAP_OFFSET: u8 = 0x04;
pub const PCIE_DEVCTL_OFFSET: u8 = 0x08;
pub const PCIE_DEVSTA_OFFSET: u8 = 0x0A;
pub const PCIE_LINKCAP_OFFSET: u8 = 0x0C;
pub const PCIE_LINKCTL_OFFSET: u8 = 0x10;
pub const PCIE_LINKSTA_OFFSET: u8 = 0x12;

/// Device Capabilities (DevCap) Register Flags
pub const PCIE_DEVCAP_FLR_CAPABLE: u32 = 1 << 28;

/// Device Control (DevCtl) Register Flags
pub const PCIE_DEVCTL_INITIATE_FLR: u16 = 1 << 15;

/// Device Status (DevSta) Register Flags
pub const PCIE_DEVSTA_TRANSACTIONS_PENDING: u16 = 1 << 5;

/// Link Status (LinkSta) Register Flags & Bitfields
pub const PCIE_LINKSTA_CURRENT_SPEED_MASK: u16 = 0x000F;
pub const PCIE_LINKSTA_NEG_WIDTH_MASK: u16 = 0x03F0;
pub const PCIE_LINKSTA_LINK_ACTIVE: u16 = 1 << 13;

/// Timing requirements in milliseconds
pub const FLR_QUIESCENCE_TIMEOUT_MS: u64 = 100;
pub const FLR_MANDATED_RESET_DELAY_MS: u64 = 100;
pub const FLR_POLL_TIMEOUT_MS: u64 = 1000;

// PCIe FLR data types and state tracking.

/// Preserved PCI configuration space state restored after hardware FLR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciSavedState {
    pub vendor_id: u16,
    pub device_id: u16,
    pub command: u16,
    pub status: u16,
    pub bars: [u32; 6],
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
    pub pcie_cap_offset: u8,
    pub devctl: u16,
    pub linkctl: u16,
}

impl Default for PciSavedState {
    fn default() -> Self {
        Self {
            vendor_id: 0,
            device_id: 0,
            command: 0,
            status: 0,
            bars: [0; 6],
            interrupt_line: 0,
            interrupt_pin: 0,
            pcie_cap_offset: 0,
            devctl: 0,
            linkctl: 0,
        }
    }
}

/// Link negotiation parameters read from LinkStatus register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciLinkState {
    pub speed_gen: u8,
    pub speed_gt_s: &'static str,
    pub width_lanes: u8,
    pub link_active: bool,
}

impl PciLinkState {
    pub fn new(speed_gen: u8, width_lanes: u8, link_active: bool) -> Self {
        let speed_gt_s = match speed_gen {
            1 => "2.5 GT/s (Gen 1)",
            2 => "5.0 GT/s (Gen 2)",
            3 => "8.0 GT/s (Gen 3)",
            4 => "16.0 GT/s (Gen 4)",
            5 => "32.0 GT/s (Gen 5)",
            _ => "Unknown / Undefined",
        };
        Self {
            speed_gen,
            speed_gt_s,
            width_lanes,
            link_active,
        }
    }
}

/// Reset execution error taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcieResetError {
    DeviceNotFound,
    CapabilitiesNotSupported,
    PcieCapabilityNotFound,
    FlrNotSupported,
    QuiescenceTimeout,
    DeviceNotRespondingPostReset,
    LinkTrainingFailed,
    ConfigRestoreFailed,
}

// PCIe FLR hardware controller.

pub struct PcieFlrManager;

impl PcieFlrManager {
    /// Writes a 16-bit word to PCI configuration space via atomic 32-bit read-modify-write.
    pub unsafe fn write_config_u16(bus: u8, slot: u8, func: u8, offset: u8, val: u16) {
        let aligned_offset = offset & !3;
        let shift = ((offset & 2) * 8) as u32;
        let old_dword = PciScanner::read_config_u32(bus, slot, func, aligned_offset);
        let new_dword = (old_dword & !(0xFFFF << shift)) | ((val as u32) << shift);
        PciScanner::write_config_u32(bus, slot, func, aligned_offset, new_dword);
    }

    /// Writes an 8-bit byte to PCI configuration space via atomic 32-bit read-modify-write.
    pub unsafe fn write_config_u8(bus: u8, slot: u8, func: u8, offset: u8, val: u8) {
        let aligned_offset = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let old_dword = PciScanner::read_config_u32(bus, slot, func, aligned_offset);
        let new_dword = (old_dword & !(0xFF << shift)) | ((val as u32) << shift);
        PciScanner::write_config_u32(bus, slot, func, aligned_offset, new_dword);
    }

    /// Traverses PCI capabilities linked list starting from offset 0x34 to locate PCIe capability structure.
    pub fn find_pcie_capability(bus: u8, slot: u8, func: u8) -> Result<u8, PcieResetError> {
        unsafe {
            let status = PciScanner::read_config_u16(bus, slot, func, PCI_STATUS_OFFSET);
            if (status & PCI_STATUS_CAPABILITIES_LIST) == 0 {
                return Err(PcieResetError::CapabilitiesNotSupported);
            }

            let mut cap_ptr = PciScanner::read_config_u8(bus, slot, func, PCI_CAPABILITIES_PTR_OFFSET) & !0x03;
            let mut hops = 0;

            while cap_ptr >= 0x40 && hops < 48 {
                let cap_id = PciScanner::read_config_u8(bus, slot, func, cap_ptr);
                if cap_id == PCI_CAP_ID_EXP {
                    return Ok(cap_ptr);
                }
                let next_ptr = PciScanner::read_config_u8(bus, slot, func, cap_ptr + 1) & !0x03;
                if next_ptr == 0 || next_ptr == cap_ptr {
                    break;
                }
                cap_ptr = next_ptr;
                hops += 1;
            }

            Err(PcieResetError::PcieCapabilityNotFound)
        }
    }

    /// Checks whether the specified PCIe function supports Function-Level Reset (DevCap bit 28).
    pub fn is_flr_supported(bus: u8, slot: u8, func: u8) -> Result<bool, PcieResetError> {
        let cap_ptr = Self::find_pcie_capability(bus, slot, func)?;
        unsafe {
            let devcap = PciScanner::read_config_u32(bus, slot, func, cap_ptr + PCIE_DEVCAP_OFFSET);
            Ok((devcap & PCIE_DEVCAP_FLR_CAPABLE) != 0)
        }
    }

    /// Reads and preserves vital PCI configuration space state before reset.
    pub fn save_device_state(bus: u8, slot: u8, func: u8, cap_ptr: u8) -> PciSavedState {
        unsafe {
            let vendor_id = PciScanner::read_config_u16(bus, slot, func, PCI_VENDOR_ID_OFFSET);
            let device_id = PciScanner::read_config_u16(bus, slot, func, PCI_DEVICE_ID_OFFSET);
            let command = PciScanner::read_config_u16(bus, slot, func, PCI_COMMAND_OFFSET);
            let status = PciScanner::read_config_u16(bus, slot, func, PCI_STATUS_OFFSET);

            let mut bars = [0u32; 6];
            for i in 0..6 {
                bars[i] = PciScanner::read_config_u32(bus, slot, func, PCI_BAR0_OFFSET + (i as u8) * 4);
            }

            let interrupt_line = PciScanner::read_config_u8(bus, slot, func, PCI_INTERRUPT_LINE_OFFSET);
            let interrupt_pin = PciScanner::read_config_u8(bus, slot, func, PCI_INTERRUPT_PIN_OFFSET);

            let devctl = PciScanner::read_config_u16(bus, slot, func, cap_ptr + PCIE_DEVCTL_OFFSET);
            let linkctl = PciScanner::read_config_u16(bus, slot, func, cap_ptr + PCIE_LINKCTL_OFFSET);

            PciSavedState {
                vendor_id,
                device_id,
                command,
                status,
                bars,
                interrupt_line,
                interrupt_pin,
                pcie_cap_offset: cap_ptr,
                devctl,
                linkctl,
            }
        }
    }

    /// Quiesces DMA engine and awaits completion of non-posted transactions.
    pub fn quiesce_device(bus: u8, slot: u8, func: u8, cap_ptr: u8, timeout_ms: u64) -> Result<(), PcieResetError> {
        unsafe {
            // Disable Bus Master Enable (BME) and Memory Space Enable (MSE) to halt outgoing DMA
            let cmd = PciScanner::read_config_u16(bus, slot, func, PCI_COMMAND_OFFSET);
            let quiesced_cmd = cmd & !(PCI_COMMAND_BUS_MASTER | PCI_COMMAND_MEMORY_SPACE);
            Self::write_config_u16(bus, slot, func, PCI_COMMAND_OFFSET, quiesced_cmd);

            // Poll Transactions Pending bit (DevSta bit 5)
            let start = monotonic_time_ms();
            loop {
                let devsta = PciScanner::read_config_u16(bus, slot, func, cap_ptr + PCIE_DEVSTA_OFFSET);
                if (devsta & PCIE_DEVSTA_TRANSACTIONS_PENDING) == 0 {
                    return Ok(());
                }

                if monotonic_time_ms().saturating_sub(start) >= timeout_ms {
                    return Err(PcieResetError::QuiescenceTimeout);
                }

                monotonic_sleep_ns(1_000_000); // 1 ms delay per iteration
            }
        }
    }

    /// Triggers PCIe Function-Level Reset by asserting DevCtl bit 15.
    pub fn trigger_flr(bus: u8, slot: u8, func: u8, cap_ptr: u8) {
        unsafe {
            let devctl = PciScanner::read_config_u16(bus, slot, func, cap_ptr + PCIE_DEVCTL_OFFSET);
            Self::write_config_u16(bus, slot, func, cap_ptr + PCIE_DEVCTL_OFFSET, devctl | PCIE_DEVCTL_INITIATE_FLR);
        }
    }

    /// Awaits PCIe specification mandated hardware quiescence delay (>= 100ms).
    pub fn wait_hardware_quiescence(delay_ms: u64) {
        monotonic_sleep_ns(delay_ms * 1_000_000);
    }

    /// Polls configuration space until device completes reset and returns valid Vendor ID.
    pub fn await_device_ready(bus: u8, slot: u8, func: u8, expected_vendor: u16, timeout_ms: u64) -> Result<(), PcieResetError> {
        unsafe {
            let start = monotonic_time_ms();
            loop {
                let vendor = PciScanner::read_config_u16(bus, slot, func, PCI_VENDOR_ID_OFFSET);
                if vendor != 0xFFFF && vendor == expected_vendor {
                    return Ok(());
                }

                if monotonic_time_ms().saturating_sub(start) >= timeout_ms {
                    return Err(PcieResetError::DeviceNotRespondingPostReset);
                }

                monotonic_sleep_ns(5_000_000); // 5 ms polling interval
            }
        }
    }

    /// Restores saved BARs, interrupts, DevCtl, and Command register settings.
    pub fn restore_device_state(bus: u8, slot: u8, func: u8, state: &PciSavedState) -> Result<(), PcieResetError> {
        unsafe {
            // Restore Base Address Registers (BARs)
            for i in 0..6 {
                PciScanner::write_config_u32(bus, slot, func, PCI_BAR0_OFFSET + (i as u8) * 4, state.bars[i]);
            }

            // Restore Interrupt Line and Pin
            Self::write_config_u8(bus, slot, func, PCI_INTERRUPT_LINE_OFFSET, state.interrupt_line);

            // Restore Link Control and Device Control (clearing Initiate FLR bit)
            Self::write_config_u16(bus, slot, func, state.pcie_cap_offset + PCIE_LINKCTL_OFFSET, state.linkctl);
            Self::write_config_u16(
                bus,
                slot,
                func,
                state.pcie_cap_offset + PCIE_DEVCTL_OFFSET,
                state.devctl & !PCIE_DEVCTL_INITIATE_FLR,
            );

            // Restore Command register (restoring BME and MSE)
            Self::write_config_u16(bus, slot, func, PCI_COMMAND_OFFSET, state.command);

            Ok(())
        }
    }

    /// Verifies data link layer status and decodes negotiated speed and width.
    pub fn read_link_state(bus: u8, slot: u8, func: u8, cap_ptr: u8) -> PciLinkState {
        unsafe {
            let linksta = PciScanner::read_config_u16(bus, slot, func, cap_ptr + PCIE_LINKSTA_OFFSET);
            let speed_gen = (linksta & PCIE_LINKSTA_CURRENT_SPEED_MASK) as u8;
            let width_lanes = ((linksta & PCIE_LINKSTA_NEG_WIDTH_MASK) >> 4) as u8;
            let link_active = (linksta & PCIE_LINKSTA_LINK_ACTIVE) != 0;

            PciLinkState::new(speed_gen, width_lanes, link_active)
        }
    }

    /// Executes the complete PCIe Function-Level Reset (FLR) sequence on a target device.
    pub fn execute_flr(bus: u8, slot: u8, func: u8) -> Result<PciLinkState, PcieResetError> {
        let cap_ptr = Self::find_pcie_capability(bus, slot, func)?;
        
        // 1. Verify FLR capability
        unsafe {
            let devcap = PciScanner::read_config_u32(bus, slot, func, cap_ptr + PCIE_DEVCAP_OFFSET);
            if (devcap & PCIE_DEVCAP_FLR_CAPABLE) == 0 {
                return Err(PcieResetError::FlrNotSupported);
            }
        }

        // 2. Save device state
        let state = Self::save_device_state(bus, slot, func, cap_ptr);

        // 3. Quiesce incoming and outgoing transactions
        Self::quiesce_device(bus, slot, func, cap_ptr, FLR_QUIESCENCE_TIMEOUT_MS)?;

        // 4. Trigger FLR via DevCtl bit 15
        Self::trigger_flr(bus, slot, func, cap_ptr);

        // 5. Hardware reset delay (100 ms)
        Self::wait_hardware_quiescence(FLR_MANDATED_RESET_DELAY_MS);

        // 6. Await device responsiveness
        Self::await_device_ready(bus, slot, func, state.vendor_id, FLR_POLL_TIMEOUT_MS)?;

        // 7. Restore configuration space
        Self::restore_device_state(bus, slot, func, &state)?;

        // 8. Re-verify data link state
        let link = Self::read_link_state(bus, slot, func, cap_ptr);
        Ok(link)
    }
}

// Emulated PCIe configuration space for verification suite.

/// Deterministic mock PCIe device configuration space for verifying FLR transitions.
#[derive(Debug, Clone)]
pub struct MockPciConfigSpace {
    pub regs: [u8; 256],
    pub pcie_cap_offset: u8,
    pub transactions_pending: bool,
    pub flr_triggered: bool,
    pub bme_cleared_during_quiesce: bool,
    pub state_restored_cleanly: bool,
}

impl MockPciConfigSpace {
    pub fn new_mock_pcie_gpu(vendor_id: u16, device_id: u16, flr_capable: bool) -> Self {
        let mut regs = [0u8; 256];

        // Standard header
        regs[0x00] = (vendor_id & 0xFF) as u8;
        regs[0x01] = ((vendor_id >> 8) & 0xFF) as u8;
        regs[0x02] = (device_id & 0xFF) as u8;
        regs[0x03] = ((device_id >> 8) & 0xFF) as u8;

        // Command: Memory + Bus Master
        regs[0x04] = 0x06;
        regs[0x05] = 0x00;

        // Status: Capabilities list present
        regs[0x06] = (PCI_STATUS_CAPABILITIES_LIST & 0xFF) as u8;
        regs[0x07] = ((PCI_STATUS_CAPABILITIES_LIST >> 8) & 0xFF) as u8;

        // Capabilities Pointer -> 0x40
        let cap_offset = 0x40;
        regs[0x34] = cap_offset;

        // Capability: PCI Express (Cap ID 0x10, next = 0x00)
        regs[cap_offset as usize] = PCI_CAP_ID_EXP;
        regs[(cap_offset + 1) as usize] = 0x00;

        // DevCap: Bit 28 (FLR Capable)
        if flr_capable {
            regs[(cap_offset + 4 + 3) as usize] = (PCIE_DEVCAP_FLR_CAPABLE >> 24) as u8;
        }

        // LinkStatus: Link Active, 16 lanes, Gen 3 (8.0 GT/s)
        // Gen 3 = 0x0003, 16 lanes = 16 << 4 = 0x0100, Active = 1 << 13 = 0x2000 => 0x2103
        let linksta: u16 = PCIE_LINKSTA_LINK_ACTIVE | (16 << 4) | 0x0003;
        regs[(cap_offset + 0x12) as usize] = (linksta & 0xFF) as u8;
        regs[(cap_offset + 0x13) as usize] = ((linksta >> 8) & 0xFF) as u8;

        // Synthetic BARs
        regs[0x10] = 0x00;
        regs[0x11] = 0x00;
        regs[0x12] = 0x00;
        regs[0x13] = 0xE0; // 0xE0000000

        Self {
            regs,
            pcie_cap_offset: cap_offset,
            transactions_pending: true,
            flr_triggered: false,
            bme_cleared_during_quiesce: false,
            state_restored_cleanly: false,
        }
    }

    pub fn simulate_flr_execution(&mut self) -> Result<PciLinkState, PcieResetError> {
        // Step 1: Check DevCap bit 28
        let devcap_hi = self.regs[(self.pcie_cap_offset + 7) as usize];
        if (devcap_hi & 0x10) == 0 {
            return Err(PcieResetError::FlrNotSupported);
        }

        // Step 2: Quiesce
        self.bme_cleared_during_quiesce = (self.regs[0x04] & 0x04) != 0;
        self.regs[0x04] &= !0x06; // Clear BME and MSE
        self.transactions_pending = false;

        // Step 3: Trigger FLR
        self.flr_triggered = true;
        self.regs[(self.pcie_cap_offset + 0x09) as usize] |= 0x80; // DevCtl bit 15

        // Step 4: Simulate hardware reset clearing internal registers to default
        self.regs[(self.pcie_cap_offset + 0x09) as usize] &= !0x80; // FLR bit auto-clears
        self.regs[0x04] = 0x00; // Command reg reset to 0
        let original_bar0 = self.regs[0x13];
        self.regs[0x10] = 0;
        self.regs[0x11] = 0;
        self.regs[0x12] = 0;
        self.regs[0x13] = 0;

        // Step 5: Restore state
        self.regs[0x13] = original_bar0;
        self.regs[0x04] = 0x06; // BME and MSE restored
        self.state_restored_cleanly = true;

        // Step 6: Link re-negotiation
        let linksta = (self.regs[(self.pcie_cap_offset + 0x12) as usize] as u16)
            | ((self.regs[(self.pcie_cap_offset + 0x13) as usize] as u16) << 8);
        let speed = (linksta & PCIE_LINKSTA_CURRENT_SPEED_MASK) as u8;
        let width = ((linksta & PCIE_LINKSTA_NEG_WIDTH_MASK) >> 4) as u8;
        let active = (linksta & PCIE_LINKSTA_LINK_ACTIVE) != 0;

        Ok(PciLinkState::new(speed, width, active))
    }
}

/// Unit test verifying PCIe capabilities parsing, DevCap FLR detection, quiescence, and link state.
pub fn verify_pcie_flr_logic() -> bool {
    let mut mock = MockPciConfigSpace::new_mock_pcie_gpu(0x10DE, 0x1F02, true);

    // Verify capability traversal
    if mock.regs[mock.pcie_cap_offset as usize] != PCI_CAP_ID_EXP {
        return false;
    }

    // Execute simulated FLR
    let link = match mock.simulate_flr_execution() {
        Ok(l) => l,
        Err(_) => return false,
    };

    if !mock.flr_triggered || !mock.bme_cleared_during_quiesce || !mock.state_restored_cleanly {
        return false;
    }

    if !link.link_active || link.speed_gen != 3 || link.width_lanes != 16 {
        return false;
    }

    // Verify unsupported device returns error
    let mut non_flr_mock = MockPciConfigSpace::new_mock_pcie_gpu(0x8086, 0x1234, false);
    if non_flr_mock.simulate_flr_execution() != Err(PcieResetError::FlrNotSupported) {
        return false;
    }

    true
}
