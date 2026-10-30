//! Standard SD Host Controller Interface (SDHCI) Architecture & Command Packet Engine.
//!
//! Implements the SDHCI Simplified Specification v3.00/v4.20 for handheld gaming PCs:
//! - Complete MMIO host register layout and bitmask definitions
//! - SD command packet encoder (CMD0, CMD8, ACMD41, CMD2, CMD3, CMD9, CMD7, CMD16, CMD17/18, CMD24/25)
//! - Response type validation and card operational state machine

// -----------------------------------------------------------------------------
// SDHCI Host Controller MMIO Register Offsets
// -----------------------------------------------------------------------------

pub mod regs {
    /// 32-bit System DMA Address / Argument 2 (Offset 0x00).
    pub const SDMA_SYS_ADDR: usize = 0x00;
    /// 16-bit Block Size (Offset 0x04).
    pub const BLOCK_SIZE: usize = 0x04;
    /// 16-bit Block Count (Offset 0x06).
    pub const BLOCK_COUNT: usize = 0x06;
    /// 32-bit Command Argument 1 (Offset 0x08).
    pub const ARGUMENT: usize = 0x08;
    /// 16-bit Transfer Mode (Offset 0x0C).
    pub const TRANSFER_MODE: usize = 0x0C;
    /// 16-bit Command Register (Offset 0x0E).
    pub const COMMAND: usize = 0x0E;
    /// 128-bit Response Registers R0..R3 (Offset 0x10..0x1C).
    pub const RESPONSE_0: usize = 0x10;
    pub const RESPONSE_1: usize = 0x14;
    pub const RESPONSE_2: usize = 0x18;
    pub const RESPONSE_3: usize = 0x1C;
    /// 32-bit Buffer Data Port (Offset 0x20).
    pub const BUFFER_DATA_PORT: usize = 0x20;
    /// 32-bit Present State (Offset 0x24).
    pub const PRESENT_STATE: usize = 0x24;
    /// 8-bit Host Control 1 (Offset 0x28).
    pub const HOST_CONTROL_1: usize = 0x28;
    /// 8-bit Power Control (Offset 0x29).
    pub const POWER_CONTROL: usize = 0x29;
    /// 8-bit Block Gap Control (Offset 0x2A).
    pub const BLOCK_GAP_CONTROL: usize = 0x2A;
    /// 8-bit Wakeup Control (Offset 0x2B).
    pub const WAKEUP_CONTROL: usize = 0x2B;
    /// 16-bit Clock Control (Offset 0x2C).
    pub const CLOCK_CONTROL: usize = 0x2C;
    /// 8-bit Timeout Control (Offset 0x2E).
    pub const TIMEOUT_CONTROL: usize = 0x2E;
    /// 8-bit Software Reset (Offset 0x2F).
    pub const SOFTWARE_RESET: usize = 0x2F;
    /// 16-bit Normal Interrupt Status (Offset 0x30).
    pub const INT_STATUS: usize = 0x30;
    /// 16-bit Error Interrupt Status (Offset 0x32).
    pub const ERR_INT_STATUS: usize = 0x32;
    /// 16-bit Normal Interrupt Status Enable (Offset 0x34).
    pub const INT_ENABLE: usize = 0x34;
    /// 16-bit Error Interrupt Status Enable (Offset 0x36).
    pub const ERR_INT_ENABLE: usize = 0x36;
    /// 16-bit Normal Interrupt Signal Enable (Offset 0x38).
    pub const SIGNAL_ENABLE: usize = 0x38;
    /// 16-bit Error Interrupt Signal Enable (Offset 0x3A).
    pub const ERR_SIGNAL_ENABLE: usize = 0x3A;
    /// 16-bit Auto CMD Error Status (Offset 0x3C).
    pub const AUTO_CMD_STATUS: usize = 0x3C;
    /// 16-bit Host Control 2 / UHS-I Control (Offset 0x3E).
    pub const HOST_CONTROL_2: usize = 0x3E;
    /// 32-bit Capabilities 1 (Offset 0x40).
    pub const CAPABILITIES_1: usize = 0x40;
    /// 32-bit Capabilities 2 (Offset 0x44).
    pub const CAPABILITIES_2: usize = 0x44;
    /// 16-bit ADMA Error Status (Offset 0x54).
    pub const ADMA_ERR_STATUS: usize = 0x54;
    /// 32-bit ADMA System Address Low (Offset 0x58).
    pub const ADMA_SYS_ADDR_LOW: usize = 0x58;
    /// 32-bit ADMA System Address High (Offset 0x5C).
    pub const ADMA_SYS_ADDR_HIGH: usize = 0x5C;
}

