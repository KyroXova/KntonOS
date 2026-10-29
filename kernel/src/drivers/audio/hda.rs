//! Intel High Definition Audio (HDA) Specification 1.0a Controller & MMIO Registers.
//!
//! Provides memory-mapped register definitions, bitmask constants, Buffer Descriptor
//! List (BDL) structures, and Command/Response Ring Buffers (CORB / RIRB).

use crate::drivers::pci_scanner::PciDevice;

pub const PCI_CLASS_MULTIMEDIA: u8 = 0x04;
pub const PCI_SUBCLASS_HDA: u8 = 0x03;

// --- Intel HDA Global Registers (Specification Section 3.3) ---
pub const HDA_REG_GCAP: usize = 0x00;     // Global Capabilities (16-bit)
pub const HDA_REG_VMIN: usize = 0x02;     // Minor Version (8-bit)
pub const HDA_REG_VMAJ: usize = 0x03;     // Major Version (8-bit)
pub const HDA_REG_OUTPAY: usize = 0x04;   // Output Payload Capability (16-bit)
pub const HDA_REG_INPAY: usize = 0x06;    // Input Payload Capability (16-bit)
pub const HDA_REG_GCTL: usize = 0x08;     // Global Control (32-bit)
pub const HDA_REG_WAKEEN: usize = 0x0C;   // Wake Enable (16-bit)
pub const HDA_REG_STATESTS: usize = 0x0E; // State Change Status (16-bit)
pub const HDA_REG_GSTS: usize = 0x10;     // Global Status (16-bit)
pub const HDA_REG_INTCTL: usize = 0x20;   // Interrupt Control (32-bit)
pub const HDA_REG_INTSTS: usize = 0x24;   // Interrupt Status (32-bit)
pub const HDA_REG_WALCLK: usize = 0x30;   // Wall Clock Counter (32-bit)
pub const HDA_REG_SSYNC: usize = 0x38;    // Stream Synchronization (32-bit)

// --- Command & Output Ring Buffer (CORB) Registers (Offset 0x40) ---
pub const HDA_REG_CORBLBASE: usize = 0x40; // CORB Lower Base Address (32-bit)
pub const HDA_REG_CORBUBASE: usize = 0x44; // CORB Upper Base Address (32-bit)
pub const HDA_REG_CORBWP: usize = 0x48;    // CORB Write Pointer (16-bit)
pub const HDA_REG_CORBRP: usize = 0x4A;    // CORB Read Pointer (16-bit)
pub const HDA_REG_CORBCTL: usize = 0x4C;   // CORB Control (8-bit)
pub const HDA_REG_CORBST: usize = 0x4D;    // CORB Status (8-bit)
pub const HDA_REG_CORBSIZE: usize = 0x4E;  // CORB Size (8-bit)

// --- Response Input Ring Buffer (RIRB) Registers (Offset 0x50) ---
pub const HDA_REG_RIRBLBASE: usize = 0x50; // RIRB Lower Base Address (32-bit)
pub const HDA_REG_RIRBUBASE: usize = 0x54; // RIRB Upper Base Address (32-bit)
pub const HDA_REG_RIRBWP: usize = 0x58;    // RIRB Write Pointer (16-bit)
pub const HDA_REG_RINTCNT: usize = 0x5A;   // Response Interrupt Count (16-bit)
pub const HDA_REG_RIRBCTL: usize = 0x5C;   // RIRB Control (8-bit)
pub const HDA_REG_RIRBSTS: usize = 0x5D;   // RIRB Status (8-bit)
pub const HDA_REG_RIRBSIZE: usize = 0x5E;  // RIRB Size (8-bit)

// --- Immediate Command / Response Registers ---
pub const HDA_REG_ICOI: usize = 0x60;     // Immediate Command Output (32-bit)
pub const HDA_REG_ICII: usize = 0x64;     // Immediate Command Input (32-bit)
pub const HDA_REG_ICIS: usize = 0x68;     // Immediate Command Status (16-bit)

// --- Stream Descriptor Registers (Base 0x80, Stride 0x20) ---
pub const HDA_SD_BASE: usize = 0x80;
pub const HDA_SD_STRIDE: usize = 0x20;

pub const SD_REG_CTL: usize = 0x00;       // Stream Control (24-bit / 32-bit)
pub const SD_REG_STS: usize = 0x03;       // Stream Status (8-bit)
pub const SD_REG_LPIB: usize = 0x04;      // Link Position in Buffer (32-bit)
pub const SD_REG_CBL: usize = 0x08;       // Cyclic Buffer Length (32-bit)
pub const SD_REG_LVI: usize = 0x0C;       // Last Valid Index (16-bit)
pub const SD_REG_FIFOS: usize = 0x10;     // FIFO Size (16-bit)
pub const SD_REG_FMT: usize = 0x12;       // Stream Format (16-bit)
pub const SD_REG_BDPL: usize = 0x18;      // BDL Lower Base (32-bit)
pub const SD_REG_BDPU: usize = 0x1C;      // BDL Upper Base (32-bit)

