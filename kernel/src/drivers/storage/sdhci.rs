//! SDHCI Host Controller Driver & Hardware Abstraction Layer.
//!
//! Provides memory-mapped register access, ADMA2 scatter-gather DMA engine,
//! and UHS-I bus speed tuning for handheld PC MicroSD host controllers.

pub use super::sdcard::*;

// -----------------------------------------------------------------------------
// ADMA2 (Advanced Direct Memory Access 2) Definitions
// -----------------------------------------------------------------------------

/// ADMA2 Descriptor Attribute Bitflags.
pub mod adma2_attr {
    /// Valid descriptor entry (hardware will process if 1).
    pub const VALID: u16 = 1 << 0;
    /// End of descriptor list (last descriptor in chain).
    pub const END: u16 = 1 << 1;
    /// Generate DMA interrupt upon completion of this descriptor.
    pub const INT: u16 = 1 << 2;
    /// Action: No operation.
    pub const ACT_NOP: u16 = 0x00 << 4;
    /// Action: Transfer data buffer.
    pub const ACT_TRAN: u16 = 0x02 << 4;
    /// Action: Link to next descriptor table address.
    pub const ACT_LINK: u16 = 0x03 << 4;
}

/// 64-bit ADMA2 Scatter-Gather Descriptor (12 bytes, 32-bit aligned).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adma2Descriptor64 {
    /// Descriptor attributes (Valid, End, Int, Action).
    pub attribute: u16,
    /// Transfer byte length (0 = 65536 bytes).
    pub length: u16,
    /// 64-bit physical host memory buffer address.
    pub address: u64,
}

impl Adma2Descriptor64 {
    pub const fn empty() -> Self {
        Self {
            attribute: 0,
            length: 0,
            address: 0,
        }
    }
}

/// Maximum number of scatter-gather entries per ADMA2 transaction.
pub const MAX_ADMA2_DESCRIPTORS: usize = 32;

/// Fixed-capacity scatter-gather descriptor table for ADMA2 DMA transfers.
#[derive(Debug, Clone)]
pub struct Adma2Table {
    /// Array of hardware ADMA2 descriptors.
    pub descriptors: [Adma2Descriptor64; MAX_ADMA2_DESCRIPTORS],
    /// Number of active descriptors populated.
    pub count: usize,
    /// Total bytes spanned by this ADMA2 descriptor table.
    pub total_bytes: usize,
}

impl Adma2Table {
    pub fn new() -> Self {
        Self {
            descriptors: [Adma2Descriptor64::empty(); MAX_ADMA2_DESCRIPTORS],
            count: 0,
            total_bytes: 0,
        }
    }

    /// Builds a scatter-gather descriptor chain for a physical buffer.
    ///
    /// Automatically fragments transfers exceeding 65536 bytes into 64KiB chunks
    /// and sets the terminal `END` attribute on the final descriptor.
    pub fn build_transfer(&mut self, mut physical_addr: u64, mut bytes_remaining: usize) -> bool {
        self.count = 0;
        self.total_bytes = bytes_remaining;

        while bytes_remaining > 0 {
            if self.count >= MAX_ADMA2_DESCRIPTORS {
                return false; // Scatter-gather list overflow
            }

            let chunk_size = bytes_remaining.min(65536);
            let is_last = chunk_size == bytes_remaining;

            let mut attr = adma2_attr::VALID | adma2_attr::ACT_TRAN;
            if is_last {
                attr |= adma2_attr::END;
            }

            // In SDHCI spec, length field 0 encodes 65536 bytes
            let len_encoded = if chunk_size == 65536 { 0u16 } else { chunk_size as u16 };

            self.descriptors[self.count] = Adma2Descriptor64 {
                attribute: attr,
                length: len_encoded,
                address: physical_addr,
            };

            self.count += 1;
            physical_addr += chunk_size as u64;
            bytes_remaining -= chunk_size;
        }

        true
    }
}

// -----------------------------------------------------------------------------
// SDHCI Host Controller Instance & State Machine
// -----------------------------------------------------------------------------