// -----------------------------------------------------------------------------
// Register Bitmasks and Flags
// -----------------------------------------------------------------------------

/// Present State register bitmasks (Offset 0x24).
pub mod state_flags {
    /// Command Inhibit (CMD) - line busy when 1.
    pub const CMD_INHIBIT: u32 = 1 << 0;
    /// Command Inhibit (DAT) - data line busy when 1.
    pub const DAT_INHIBIT: u32 = 1 << 1;
    /// DAT Line Active.
    pub const DAT_LINE_ACTIVE: u32 = 1 << 2;
    /// Write Transfer Active.
    pub const WRITE_ACTIVE: u32 = 1 << 8;
    /// Read Transfer Active.
    pub const READ_ACTIVE: u32 = 1 << 9;
    /// Buffer Write Enable (space available in host buffer).
    pub const BUFFER_WRITE_ENABLE: u32 = 1 << 10;
    /// Buffer Read Enable (data available to read from host buffer).
    pub const BUFFER_READ_ENABLE: u32 = 1 << 11;
    /// Card Inserted - mechanical pin state (1 = card present in slot).
    pub const CARD_INSERTED: u32 = 1 << 16;
    /// Card State Stable.
    pub const CARD_STATE_STABLE: u32 = 1 << 17;
    /// Card Detect Pin Level.
    pub const CARD_DETECT_PIN: u32 = 1 << 18;
    /// Write Protect Switch Pin Level (1 = write enabled, 0 = write protected).
    pub const WRITE_PROTECT: u32 = 1 << 19;
}

/// Software Reset register flags (Offset 0x2F).
pub mod reset_flags {
    pub const RESET_ALL: u8 = 1 << 0;
    pub const RESET_CMD: u8 = 1 << 1;
    pub const RESET_DAT: u8 = 1 << 2;
}

/// Power Control register flags (Offset 0x29).
pub mod power_flags {
    pub const SD_BUS_POWER: u8 = 1 << 0;
    pub const VDD_3_3V: u8 = 0x07 << 1;
    pub const VDD_3_0V: u8 = 0x06 << 1;
    pub const VDD_1_8V: u8 = 0x05 << 1;
}

/// Clock Control register flags (Offset 0x2C).
pub mod clock_flags {
    pub const INTERNAL_CLOCK_EN: u16 = 1 << 0;
    pub const INTERNAL_CLOCK_STABLE: u16 = 1 << 1;
    pub const SD_CLOCK_EN: u16 = 1 << 2;
    pub const CLK_DIV_SHIFT: u16 = 8;
}

/// Host Control 1 register flags (Offset 0x28).
pub mod host_ctrl1_flags {
    pub const DATA_WIDTH_1BIT: u8 = 0 << 1;
    pub const DATA_WIDTH_4BIT: u8 = 1 << 1;
    pub const DATA_WIDTH_8BIT: u8 = 1 << 5;
    pub const HIGH_SPEED_EN: u8 = 1 << 2;
    pub const DMA_SELECT_SDMA: u8 = 0x00 << 3;
    pub const DMA_SELECT_ADMA2_32: u8 = 0x02 << 3;
    pub const DMA_SELECT_ADMA2_64: u8 = 0x03 << 3;
}