// --- Control and Status Bitmasks ---
pub const GCTL_CRST: u32 = 1 << 0;        // Controller Reset (0: in reset, 1: operational)
pub const GCTL_FCNTRL: u32 = 1 << 1;      // Flush Control
pub const GCTL_UNSOL: u32 = 1 << 8;       // Accept Unsolicited Responses

pub const CORBCTL_RUN: u8 = 1 << 1;       // Start CORB DMA engine
pub const CORBCTL_MEIE: u8 = 1 << 0;      // Memory Error Interrupt Enable
pub const CORBRP_RST: u16 = 1 << 15;      // Reset Read Pointer

pub const RIRBCTL_DMAEN: u8 = 1 << 1;     // Start RIRB DMA engine
pub const RIRBCTL_RINTCTL: u8 = 1 << 0;   // Response Interrupt Control
pub const RIRBWP_RST: u16 = 1 << 15;      // Reset Write Pointer

pub const SD_CTL_SRST: u32 = 1 << 0;      // Stream Reset
pub const SD_CTL_RUN: u32 = 1 << 1;       // Stream Run
pub const SD_CTL_IOCE: u32 = 1 << 2;      // Interrupt On Completion Enable
pub const SD_CTL_FEIE: u32 = 1 << 3;      // FIFO Error Interrupt Enable
pub const SD_CTL_DEIE: u32 = 1 << 4;      // Descriptor Error Interrupt Enable

pub const SD_STS_BCIS: u8 = 1 << 2;       // Buffer Completion Interrupt Status
pub const SD_STS_FIFORDY: u8 = 1 << 5;    // FIFO Ready

/// Error variants for Intel HDA Controller operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdaError {
    NotHdaDevice,
    MissingBar0,
    ControllerResetTimeout,
    CorbFull,
    RirbEmpty,
    InvalidStreamIndex,
    CodecTimeout,
    InvalidBufferSize,
    NoMatchingPath,
    WidgetLimitReached,
}

/// Decoded Global Capability properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdaGlobalCapabilities {
    pub num_oss: u8,       // Number of Output Streams
    pub num_iss: u8,       // Number of Input Streams
    pub num_bss: u8,       // Number of Bidirectional Streams
    pub addr_64bit: bool,  // Supports 64-bit DMA addressing
    pub corb_entries: u16, // Maximum supported CORB entries
    pub rirb_entries: u16, // Maximum supported RIRB entries
}

impl HdaGlobalCapabilities {
    pub fn parse(gcap: u16, corb_size_reg: u8, rirb_size_reg: u8) -> Self {
        let num_oss = ((gcap >> 12) & 0x0F) as u8;
        let num_iss = ((gcap >> 8) & 0x0F) as u8;
        let num_bss = ((gcap >> 3) & 0x1F) as u8;
        let addr_64bit = (gcap & 1) != 0;

        let corb_entries = match corb_size_reg & 0x03 {
            0 => 2,
            1 => 16,
            _ => 256,
        };
        let rirb_entries = match rirb_size_reg & 0x03 {
            0 => 2,
            1 => 16,
            _ => 256,
        };

        Self {
            num_oss,
            num_iss,
            num_bss,
            addr_64bit,
            corb_entries,
            rirb_entries,
        }
    }
}

/// Buffer Descriptor List (BDL) Entry (Specification Section 3.6.3).
/// Must be 16-byte aligned in memory.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdaBufferDescriptorEntry {
    pub address_low: u32,
    pub address_high: u32,
    pub length: u32,
    pub flags: u32, // Bit 0 = Interrupt on Completion (IOC)
}

impl HdaBufferDescriptorEntry {
    pub const IOC_FLAG: u32 = 1 << 0;

    pub fn new(phys_addr: u64, length: u32, ioc: bool) -> Self {
        Self {
            address_low: (phys_addr & 0xFFFF_FFFF) as u32,
            address_high: ((phys_addr >> 32) & 0xFFFF_FFFF) as u32,
            length,
            flags: if ioc { Self::IOC_FLAG } else { 0 },
        }
    }

    pub fn physical_address(&self) -> u64 {
        (self.address_low as u64) | ((self.address_high as u64) << 32)
    }
}

/// Command and Output Ring Buffer (CORB) Queue (256 32-bit entries = 1 KiB).
pub struct CorbBuffer {
    pub entries: [u32; 256],
    pub write_ptr: u8,
    pub read_ptr: u8,
}

impl CorbBuffer {
    pub const fn new() -> Self {
        Self {
            entries: [0u32; 256],
            write_ptr: 0,
            read_ptr: 0,
        }
    }