/// SDHCI Host Controller Instance for Handheld PC Game Storage Expansion.
#[derive(Debug)]
pub struct SdhciHost {
    /// Memory-mapped base physical address of the SDHCI controller.
    pub base_addr: usize,
    /// Whether controller operates in 64-bit ADMA2 addressing mode.
    pub is_adma64: bool,
    /// Current bus clock frequency in kilohertz.
    pub current_clock_khz: u32,
    /// Active UHS-I bus speed mode.
    pub bus_speed: UhsiBusSpeed,
    /// Cached physical card insertion status.
    pub card_inserted: bool,
    /// Current operational card state.
    pub card_state: SdCardState,
    /// Assigned Relative Card Address (RCA).
    pub rca: u16,
    /// Total capacity in 512-byte blocks.
    pub block_count: u64,
    /// Cached Card Identification (CID).
    pub cid: Option<CidRegister>,
    /// Cached Card Specific Data (CSD).
    pub csd: Option<CsdRegister>,
    /// Active ADMA2 descriptor table.
    pub adma_table: Adma2Table,
}

impl SdhciHost {
    /// Creates a new SDHCI host controller instance.
    pub fn new(base_addr: usize) -> Self {
        Self {
            base_addr,
            is_adma64: true,
            current_clock_khz: 400, // 400 kHz identification frequency
            bus_speed: UhsiBusSpeed::DefaultSpeed25Mhz,
            card_inserted: false,
            card_state: SdCardState::Empty,
            rca: 0,
            block_count: 0,
            cid: None,
            csd: None,
            adma_table: Adma2Table::new(),
        }
    }

    /// Handles card insertion event detected by card-detect mechanical pin.
    pub fn on_card_inserted(&mut self) {
        self.card_inserted = true;
        self.card_state = SdCardState::Inserted;
    }

    /// Handles card removal event.
    pub fn on_card_removed(&mut self) {
        self.card_inserted = false;
        self.card_state = SdCardState::Empty;
        self.rca = 0;
        self.block_count = 0;
        self.cid = None;
        self.csd = None;
        self.bus_speed = UhsiBusSpeed::DefaultSpeed25Mhz;
        self.current_clock_khz = 400;
    }

    /// Performs UHS-I bus tuning sequence (CMD19) to lock sampling clock phase for SDR104 mode.
    pub fn tune_uhs_sdr104(&mut self) -> Result<UhsiBusSpeed, &'static str> {
        if self.card_state != SdCardState::Transfer {
            return Err("Card not in Transfer state for tuning");
        }

        // Simulate sending 40 CMD19 tuning patterns to align sampling clock phase
        let mut tuning_success = false;
        for attempt in 0..40 {
            // Simulated host sampling clock phase search: succeeds on attempt 12
            if attempt >= 12 {
                tuning_success = true;
                break;
            }
        }

        if !tuning_success {
            return Err("UHS-I SDR104 tuning failed");
        }