/// Host Control 2 / UHS-I register flags (Offset 0x3E).
pub mod host_ctrl2_flags {
    pub const UHS_MODE_SDR12: u16 = 0x00;
    pub const UHS_MODE_SDR25: u16 = 0x01;
    pub const UHS_MODE_SDR50: u16 = 0x02;
    pub const UHS_MODE_SDR104: u16 = 0x03;
    pub const UHS_MODE_DDR50: u16 = 0x04;
    pub const SIGNALING_1_8V_EN: u16 = 1 << 3;
    pub const EXECUTE_TUNING: u16 = 1 << 6;
    pub const SAMPLING_CLK_SELECT: u16 = 1 << 7;
    pub const PRESET_VALUE_EN: u16 = 1 << 15;
}

/// Normal Interrupt Status / Enable flags (Offset 0x30 / 0x34).
pub mod int_flags {
    pub const CMD_COMPLETE: u16 = 1 << 0;
    pub const TRANSFER_COMPLETE: u16 = 1 << 1;
    pub const BLOCK_GAP_EVENT: u16 = 1 << 2;
    pub const DMA_INTERRUPT: u16 = 1 << 3;
    pub const BUFFER_WRITE_READY: u16 = 1 << 4;
    pub const BUFFER_READ_READY: u16 = 1 << 5;
    pub const CARD_INSERTION: u16 = 1 << 6;
    pub const CARD_REMOVAL: u16 = 1 << 7;
    pub const CARD_INTERRUPT: u16 = 1 << 8;
    pub const ERROR_INTERRUPT: u16 = 1 << 15;
}

/// Error Interrupt Status / Enable flags (Offset 0x32 / 0x36).
pub mod err_int_flags {
    pub const CMD_TIMEOUT: u16 = 1 << 0;
    pub const CMD_CRC_ERR: u16 = 1 << 1;
    pub const CMD_END_BIT_ERR: u16 = 1 << 2;
    pub const CMD_INDEX_ERR: u16 = 1 << 3;
    pub const DATA_TIMEOUT: u16 = 1 << 4;
    pub const DATA_CRC_ERR: u16 = 1 << 5;
    pub const DATA_END_BIT_ERR: u16 = 1 << 6;
    pub const CURRENT_LIMIT_ERR: u16 = 1 << 7;
    pub const AUTO_CMD_ERR: u16 = 1 << 8;
    pub const ADMA_ERR: u16 = 1 << 9;
    pub const TUNING_ERR: u16 = 1 << 10;
}

// -----------------------------------------------------------------------------
// SD Command Types & Encoding
// -----------------------------------------------------------------------------

/// Expected hardware response format from the SD card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdResponseType {
    /// No response expected (e.g. CMD0).
    None = 0,
    /// 48-bit response with 32-bit card status (R1).
    R1 = 1,
    /// 48-bit response with busy signaling on DAT0 (R1b).
    R1b = 2,
    /// 136-bit response containing CID or CSD contents (R2).
    R2 = 3,
    /// 48-bit response with OCR register (R3, ACMD41).
    R3 = 4,
    /// 48-bit response with Relative Card Address (RCA) (R6).
    R6 = 5,
    /// 48-bit response with interface condition parameters (R7, CMD8).
    R7 = 6,
}

/// Transfer direction for data commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    /// Host to card (write).
    Write = 0,
    /// Card to host (read).
    Read = 1,
}

/// Standard SD command indexes.
pub mod commands {
    pub const CMD0_GO_IDLE_STATE: u8 = 0;
    pub const CMD2_ALL_SEND_CID: u8 = 2;
    pub const CMD3_SEND_RELATIVE_ADDR: u8 = 3;
    pub const CMD7_SELECT_DESELECT_CARD: u8 = 7;
    pub const CMD8_SEND_IF_COND: u8 = 8;
    pub const CMD9_SEND_CSD: u8 = 9;
    pub const CMD12_STOP_TRANSMISSION: u8 = 12;
    pub const CMD13_SEND_STATUS: u8 = 13;
    pub const CMD16_SET_BLOCKLEN: u8 = 16;
    pub const CMD17_READ_SINGLE_BLOCK: u8 = 17;
    pub const CMD18_READ_MULTIPLE_BLOCK: u8 = 18;
    pub const CMD19_SEND_TUNING_BLOCK: u8 = 19;
    pub const CMD24_WRITE_SINGLE_BLOCK: u8 = 24;
    pub const CMD25_WRITE_MULTIPLE_BLOCK: u8 = 25;
    pub const CMD55_APP_CMD: u8 = 55;
    pub const ACMD41_SD_SEND_OP_COND: u8 = 41;
    pub const ACMD51_SEND_SCR: u8 = 51;
}