    pub fn push_verb(&mut self, verb: u32) -> Result<u8, HdaError> {
        let next_wp = self.write_ptr.wrapping_add(1);
        if next_wp == self.read_ptr {
            return Err(HdaError::CorbFull);
        }
        self.entries[self.write_ptr as usize] = verb;
        let wp = self.write_ptr;
        self.write_ptr = next_wp;
        Ok(wp)
    }

    pub fn is_empty(&self) -> bool {
        self.write_ptr == self.read_ptr
    }

    pub fn entry_count(&self) -> usize {
        self.write_ptr.wrapping_sub(self.read_ptr) as usize
    }
}

/// Response Input Ring Buffer (RIRB) Queue (256 64-bit entries = 2 KiB).
pub struct RirbBuffer {
    pub entries: [u64; 256],
    pub write_ptr: u8,
    pub read_ptr: u8,
}

impl RirbBuffer {
    pub const fn new() -> Self {
        Self {
            entries: [0u64; 256],
            write_ptr: 0,
            read_ptr: 0,
        }
    }

    pub fn push_response(&mut self, resp: u64) {
        self.entries[self.write_ptr as usize] = resp;
        self.write_ptr = self.write_ptr.wrapping_add(1);
    }

    pub fn pop_response(&mut self) -> Result<u64, HdaError> {
        if self.read_ptr == self.write_ptr {
            return Err(HdaError::RirbEmpty);
        }
        let resp = self.entries[self.read_ptr as usize];
        self.read_ptr = self.read_ptr.wrapping_add(1);
        Ok(resp)
    }

    pub fn is_empty(&self) -> bool {
        self.read_ptr == self.write_ptr
    }
}

/// PCM Audio Stream Sample Format (Specification Section 3.7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdaStreamFormat {
    pub sample_rate: u32,      // 44100, 48000, 96000
    pub bits_per_sample: u8,   // 8, 16, 20, 24, 32
    pub channels: u8,          // 1 (mono), 2 (stereo), etc.
}

impl HdaStreamFormat {
    pub const fn new_stereo_48khz_16bit() -> Self {
        Self {
            sample_rate: 48000,
            bits_per_sample: 16,
            channels: 2,
        }
    }

    pub const fn new_stereo_44khz_16bit() -> Self {
        Self {
            sample_rate: 44100,
            bits_per_sample: 16,
            channels: 2,
        }
    }

    /// Serializes format parameters into standard 16-bit HDA Stream Format register value.
    pub fn to_format_code(&self) -> u16 {
        let mut code = 0u16;

        // Base rate: 0 = 48 kHz, 1 = 44.1 kHz
        if self.sample_rate % 44100 == 0 {
            code |= 1 << 14;
        }

        // Multiplier & Divisor
        if self.sample_rate == 96000 {
            code |= 1 << 11; // 2x multiplier
        }

        // Bits per sample: 000: 8-bit, 001: 16-bit, 010: 20-bit, 011: 24-bit, 100: 32-bit
        let bits_field = match self.bits_per_sample {
            8 => 0b000,
            16 => 0b001,
            20 => 0b010,
            24 => 0b011,
            32 => 0b100,
            _ => 0b001,
        };
        code |= bits_field << 4;

        // Channels: 0 = 1 channel, 1 = 2 channels, ..., 15 = 16 channels
        code |= ((self.channels.max(1) - 1) & 0x0F) as u16;

        code
    }

    /// Parses format parameters from standard 16-bit HDA format code.
    pub fn from_format_code(code: u16) -> Self {
        let base = if (code & (1 << 14)) != 0 { 44100 } else { 48000 };
        let mult = if (code & (1 << 11)) != 0 { 2 } else { 1 };
        let sample_rate = base * mult;

        let bits_per_sample = match (code >> 4) & 0x07 {
            0b000 => 8,
            0b001 => 16,
            0b010 => 20,
            0b011 => 24,
            0b100 => 32,
            _ => 16,
        };

        let channels = ((code & 0x0F) as u8) + 1;

        Self {
            sample_rate,
            bits_per_sample,
            channels,
        }
    }
}

/// Intel HDA Controller abstraction over physical PCI MMIO window.
pub struct HdaController {
    pub pci_bus: u8,
    pub pci_slot: u8,
    pub pci_func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub mmio_base: u64,
    pub mmio_size: u64,
    pub caps: HdaGlobalCapabilities,
    pub corb: CorbBuffer,
    pub rirb: RirbBuffer,
}