        // Transition host and card to SDR104 (208 MHz, 104 MB/s)
        self.bus_speed = UhsiBusSpeed::Sdr104_208Mhz;
        self.current_clock_khz = 208_000;
        Ok(self.bus_speed)
    }

    /// Executes the full SD card enumeration and initialization sequence.
    pub fn initialize_card(&mut self) -> Result<u64, &'static str> {
        if !self.card_inserted {
            return Err("No MicroSD card inserted in slot");
        }

        // Step 1: Power-up bus and set identification clock to 400 kHz
        self.current_clock_khz = 400;

        // Step 2: CMD0 (Go Idle State)
        self.card_state = SdCardState::Idle;

        // Step 3: CMD8 (Send Interface Condition - 3.3V, check pattern 0xAA)
        let cmd8 = SdCommand::new(commands::CMD8_SEND_IF_COND, 0x0000_01AA, SdResponseType::R7);
        let _ = cmd8.encode_command_reg();

        // Step 4: ACMD41 negotiation (HCS=1, S18R=1 for UHS-I 1.8V switching)
        // Simulated OCR: Ready (bit 31), CCS=1 (bit 30), S18A=1 (bit 24), 3.3V window
        let raw_ocr: u32 = (1 << 31) | (1 << 30) | (1 << 24) | 0x00FF_8000;
        let ocr = OcrRegister::parse(raw_ocr);
        if !ocr.is_ready || !ocr.is_high_capacity_ccs {
            return Err("Card initialization rejected by ACMD41");
        }
        self.card_state = SdCardState::Ready;

        // Step 5: CMD2 (All Send CID)
        self.cid = Some(CidRegister::default_mock());
        self.card_state = SdCardState::Identification;

        // Step 6: CMD3 (Send Relative Address)
        self.rca = 0xAAAA;
        self.card_state = SdCardState::Standby;

        // Step 7: CMD9 (Send CSD) - Read capacity
        // Simulated CSD v2.0 for a 512GB MicroSDXC card (c_size = 1,048,575)
        let csd = CsdRegister::parse_v2(1_048_575);
        self.block_count = csd.capacity_blocks;
        self.csd = Some(csd);

        // Step 8: CMD7 (Select Card) -> Card enters Transfer state
        self.card_state = SdCardState::Transfer;

        // Step 9: ACMD6 (Switch to 4-bit bus width)
        // Step 10: UHS-I SDR104 bus tuning
        self.tune_uhs_sdr104()?;

        Ok(self.block_count)
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying card initialization, UHS-I bus tuning, and ADMA2 DMA engine.
pub fn verify_card_init_and_adma2() -> bool {
    let mut host = SdhciHost::new(0xFE80_0000);

    // 1. Initial State: No card inserted
    if host.card_inserted || host.card_state != SdCardState::Empty {
        return false;
    }

    // 2. Insert card
    host.on_card_inserted();
    if !host.card_inserted || host.card_state != SdCardState::Inserted {
        return false;
    }

    // 3. Initialize Card
    let capacity_blocks = match host.initialize_card() {
        Ok(b) => b,
        Err(_) => return false,
    };

    // Capacity must be 512 GiB = 1,073,741,824 blocks of 512 bytes
    if capacity_blocks != 1_073_741_824 {
        return false;
    }
    if host.card_state != SdCardState::Transfer {
        return false;
    }
    if host.rca != 0xAAAA {
        return false;
    }

    // 4. Verify UHS-I SDR104 Bus Speed & Clock Frequency
    if host.bus_speed != UhsiBusSpeed::Sdr104_208Mhz || host.current_clock_khz != 208_000 {
        return false;
    }

    // 5. Verify ADMA2 Scatter-Gather Descriptor Table Engine
    let mut adma = Adma2Table::new();

    // Test A: Single 32 KiB buffer
    if !adma.build_transfer(0x1000_0000, 32768) {
        return false;
    }
    if adma.count != 1 {
        return false;
    }
    let d0 = adma.descriptors[0];
    if d0.attribute != (adma2_attr::VALID | adma2_attr::ACT_TRAN | adma2_attr::END) {
        return false;
    }
    if d0.length != 32768 || d0.address != 0x1000_0000 {
        return false;
    }

    // Test B: Large 192 KiB transfer (must fragment into three 64KiB descriptors: 65536 + 65536 + 65536)
    if !adma.build_transfer(0x2000_0000, 196608) {
        return false;
    }
    if adma.count != 3 {
        return false;
    }

    // Descriptor 0: 64 KiB, Valid, Tran (not End)
    let d0 = adma.descriptors[0];
    if d0.attribute != (adma2_attr::VALID | adma2_attr::ACT_TRAN) || d0.length != 0 {
        return false;
    }
    if d0.address != 0x2000_0000 {
        return false;
    }

    // Descriptor 1: 64 KiB, Valid, Tran (not End)
    let d1 = adma.descriptors[1];
    if d1.attribute != (adma2_attr::VALID | adma2_attr::ACT_TRAN) || d1.length != 0 {
        return false;
    }
    if d1.address != 0x2001_0000 {
        return false;
    }

    // Descriptor 2: 64 KiB, Valid, Tran, END
    let d2 = adma.descriptors[2];
    if d2.attribute != (adma2_attr::VALID | adma2_attr::ACT_TRAN | adma2_attr::END) || d2.length != 0 {
        return false;
    }
    if d2.address != 0x2002_0000 {
        return false;
    }

    // 6. Test Card Removal
    host.on_card_removed();
    if host.card_inserted || host.card_state != SdCardState::Empty || host.block_count != 0 {
        return false;
    }

    true
}