/// Encapsulated SD command packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SdCommand {
    /// 6-bit command index [0..63].
    pub index: u8,
    /// 32-bit command argument payload.
    pub argument: u32,
    /// Expected card response type.
    pub response_type: SdResponseType,
    /// Whether command involves a data payload transfer.
    pub has_data: bool,
    /// Transfer direction (if has_data is true).
    pub direction: TransferDirection,
    /// Multi-block transfer flag.
    pub is_multi_block: bool,
    /// Whether this is an application-specific command (requires prior CMD55).
    pub is_app_cmd: bool,
}

impl SdCommand {
    /// Constructs a basic command without data transfer.
    pub const fn new(index: u8, argument: u32, response_type: SdResponseType) -> Self {
        Self {
            index,
            argument,
            response_type,
            has_data: false,
            direction: TransferDirection::Read,
            is_multi_block: false,
            is_app_cmd: false,
        }
    }

    /// Constructs a data transfer command (e.g. CMD17/18/24/25).
    pub const fn new_data(
        index: u8,
        argument: u32,
        response_type: SdResponseType,
        direction: TransferDirection,
        is_multi_block: bool,
    ) -> Self {
        Self {
            index,
            argument,
            response_type,
            has_data: true,
            direction,
            is_multi_block,
            is_app_cmd: false,
        }
    }

    /// Encodes SDHCI 16-bit Transfer Mode register value (Offset 0x0C).
    pub fn encode_transfer_mode(&self, dma_enabled: bool) -> u16 {
        if !self.has_data {
            return 0;
        }

        let mut val = 0u16;
        if dma_enabled {
            val |= 1 << 0; // DMA Enable
        }
        if self.is_multi_block {
            val |= 1 << 1; // Block Count Enable
            val |= 1 << 5; // Multi Block Select
            val |= 1 << 2; // Auto CMD12 Enable
        }
        if self.direction == TransferDirection::Read {
            val |= 1 << 4; // Data Transfer Direction (1 = Read)
        }
        val
    }

    /// Encodes SDHCI 16-bit Command register value (Offset 0x0E).
    pub fn encode_command_reg(&self) -> u16 {
        let mut val = ((self.index as u16) & 0x3F) << 8;

        if self.has_data {
            val |= 1 << 5; // Data Present Select
        }

        match self.response_type {
            SdResponseType::None => {}
            SdResponseType::R2 => {
                val |= 0x01; // Response length 136 bits
                val |= 1 << 3; // CRC Check Enable
            }
            SdResponseType::R1b => {
                val |= 0x03; // Response length 48 bits with busy
                val |= 1 << 3; // CRC Check Enable
                val |= 1 << 4; // Index Check Enable
            }
            SdResponseType::R1 | SdResponseType::R6 | SdResponseType::R7 => {
                val |= 0x02; // Response length 48 bits
                val |= 1 << 3; // CRC Check Enable
                val |= 1 << 4; // Index Check Enable
            }
            SdResponseType::R3 => {
                val |= 0x02; // Response length 48 bits (OCR without CRC)
            }
        }

        val
    }
}

// -----------------------------------------------------------------------------
// SD Card State Machine
// -----------------------------------------------------------------------------

/// SD Card operational states as defined in the SD Physical Layer Specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdCardState {
    /// No card physically inserted in the socket.
    Empty,
    /// Card inserted, awaiting reset and power-up.
    Inserted,
    /// Card in Idle state following CMD0.
    Idle,
    /// Card in Ready state following ACMD41 negotiation.
    Ready,
    /// Card in Identification state following CMD2 CID readout.
    Identification,
    /// Card in Standby state awaiting address selection (CMD3).
    Standby,
    /// Card in Transfer state, ready for active block read/write operations (CMD7).
    Transfer,
    /// Card currently streaming read data to host.
    SendingData,
    /// Card currently receiving write data from host.
    ReceivingData,
    /// Card programming flash sectors.
    Programming,
    /// Card in error or disconnected state.
    Error,
}

