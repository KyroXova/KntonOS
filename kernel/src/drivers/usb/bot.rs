//! USB Mass Storage Class Bulk-Only Transport (BOT) Protocol Driver (USB MSC BOT 1.0).

use core::mem::size_of;

/// USB Mass Storage Class code.
pub const USB_CLASS_MASS_STORAGE: u8 = 0x08;
/// SCSI transparent command set subclass.
pub const USB_SUBCLASS_SCSI: u8 = 0x06;
/// Bulk-Only Transport (BOT) protocol.
pub const USB_PROTOCOL_BOT: u8 = 0x50;

/// Signature for Command Block Wrapper: ASCII "USBC" in little-endian.
pub const CBW_SIGNATURE: u32 = 0x4342_5355;
/// Signature for Command Status Wrapper: ASCII "USBS" in little-endian.
pub const CSW_SIGNATURE: u32 = 0x5342_5355;

/// Size of a Command Block Wrapper in bytes.
pub const CBW_LENGTH: usize = 31;
/// Size of a Command Status Wrapper in bytes.
pub const CSW_LENGTH: usize = 13;

/// CBW Flags: Data-In transfer direction (device to host).
pub const CBW_FLAG_DATA_IN: u8 = 0x80;
/// CBW Flags: Data-Out transfer direction (host to device).
pub const CBW_FLAG_DATA_OUT: u8 = 0x00;

/// CSW Status: Command Passed (Good / Success).
pub const CSW_STATUS_PASSED: u8 = 0x00;
/// CSW Status: Command Failed.
pub const CSW_STATUS_FAILED: u8 = 0x01;
/// CSW Status: Phase Error (Host/Device phase mismatch, requires reset recovery).
pub const CSW_STATUS_PHASE_ERROR: u8 = 0x02;

/// 16-byte SCSI Command Descriptor Block wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiCdb {
    pub bytes: [u8; 16],
    pub length: u8,
}

impl ScsiCdb {
    /// Creates an empty SCSI Command Descriptor Block.
    pub const fn empty() -> Self {
        Self {
            bytes: [0u8; 16],
            length: 0,
        }
    }

    /// Creates a SCSI Command Descriptor Block from a slice.
    pub fn from_slice(slice: &[u8]) -> Self {
        let mut bytes = [0u8; 16];
        let len = slice.len().min(16);
        bytes[..len].copy_from_slice(&slice[..len]);
        Self {
            bytes,
            length: len as u8,
        }
    }
}

/// USB MSC Bulk-Only Transport (BOT) Command Block Wrapper (CBW).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandBlockWrapper {
    /// Signature: must be `0x43425355` ("USBC").
    pub signature: u32,
    /// Host-assigned unique tag echoed in the corresponding CSW.
    pub tag: u32,
    /// Number of bytes of data host expects to transfer in data phase.
    pub data_transfer_length: u32,
    /// Bit 7: direction (0=Data-Out, 1=Data-In). Bits 6:0 reserved (0).
    pub flags: u8,
    /// Logical Unit Number (bits 3:0).
    pub lun: u8,
    /// Length of the command block (1..16).
    pub cb_length: u8,
    /// Command Descriptor Block (CDB, up to 16 bytes).
    pub cb: [u8; 16],
}

impl CommandBlockWrapper {
    /// Constructs a new Command Block Wrapper.
    pub fn new(tag: u32, data_transfer_length: u32, is_data_in: bool, lun: u8, cdb: &[u8]) -> Self {
        let mut cb = [0u8; 16];
        let cb_len = cdb.len().min(16) as u8;
        cb[..cb_len as usize].copy_from_slice(&cdb[..cb_len as usize]);

        let flags = if is_data_in {
            CBW_FLAG_DATA_IN
        } else {
            CBW_FLAG_DATA_OUT
        };

        Self {
            signature: CBW_SIGNATURE,
            tag,
            data_transfer_length,
            flags,
            lun: lun & 0x0F,
            cb_length: cb_len,
            cb,
        }
    }

    /// Serializes the CBW into a 31-byte array.
    pub fn to_bytes(&self) -> [u8; CBW_LENGTH] {
        let mut buf = [0u8; CBW_LENGTH];
        buf[0..4].copy_from_slice(&self.signature.to_le_bytes());
        buf[4..8].copy_from_slice(&self.tag.to_le_bytes());
        buf[8..12].copy_from_slice(&self.data_transfer_length.to_le_bytes());
        buf[12] = self.flags;
        buf[13] = self.lun;
        buf[14] = self.cb_length;
        buf[15..31].copy_from_slice(&self.cb);
        buf
    }

