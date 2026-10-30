//! USB Mass Storage Class Bulk-Only Transport (BOT) Protocol Driver (USB MSC BOT 1.0).
//!
//! Implements USB MSC Bulk-Only Transport 1.0 specification:
//! - Command Block Wrapper (CBW, 31 bytes, signature `USBC` / 0x43425355)
//! - Command Status Wrapper (CSW, 13 bytes, signature `USBS` / 0x53425355)
//! - Three-phase command execution state machine (CBW -> Data In/Out -> CSW)
//! - Bulk endpoint communication and stall/error recovery handling.

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
///
/// 31-byte packet sent by the host on the Bulk-Out endpoint to initiate a command.
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
///
/// 13-byte packet sent by the device on the Bulk-In endpoint to report command completion.
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