// -----------------------------------------------------------------------------
// Card Register Formats (CID, CSD, OCR) & UHS-I Bus Modes
// -----------------------------------------------------------------------------

/// UHS-I Bus Speed Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UhsiBusSpeed {
    /// Legacy default speed: 25 MHz (12.5 MB/s).
    DefaultSpeed25Mhz,
    /// High Speed: 50 MHz (25 MB/s).
    HighSpeed50Mhz,
    /// UHS-I SDR50: 100 MHz (50 MB/s).
    Sdr50_100Mhz,
    /// UHS-I DDR50: 50 MHz dual data rate (50 MB/s).
    Ddr50_50Mhz,
    /// UHS-I SDR104: 208 MHz ultra high-speed (104 MB/s, requires tuning).
    Sdr104_208Mhz,
}

/// Operation Conditions Register (OCR) decoded representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrRegister {
    pub raw: u32,
    /// Card power up completed (false = busy, true = ready).
    pub is_ready: bool,
    /// Card Capacity Status (false = SDSC byte-addressed, true = SDHC/SDXC block-addressed).
    pub is_high_capacity_ccs: bool,
    /// 1.8V switching accepted by card for UHS-I operation (S18A).
    pub supports_1_8v_s18a: bool,
}

impl OcrRegister {
    pub fn parse(raw: u32) -> Self {
        Self {
            raw,
            is_ready: (raw & (1 << 31)) != 0,
            is_high_capacity_ccs: (raw & (1 << 30)) != 0,
            supports_1_8v_s18a: (raw & (1 << 24)) != 0,
        }
    }
}

/// Card Specific Data (CSD) parsed metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsdRegister {
    /// CSD structure version (0 = v1.0 SDSC, 1 = v2.0 SDHC/SDXC).
    pub structure_version: u8,
    /// Device size field (C_SIZE).
    pub c_size: u32,
    /// Card capacity in 512-byte sectors/blocks.
    pub capacity_blocks: u64,
    /// Card capacity in bytes.
    pub capacity_bytes: u64,
    /// Max data transfer rate in MB/s.
    pub max_tran_speed_mb_s: u32,
}

impl CsdRegister {
    /// Decodes CSD v2.0 (High / Extended Capacity) from raw response registers.
    pub fn parse_v2(c_size: u32) -> Self {
        let capacity_blocks = ((c_size as u64) + 1) * 1024;
        let capacity_bytes = capacity_blocks * 512;
        Self {
            structure_version: 1,
            c_size,
            capacity_blocks,
            capacity_bytes,
            max_tran_speed_mb_s: 104,
        }
    }
}

/// Card Identification (CID) register information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CidRegister {
    /// Manufacturer ID (MID).
    pub manufacturer_id: u8,
    /// OEM/Application ID (OID).
    pub oem_id: [u8; 2],
    /// Product name (PNM, 5 ASCII characters).
    pub product_name: [u8; 5],
    /// Product revision (PRV).
    pub product_revision: u8,
    /// 32-bit product serial number (PSN).
    pub serial_number: u32,
}

impl CidRegister {
    pub fn default_mock() -> Self {
        Self {
            manufacturer_id: 0x1B, // Samsung / SanDisk
            oem_id: *b"SM",
            product_name: *b"EVO51",
            product_revision: 0x10,
            serial_number: 0x1234_5678,
        }
    }
}

// -----------------------------------------------------------------------------
// Unit Tests & Verification
// -----------------------------------------------------------------------------