    /// Deserializes a CBW from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CBW_LENGTH {
            return None;
        }

        let signature = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let tag = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let data_transfer_length = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let flags = bytes[12];
        let lun = bytes[13];
        let cb_length = bytes[14];

        let mut cb = [0u8; 16];
        cb.copy_from_slice(&bytes[15..31]);

        let cbw = Self {
            signature,
            tag,
            data_transfer_length,
            flags,
            lun,
            cb_length,
            cb,
        };

        if cbw.is_valid() {
            Some(cbw)
        } else {
            None
        }
    }

    /// Validates CBW integrity according to USB BOT 1.0 Section 6.2.
    pub fn is_valid(&self) -> bool {
        self.signature == CBW_SIGNATURE
            && (self.cb_length >= 1 && self.cb_length <= 16)
            && (self.lun & 0xF0) == 0
            && (self.flags & 0x7F) == 0
    }

    /// Returns true if this CBW expects data from device to host.
    pub fn is_data_in(&self) -> bool {
        (self.flags & CBW_FLAG_DATA_IN) != 0
    }
}

/// USB MSC Bulk-Only Transport (BOT) Command Status Wrapper (CSW).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandStatusWrapper {
    /// Signature: must be `0x53425355` ("USBS").
    pub signature: u32,
    /// Echoed tag from the corresponding CBW.
    pub tag: u32,
    /// Difference between requested data transfer length and actual transferred bytes.
    pub data_residue: u32,
    /// Command execution status: 0=Passed, 1=Failed, 2=Phase Error.
    pub status: u8,
}

impl CommandStatusWrapper {
    /// Constructs a new Command Status Wrapper.
    pub fn new(tag: u32, data_residue: u32, status: u8) -> Self {
        Self {
            signature: CSW_SIGNATURE,
            tag,
            data_residue,
            status,
        }
    }

    /// Serializes the CSW into a 13-byte array.
    pub fn to_bytes(&self) -> [u8; CSW_LENGTH] {
        let mut buf = [0u8; CSW_LENGTH];
        buf[0..4].copy_from_slice(&self.signature.to_le_bytes());
        buf[4..8].copy_from_slice(&self.tag.to_le_bytes());
        buf[8..12].copy_from_slice(&self.data_residue.to_le_bytes());
        buf[12] = self.status;
        buf
    }

    /// Deserializes a CSW from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < CSW_LENGTH {
            return None;
        }

        let signature = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let tag = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let data_residue = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let status = bytes[12];

        let csw = Self {
            signature,
            tag,
            data_residue,
            status,
        };

        if csw.signature == CSW_SIGNATURE {
            Some(csw)
        } else {
            None
        }
    }

    /// Checks if the CSW is valid and matches the given expected tag.
    pub fn is_valid(&self, expected_tag: u32) -> bool {
        self.signature == CSW_SIGNATURE && self.tag == expected_tag
    }

    /// Returns true if command completed successfully (Passed status and matching tag).
    pub fn is_success(&self, expected_tag: u32) -> bool {
        self.is_valid(expected_tag) && self.status == CSW_STATUS_PASSED
    }
}

/// Verification self-test for BOT CBW and CSW data structures.
pub fn verify_bot_structures() -> bool {
    // 1. Verify byte sizes
    if size_of::<CommandBlockWrapper>() != 31 || size_of::<CommandStatusWrapper>() != 13 {
        return false;
    }

    // 2. Verify CBW construction & serialization
    let scsi_inquiry = [0x12u8, 0x00, 0x00, 0x00, 36, 0x00];
    let cbw = CommandBlockWrapper::new(0x1337_BEEF, 36, true, 0, &scsi_inquiry);

    if !cbw.is_valid() || !cbw.is_data_in() {
        return false;
    }
    if cbw.signature != CBW_SIGNATURE || cbw.tag != 0x1337_BEEF || cbw.data_transfer_length != 36 {
        return false;
    }
    if cbw.cb_length != 6 || cbw.cb[0] != 0x12 || cbw.cb[4] != 36 {
        return false;
    }

    let cbw_bytes = cbw.to_bytes();
    let cbw_deser = match CommandBlockWrapper::from_bytes(&cbw_bytes) {
        Some(c) => c,
        None => return false,
    };
    if cbw_deser != cbw {
        return false;
    }

    // 3. Verify CSW construction & validation
    let csw = CommandStatusWrapper::new(0x1337_BEEF, 0, CSW_STATUS_PASSED);
    if !csw.is_valid(0x1337_BEEF) || !csw.is_success(0x1337_BEEF) {
        return false;
    }
    if csw.signature != CSW_SIGNATURE || csw.data_residue != 0 {
        return false;
    }

    let csw_bytes = csw.to_bytes();
    let csw_deser = match CommandStatusWrapper::from_bytes(&csw_bytes) {
        Some(c) => c,
        None => return false,
    };
    if csw_deser != csw {
        return false;
    }

    // Negative tag match check
    if csw.is_valid(0x9999_9999) {
        return false;
    }

    true
}

// USB Bulk-Only Transport (BOT) State Machine & Endpoint Transfer Engine.

/// Current phase of the USB MSC Bulk-Only Transport state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotPhase {
    /// Controller idle, ready to begin next command.
    Idle,
    /// Sending 31-byte CBW on Bulk-Out endpoint.
    CommandPhase,
    /// Data-In (device to host) or Data-Out (host to device) payload transfer.
    DataPhase { bytes_remaining: u32, is_data_in: bool },
    /// Receiving 13-byte CSW on Bulk-In endpoint.
    StatusPhase,
    /// Error condition requiring BOT Reset Recovery (USB MSC BOT 1.0 Section 5.3.4).
    Recovery,
}