impl HdaController {
    /// Instantiates an HDA controller structure around a given MMIO base window.
    pub fn new(mmio_base: u64, mmio_size: u64) -> Self {
        // Standard virtual capabilities fallback: 4 output streams, 4 input streams, 64-bit addressing
        let caps = HdaGlobalCapabilities {
            num_oss: 4,
            num_iss: 4,
            num_bss: 0,
            addr_64bit: true,
            corb_entries: 256,
            rirb_entries: 256,
        };

        Self {
            pci_bus: 0,
            pci_slot: 0,
            pci_func: 0,
            vendor_id: 0x8086, // Intel
            device_id: 0x2668, // ICH6/Q35 High Definition Audio
            mmio_base,
            mmio_size,
            caps,
            corb: CorbBuffer::new(),
            rirb: RirbBuffer::new(),
        }
    }

    /// Attempts to instantiate an HDA controller from an enumerated PCI device.
    pub fn from_pci(dev: &PciDevice) -> Result<Self, HdaError> {
        if dev.class_code != PCI_CLASS_MULTIMEDIA || dev.subclass != PCI_SUBCLASS_HDA {
            return Err(HdaError::NotHdaDevice);
        }

        let bar0 = dev.get_bar(0).ok_or(HdaError::MissingBar0)?;
        if bar0.address == 0 {
            return Err(HdaError::MissingBar0);
        }

        let mut controller = Self::new(bar0.address, bar0.size);
        controller.pci_bus = dev.bus;
        controller.pci_slot = dev.slot;
        controller.pci_func = dev.function;
        controller.vendor_id = dev.vendor_id;
        controller.device_id = dev.device_id;

        Ok(controller)
    }

    /// Calculates offset of stream descriptor `index`.
    #[inline(always)]
    pub fn stream_offset(&self, index: usize) -> usize {
        HDA_SD_BASE + (index * HDA_SD_STRIDE)
    }

    /// Calculates output stream descriptor offset.
    #[inline(always)]
    pub fn output_stream_offset(&self, output_idx: usize) -> usize {
        let total_idx = (self.caps.num_iss as usize) + output_idx;
        self.stream_offset(total_idx)
    }
}

/// Self-test verifying HDA MMIO register map, BDL alignment, and format code translation.
pub fn verify_hda_structures() -> bool {
    // 1. Verify BDL entry alignment and size
    if core::mem::size_of::<HdaBufferDescriptorEntry>() != 16 {
        return false;
    }

    let bdl = HdaBufferDescriptorEntry::new(0x1_0000_2000, 4096, true);
    if bdl.physical_address() != 0x1_0000_2000 {
        return false;
    }
    if bdl.length != 4096 || (bdl.flags & HdaBufferDescriptorEntry::IOC_FLAG) == 0 {
        return false;
    }

    // 2. Verify Stream Format code encoding and decoding
    let fmt_48k = HdaStreamFormat::new_stereo_48khz_16bit();
    let code_48k = fmt_48k.to_format_code();
    // 48 kHz (base=0, mult=0), 16-bit (0b001 << 4 = 0x10), stereo (channels-1 = 1) -> 0x0011
    if code_48k != 0x0011 {
        return false;
    }
    let decoded_48k = HdaStreamFormat::from_format_code(code_48k);
    if decoded_48k != fmt_48k {
        return false;
    }

    let fmt_44k = HdaStreamFormat::new_stereo_44khz_16bit();
    let code_44k = fmt_44k.to_format_code();
    // 44.1 kHz (base=1 << 14 = 0x4000), 16-bit (0x10), stereo (0x01) -> 0x4011
    if code_44k != 0x4011 {
        return false;
    }
    let decoded_44k = HdaStreamFormat::from_format_code(code_44k);
    if decoded_44k != fmt_44k {
        return false;
    }

    // 3. Verify CORB and RIRB ring buffer operations
    let mut corb = CorbBuffer::new();
    let mut rirb = RirbBuffer::new();

    if corb.push_verb(0x001F_0000).is_err() {
        return false;
    }
    if corb.entry_count() != 1 {
        return false;
    }

    rirb.push_response(0x10EC_0888);
    let resp = match rirb.pop_response() {
        Ok(r) => r,
        Err(_) => return false,
    };
    if resp != 0x10EC_0888 || !rirb.is_empty() {
        return false;
    }

    // 4. Verify controller stream offsets
    let ctrl = HdaController::new(0xFEE0_0000, 16384);
    if ctrl.stream_offset(0) != 0x80 {
        return false;
    }
    // Output stream 0 with 4 input streams -> stream 4 (offset 0x80 + 4*0x20 = 0x100)
    if ctrl.output_stream_offset(0) != 0x100 {
        return false;
    }

    true
}


/// Intel HDA Verb Command Encoding (Specification Section 7.3).
pub struct HdaVerb;