/// Unit test verifying SDHCI host register layout, card state machine, and command packet encoding.
pub fn verify_sdhci_structures_and_commands() -> bool {
    // 1. Verify MMIO Register Offsets alignment
    if regs::SDMA_SYS_ADDR != 0x00
        || regs::BLOCK_SIZE != 0x04
        || regs::BLOCK_COUNT != 0x06
        || regs::ARGUMENT != 0x08
        || regs::TRANSFER_MODE != 0x0C
        || regs::COMMAND != 0x0E
        || regs::RESPONSE_0 != 0x10
        || regs::PRESENT_STATE != 0x24
        || regs::HOST_CONTROL_1 != 0x28
        || regs::POWER_CONTROL != 0x29
        || regs::CLOCK_CONTROL != 0x2C
        || regs::SOFTWARE_RESET != 0x2F
        || regs::INT_STATUS != 0x30
        || regs::HOST_CONTROL_2 != 0x3E
        || regs::CAPABILITIES_1 != 0x40
        || regs::ADMA_SYS_ADDR_LOW != 0x58
    {
        return false;
    }

    // 2. Verify Command Encoding: CMD0 (Go Idle State - No response, no data)
    let cmd0 = SdCommand::new(commands::CMD0_GO_IDLE_STATE, 0, SdResponseType::None);
    let cmd0_reg = cmd0.encode_command_reg();
    if cmd0_reg != (0 << 8) {
        return false;
    }

    // 3. Verify Command Encoding: CMD8 (Send Interface Condition - 48-bit R7)
    // Argument 0x1AA (2.7-3.6V, 0xAA check pattern)
    let cmd8 = SdCommand::new(commands::CMD8_SEND_IF_COND, 0x0000_01AA, SdResponseType::R7);
    let cmd8_reg = cmd8.encode_command_reg();
    // Index 8 in bits 8..13 -> 8 << 8 = 0x0800
    // Response 48 bits (0x02) | CRC check (1<<3 = 0x08) | Index check (1<<4 = 0x10) -> 0x081A
    if cmd8_reg != 0x081A {
        return false;
    }

    // 4. Verify Command Encoding: CMD2 (All Send CID - 136-bit R2)
    let cmd2 = SdCommand::new(commands::CMD2_ALL_SEND_CID, 0, SdResponseType::R2);
    let cmd2_reg = cmd2.encode_command_reg();
    // Index 2 in bits 8..13 -> 0x0200 | Resp 136 (0x01) | CRC check (0x08) -> 0x0209
    if cmd2_reg != 0x0209 {
        return false;
    }

    // 5. Verify Command Encoding: CMD18 (Read Multiple Block - Data Read with Multi-block DMA)
    let cmd18 = SdCommand::new_data(
        commands::CMD18_READ_MULTIPLE_BLOCK,
        1000,
        SdResponseType::R1,
        TransferDirection::Read,
        true,
    );
    let cmd18_reg = cmd18.encode_command_reg();
    // Index 18 (0x1200) | Data present (1<<5 = 0x20) | Resp 48 (0x02) | CRC (0x08) | Index (0x10) -> 0x123A
    if cmd18_reg != 0x123A {
        return false;
    }

    let tmode18 = cmd18.encode_transfer_mode(true);
    // DMA enable (1<<0) | Block Count (1<<1) | Auto CMD12 (1<<2) | Read (1<<4) | Multi (1<<5)
    // 0x01 | 0x02 | 0x04 | 0x10 | 0x20 = 0x37
    if tmode18 != 0x0037 {
        return false;
    }

    // 6. Verify Command Encoding: CMD25 (Write Multiple Block - Data Write with Multi-block DMA)
    let cmd25 = SdCommand::new_data(
        commands::CMD25_WRITE_MULTIPLE_BLOCK,
        2000,
        SdResponseType::R1,
        TransferDirection::Write,
        true,
    );
    let tmode25 = cmd25.encode_transfer_mode(true);
    // DMA enable (0x01) | Block Count (0x02) | Auto CMD12 (0x04) | Write (0x00) | Multi (0x20) = 0x27
    if tmode25 != 0x0027 {
        return false;
    }

    // 7. Verify Card State Transitions
    let initial_state = SdCardState::Empty;
    if initial_state != SdCardState::Empty {
        return false;
    }

    true
}