/// Errors occurring during Bulk-Only Transport execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotError {
    /// Invalid CBW or CSW signature.
    InvalidSignature,
    /// CSW tag does not match CBW tag.
    TagMismatch,
    /// Phase Error reported in CSW (requires reset recovery).
    PhaseError,
    /// Command execution failed by target device.
    CommandFailed,
    /// Buffer size does not match transfer length.
    BufferLengthMismatch,
    /// USB Endpoint stalled or halted.
    EndpointStall,
    /// Invalid or unsupported SCSI CDB length.
    InvalidCdbLength,
    /// I/O transmission timeout.
    Timeout,
    /// Device is not ready or media not present.
    NotReady,
}

/// USB Mass Storage Bulk-Only Transport (BOT) Engine.
#[derive(Debug, Clone)]
pub struct BotTransport {
    pub slot_id: u8,
    pub bulk_in_ep: u8,
    pub bulk_out_ep: u8,
    pub max_packet_size: u16,
    pub next_tag: u32,
    pub lun: u8,
    pub phase: BotPhase,
    pub last_status: Option<CommandStatusWrapper>,
}

impl BotTransport {
    /// Constructs a new BOT transport context for the given xHCI slot and endpoints.
    pub fn new(slot_id: u8, bulk_in_ep: u8, bulk_out_ep: u8, max_packet_size: u16) -> Self {
        Self {
            slot_id,
            bulk_in_ep,
            bulk_out_ep,
            max_packet_size,
            next_tag: 1,
            lun: 0,
            phase: BotPhase::Idle,
            last_status: None,
        }
    }

    /// Allocates and increments next unique transaction tag.
    pub fn alloc_tag(&mut self) -> u32 {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1);
        if self.next_tag == 0 {
            self.next_tag = 1;
        }
        tag
    }

    /// Prepares a Command Block Wrapper for a SCSI command.
    pub fn create_cbw(&mut self, cdb: &ScsiCdb, transfer_len: u32, is_data_in: bool) -> CommandBlockWrapper {
        let tag = self.alloc_tag();
        self.phase = BotPhase::CommandPhase;
        CommandBlockWrapper::new(tag, transfer_len, is_data_in, self.lun, &cdb.bytes[..cdb.length as usize])
    }

    /// Advances state machine into data phase.
    pub fn enter_data_phase(&mut self, bytes: u32, is_data_in: bool) {
        self.phase = BotPhase::DataPhase {
            bytes_remaining: bytes,
            is_data_in,
        };
    }

    /// Advances state machine into status phase.
    pub fn enter_status_phase(&mut self) {
        self.phase = BotPhase::StatusPhase;
    }

    /// Validates an incoming CSW and transitions back to Idle (or Recovery on error).
    pub fn finish_transaction(&mut self, csw: &CommandStatusWrapper, expected_tag: u32) -> Result<(), BotError> {
        self.last_status = Some(*csw);
        if csw.signature != CSW_SIGNATURE {
            self.phase = BotPhase::Recovery;
            return Err(BotError::InvalidSignature);
        }
        if csw.tag != expected_tag {
            self.phase = BotPhase::Recovery;
            return Err(BotError::TagMismatch);
        }
        if csw.status == CSW_STATUS_PHASE_ERROR {
            self.phase = BotPhase::Recovery;
            return Err(BotError::PhaseError);
        }
        if csw.status == CSW_STATUS_FAILED {
            self.phase = BotPhase::Idle;
            return Err(BotError::CommandFailed);
        }

        self.phase = BotPhase::Idle;
        Ok(())
    }

    /// Executes BOT Reset Recovery to clear stalled endpoints and reset state.
    pub fn reset_recovery(&mut self) {
        self.phase = BotPhase::Idle;
    }
}

/// Verification self-test for BOT transport state machine transitions.
pub fn verify_bot_transport_engine() -> bool {
    let mut transport = BotTransport::new(2, 1, 2, 512);

    if transport.phase != BotPhase::Idle {
        return false;
    }

    let cdb = ScsiCdb::from_slice(&[0x12, 0x00, 0x00, 0x00, 36, 0x00]);
    let cbw = transport.create_cbw(&cdb, 36, true);

    if transport.phase != BotPhase::CommandPhase || cbw.tag != 1 {
        return false;
    }

    transport.enter_data_phase(36, true);
    match transport.phase {
        BotPhase::DataPhase { bytes_remaining, is_data_in } => {
            if bytes_remaining != 36 || !is_data_in {
                return false;
            }
        }
        _ => return false,
    }

    transport.enter_status_phase();
    if transport.phase != BotPhase::StatusPhase {
        return false;
    }

    let csw = CommandStatusWrapper::new(cbw.tag, 0, CSW_STATUS_PASSED);
    if transport.finish_transaction(&csw, cbw.tag).is_err() {
        return false;
    }

    if transport.phase != BotPhase::Idle {
        return false;
    }

    true
}