impl HdaVerb {
    pub const GET_PARAMETER: u16 = 0xF00;
    pub const GET_CONNECTION_SELECT: u16 = 0xF01;
    pub const SET_CONNECTION_SELECT: u16 = 0x701;
    pub const GET_CONNECTION_LIST: u16 = 0xF02;
    pub const GET_PROCESSING_STATE: u16 = 0xF03;
    pub const SET_PROCESSING_STATE: u16 = 0x703;
    pub const GET_AMP_GAIN_MUTE: u16 = 0xB;
    pub const SET_AMP_GAIN_MUTE: u16 = 0x3;
    pub const GET_CONVERTER_FORMAT: u16 = 0xA;
    pub const SET_CONVERTER_FORMAT: u16 = 0x2;
    pub const GET_POWER_STATE: u16 = 0xF05;
    pub const SET_POWER_STATE: u16 = 0x705;
    pub const GET_CONVERTER_STREAM_CHANNEL: u16 = 0xF06;
    pub const SET_CONVERTER_STREAM_CHANNEL: u16 = 0x706;
    pub const GET_PIN_WIDGET_CONTROL: u16 = 0xF07;
    pub const SET_PIN_WIDGET_CONTROL: u16 = 0x707;
    pub const GET_UNSOLICITED_RESPONSE: u16 = 0xF08;
    pub const SET_UNSOLICITED_RESPONSE: u16 = 0x708;
    pub const GET_PIN_SENSE: u16 = 0xF09;
    pub const EXECUTE_PIN_SENSE: u16 = 0x709;
    pub const GET_CONFIGURATION_DEFAULT: u16 = 0xF1C;

    // Parameter Identifiers (Section 7.3.4)
    pub const PARAM_VENDOR_ID: u8 = 0x00;
    pub const PARAM_REVISION_ID: u8 = 0x02;
    pub const PARAM_SUB_NODE_COUNT: u8 = 0x04;
    pub const PARAM_FUNCTION_GROUP_TYPE: u8 = 0x05;
    pub const PARAM_AUDIO_GROUP_CAPS: u8 = 0x08;
    pub const PARAM_AUDIO_WIDGET_CAPS: u8 = 0x09;
    pub const PARAM_SAMPLE_SIZE_RATE_CAPS: u8 = 0x0A;
    pub const PARAM_STREAM_FORMATS: u8 = 0x0B;
    pub const PARAM_PIN_CAPABILITIES: u8 = 0x0C;
    pub const PARAM_INPUT_AMP_CAPS: u8 = 0x0D;
    pub const PARAM_CONN_LIST_LEN: u8 = 0x0E;
    pub const PARAM_POWER_STATES: u8 = 0x0F;
    pub const PARAM_PROCESSING_CAPS: u8 = 0x10;
    pub const PARAM_OUTPUT_AMP_CAPS: u8 = 0x12;

    // Pin Widget Control Flags (Section 7.3.3.13)
    pub const PIN_CTL_HEADPHONE_ENABLE: u8 = 1 << 7;
    pub const PIN_CTL_OUTPUT_ENABLE: u8 = 1 << 6;
    pub const PIN_CTL_INPUT_ENABLE: u8 = 1 << 5;
    pub const PIN_CTL_VREF_HIZ: u8 = 0x00;
    pub const PIN_CTL_VREF_50: u8 = 0x01;
    pub const PIN_CTL_VREF_GROUND: u8 = 0x02;
    pub const PIN_CTL_VREF_80: u8 = 0x04;
    pub const PIN_CTL_VREF_100: u8 = 0x05;

    // Power Management States (Section 7.3.3.10)
    pub const POWER_STATE_D0: u8 = 0;
    pub const POWER_STATE_D1: u8 = 1;
    pub const POWER_STATE_D2: u8 = 2;
    pub const POWER_STATE_D3: u8 = 3;

    /// Builds standard 12-bit verb with 8-bit parameter payload.
    #[inline(always)]
    pub fn build_12bit(cad: u8, nid: u8, verb: u16, payload: u8) -> u32 {
        ((cad as u32 & 0x0F) << 28)
            | ((nid as u32 & 0xFF) << 20)
            | ((verb as u32 & 0xFFF) << 8)
            | (payload as u32 & 0xFF)
    }

    /// Builds 4-bit verb with 16-bit parameter payload.
    #[inline(always)]
    pub fn build_4bit(cad: u8, nid: u8, verb: u8, payload: u16) -> u32 {
        ((cad as u32 & 0x0F) << 28)
            | ((nid as u32 & 0xFF) << 20)
            | ((verb as u32 & 0x0F) << 16)
            | (payload as u32 & 0xFFFF)
    }

    /// Builds Get Parameter verb.
    pub fn get_parameter(cad: u8, nid: u8, param_id: u8) -> u32 {
        Self::build_12bit(cad, nid, Self::GET_PARAMETER, param_id)
    }

    /// Builds Set Converter Stream and Channel verb.
    pub fn set_stream_channel(cad: u8, nid: u8, stream_id: u8, channel: u8) -> u32 {
        let payload = ((stream_id & 0x0F) << 4) | (channel & 0x0F);
        Self::build_12bit(cad, nid, Self::SET_CONVERTER_STREAM_CHANNEL, payload)
    }

    /// Builds Set Converter Format verb.
    pub fn set_converter_format(cad: u8, nid: u8, format_code: u16) -> u32 {
        Self::build_4bit(cad, nid, Self::SET_CONVERTER_FORMAT as u8, format_code)
    }

    /// Builds Set Pin Widget Control verb.
    pub fn set_pin_widget_control(cad: u8, nid: u8, control: u8) -> u32 {
        Self::build_12bit(cad, nid, Self::SET_PIN_WIDGET_CONTROL, control)
    }

    /// Builds Set Power State verb.
    pub fn set_power_state(cad: u8, nid: u8, state: u8) -> u32 {
        Self::build_12bit(cad, nid, Self::SET_POWER_STATE, state & 0x03)
    }

    /// Builds Set Amplifier Gain / Mute verb.
    pub fn set_amp_gain_mute(
        cad: u8,
        nid: u8,
        is_input: bool,
        left: bool,
        right: bool,
        index: u8,
        gain: u8,
        mute: bool,
    ) -> u32 {
        let mut payload = (gain & 0x7F) as u16;
        if mute {
            payload |= 1 << 7;
        }
        if index != 0 {
            payload |= ((index & 0x0F) as u16) << 8;
        }
        if right {
            payload |= 1 << 12;
        }
        if left {
            payload |= 1 << 13;
        }
        if is_input {
            payload |= 1 << 14;
        } else {
            payload |= 1 << 15;
        }
        Self::build_4bit(cad, nid, Self::SET_AMP_GAIN_MUTE as u8, payload)
    }
}

/// Intel HDA Widget Types (Specification Section 7.3.4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdaWidgetType {
    AudioOutput,     // DAC (0x0)
    AudioInput,      // ADC (0x1)
    AudioMixer,      // Mixer (0x2)
    AudioSelector,   // Selector (0x3)
    PinComplex,      // Pin Complex (0x4)
    PowerWidget,     // Power Widget (0x5)
    VolumeKnob,      // Volume Knob (0x6)
    BeepGenerator,   // Beep Generator (0x7)
    VendorDefined,   // Vendor Defined (0xF)
    Unknown(u8),
}

impl HdaWidgetType {
    pub fn from_raw(val: u8) -> Self {
        match val {
            0x0 => Self::AudioOutput,
            0x1 => Self::AudioInput,
            0x2 => Self::AudioMixer,
            0x3 => Self::AudioSelector,
            0x4 => Self::PinComplex,
            0x5 => Self::PowerWidget,
            0x6 => Self::VolumeKnob,
            0x7 => Self::BeepGenerator,
            0xF => Self::VendorDefined,
            other => Self::Unknown(other),
        }
    }
}

/// Codec Node Widget descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdaWidget {
    pub nid: u8,
    pub widget_type: HdaWidgetType,
    pub capabilities: u32,
    pub connections: [u8; 8],
    pub connection_count: u8,
}

impl HdaWidget {
    pub const fn new(nid: u8, widget_type: HdaWidgetType, capabilities: u32) -> Self {
        Self {
            nid,
            widget_type,
            capabilities,
            connections: [0u8; 8],
            connection_count: 0,
        }
    }

    pub fn add_connection(&mut self, source_nid: u8) -> bool {
        if (self.connection_count as usize) < self.connections.len() {
            self.connections[self.connection_count as usize] = source_nid;
            self.connection_count += 1;
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn is_output_dac(&self) -> bool {
        self.widget_type == HdaWidgetType::AudioOutput
    }

    #[inline(always)]
    pub fn is_pin_complex(&self) -> bool {
        self.widget_type == HdaWidgetType::PinComplex
    }

    pub fn has_connection(&self, source_nid: u8) -> bool {
        for i in 0..(self.connection_count as usize) {
            if self.connections[i] == source_nid {
                return true;
            }
        }
        false
    }
}

pub const MAX_CODEC_WIDGETS: usize = 32;

/// Intel HDA Codec representation with discovered widgets and audio paths.
#[derive(Debug, Clone)]
pub struct HdaCodec {
    pub cad: u8,
    pub vendor_id: u32,
    pub revision_id: u32,
    pub afg_nid: u8,
    pub widgets: [Option<HdaWidget>; MAX_CODEC_WIDGETS],
    pub widget_count: usize,
    pub active_dac_nid: Option<u8>,
    pub active_pin_nid: Option<u8>,
}

impl HdaCodec {
    pub const fn new(cad: u8) -> Self {
        const NONE_WIDGET: Option<HdaWidget> = None;
        Self {
            cad,
            vendor_id: 0,
            revision_id: 0,
            afg_nid: 1,
            widgets: [NONE_WIDGET; MAX_CODEC_WIDGETS],
            widget_count: 0,
            active_dac_nid: None,
            active_pin_nid: None,
        }
    }

    pub fn add_widget(&mut self, widget: HdaWidget) -> bool {
        if self.widget_count < MAX_CODEC_WIDGETS {
            self.widgets[self.widget_count] = Some(widget);
            self.widget_count += 1;
            true
        } else {
            false
        }
    }

    /// Traverses widget hierarchy to resolve an active (DAC Node, Pin Node) audio output path.
    pub fn find_audio_output_path(&self) -> Option<(u8, u8)> {
        for i in 0..self.widget_count {
            if let Some(pin) = &self.widgets[i] {
                if pin.is_pin_complex() {
                    for j in 0..self.widget_count {
                        if let Some(dac) = &self.widgets[j] {
                            if dac.is_output_dac() && pin.has_connection(dac.nid) {
                                return Some((dac.nid, pin.nid));
                            }
                        }
                    }
                }
            }
        }

        let mut dac_candidate = None;
        let mut pin_candidate = None;

        for i in 0..self.widget_count {
            if let Some(widget) = &self.widgets[i] {
                if widget.is_output_dac() && dac_candidate.is_none() {
                    dac_candidate = Some(widget.nid);
                } else if widget.is_pin_complex() && pin_candidate.is_none() {
                    pin_candidate = Some(widget.nid);
                }
            }
        }

        match (dac_candidate, pin_candidate) {
            (Some(dac), Some(pin)) => Some((dac, pin)),
            _ => None,
        }
    }

    /// Configures the resolved playback path through CORB verbs.
    pub fn configure_playback_path(
        &mut self,
        stream_id: u8,
        format_code: u16,
        corb: &mut CorbBuffer,
    ) -> Result<(), HdaError> {
        let (dac_nid, pin_nid) = self.find_audio_output_path().ok_or(HdaError::NoMatchingPath)?;
        self.active_dac_nid = Some(dac_nid);
        self.active_pin_nid = Some(pin_nid);

        // 1. Power on Audio Function Group
        corb.push_verb(HdaVerb::set_power_state(self.cad, self.afg_nid, HdaVerb::POWER_STATE_D0))?;

        // 2. Power on DAC converter widget
        corb.push_verb(HdaVerb::set_power_state(self.cad, dac_nid, HdaVerb::POWER_STATE_D0))?;

        // 3. Assign stream and channel to DAC
        corb.push_verb(HdaVerb::set_stream_channel(self.cad, dac_nid, stream_id, 0))?;

        // 4. Set stream format on DAC
        corb.push_verb(HdaVerb::set_converter_format(self.cad, dac_nid, format_code))?;

        // 5. Un-mute DAC output amplifier (0 dB / unity gain)
        corb.push_verb(HdaVerb::set_amp_gain_mute(self.cad, dac_nid, false, true, true, 0, 0x7F, false))?;

        // 6. Power on Pin Complex widget
        corb.push_verb(HdaVerb::set_power_state(self.cad, pin_nid, HdaVerb::POWER_STATE_D0))?;

        // 7. Enable Pin output and headphone drive
        corb.push_verb(HdaVerb::set_pin_widget_control(
            self.cad,
            pin_nid,
            HdaVerb::PIN_CTL_OUTPUT_ENABLE | HdaVerb::PIN_CTL_HEADPHONE_ENABLE,
        ))?;

        // 8. Un-mute Pin output amplifier
        corb.push_verb(HdaVerb::set_amp_gain_mute(self.cad, pin_nid, false, true, true, 0, 0x7F, false))?;

        Ok(())
    }
}

pub const MAX_BDL_ENTRIES: usize = 16;

/// Cyclic DMA Stream Engine managing Buffer Descriptor List (BDL) and Link Position.
#[derive(Debug, Clone)]
pub struct HdaStreamDma {
    pub stream_index: usize,
    pub stream_id: u8,
    pub is_output: bool,
    pub format: HdaStreamFormat,
    pub buffer_phys_addr: u64,
    pub buffer_size: usize,
    pub period_size: usize,
    pub bdl_entries: [HdaBufferDescriptorEntry; MAX_BDL_ENTRIES],
    pub bdl_entry_count: usize,
    pub running: bool,
    pub lpib: u32,
}

impl HdaStreamDma {
    pub fn new_output(
        stream_index: usize,
        stream_id: u8,
        format: HdaStreamFormat,
        buffer_phys_addr: u64,
        buffer_size: usize,
        period_size: usize,
    ) -> Result<Self, HdaError> {
        if period_size == 0 || buffer_size < period_size || buffer_size % period_size != 0 {
            return Err(HdaError::InvalidBufferSize);
        }
        let num_periods = buffer_size / period_size;
        if num_periods > MAX_BDL_ENTRIES {
            return Err(HdaError::InvalidBufferSize);
        }

        let mut bdl_entries = [HdaBufferDescriptorEntry::new(0, 0, false); MAX_BDL_ENTRIES];
        for i in 0..num_periods {
            let period_addr = buffer_phys_addr + (i * period_size) as u64;
            bdl_entries[i] = HdaBufferDescriptorEntry::new(period_addr, period_size as u32, true);
        }

        Ok(Self {
            stream_index,
            stream_id,
            is_output: true,
            format,
            buffer_phys_addr,
            buffer_size,
            period_size,
            bdl_entries,
            bdl_entry_count: num_periods,
            running: false,
            lpib: 0,
        })
    }

    #[inline(always)]
    pub fn start(&mut self) {
        self.running = true;
    }

    #[inline(always)]
    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn update_lpib(&mut self, bytes_advanced: u32) -> u32 {
        if !self.running || self.buffer_size == 0 {
            return self.lpib;
        }
        self.lpib = (self.lpib + bytes_advanced) % (self.buffer_size as u32);
        self.lpib
    }

    #[inline(always)]
    pub fn lpib(&self) -> u32 {
        self.lpib
    }

    #[inline(always)]
    pub fn current_period(&self) -> usize {
        if self.period_size == 0 {
            0
        } else {
            (self.lpib as usize) / self.period_size
        }
    }

    #[inline(always)]
    pub fn is_running(&self) -> bool {
        self.running
    }
}

/// Self-test verifying HDA codec discovery, widget routing, verb dispatch, and stream DMA engine.
pub fn verify_hda_codec_and_stream() -> bool {
    // 1. Verify Verb encoding
    let get_vendor = HdaVerb::get_parameter(0, 0, HdaVerb::PARAM_VENDOR_ID);
    if get_vendor != 0x000F_0000 {
        return false;
    }

    let set_sc = HdaVerb::set_stream_channel(0, 2, 1, 0);
    if set_sc != 0x0027_0610 {
        return false;
    }

    let set_fmt = HdaVerb::set_converter_format(0, 2, 0x0011);
    if set_fmt != 0x0022_0011 {
        return false;
    }

    let set_pin = HdaVerb::set_pin_widget_control(
        0,
        3,
        HdaVerb::PIN_CTL_OUTPUT_ENABLE | HdaVerb::PIN_CTL_HEADPHONE_ENABLE,
    );
    if set_pin != 0x0037_07C0 {
        return false;
    }

    // 2. Verify Codec discovery and widget path routing
    let mut codec = HdaCodec::new(0);
    codec.vendor_id = 0x10EC_0888;
    codec.afg_nid = 1;

    let dac_widget = HdaWidget::new(2, HdaWidgetType::AudioOutput, 0x0000_0000);
    let mut pin_widget = HdaWidget::new(3, HdaWidgetType::PinComplex, 0x0001_0000);
    pin_widget.add_connection(2);

    if !codec.add_widget(dac_widget) || !codec.add_widget(pin_widget) {
        return false;
    }

    let path = match codec.find_audio_output_path() {
        Some(p) => p,
        None => return false,
    };
    if path != (2, 3) {
        return false;
    }

    // Configure playback path and verify 8 verbs queued into CORB
    let mut corb = CorbBuffer::new();
    if codec.configure_playback_path(1, 0x0011, &mut corb).is_err() {
        return false;
    }
    if corb.entry_count() != 8 {
        return false;
    }

    // 3. Verify Stream DMA Engine & cyclic buffer progression
    let format = HdaStreamFormat::new_stereo_48khz_16bit();
    let mut dma = match HdaStreamDma::new_output(0, 1, format, 0x2000_0000, 8192, 4096) {
        Ok(d) => d,
        Err(_) => return false,
    };

    if dma.bdl_entry_count != 2 {
        return false;
    }
    if dma.bdl_entries[0].physical_address() != 0x2000_0000 || dma.bdl_entries[0].length != 4096 {
        return false;
    }
    if dma.bdl_entries[1].physical_address() != 0x2000_1000 || dma.bdl_entries[1].length != 4096 {
        return false;
    }

    dma.start();
    if !dma.is_running() || dma.current_period() != 0 {
        return false;
    }

    // Advance 4096 bytes -> period 1
    dma.update_lpib(4096);
    if dma.lpib() != 4096 || dma.current_period() != 1 {
        return false;
    }

    // Advance 4096 bytes -> wraps to 0
    dma.update_lpib(4096);
    if dma.lpib() != 0 || dma.current_period() != 0 {
        return false;
    }

    dma.stop();
    if dma.is_running() {
        return false;
    }

    true
}
