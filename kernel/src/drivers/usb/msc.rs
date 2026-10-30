//! USB Mass Storage Class (MSC) & SCSI Primary / Block Command Subsystem.
//!
//! Implements SCSI Primary Commands (SPC) and SCSI Block Commands (SBC) for
//! USB MSC Bulk-Only Transport devices:
//! - TEST UNIT READY (0x00)
//! - REQUEST SENSE (0x03)
//! - INQUIRY (0x12)
//! - READ CAPACITY 10 (0x25)
//! - READ 10 (0x28) / WRITE 10 (0x2A)

use alloc::string::String;
use crate::drivers::usb::bot::{
    BotError, BotTransport, CommandBlockWrapper, CommandStatusWrapper, ScsiCdb,
    CSW_STATUS_PASSED,
};

// --- SCSI Command Operation Codes ---
pub const SCSI_CMD_TEST_UNIT_READY: u8 = 0x00;
pub const SCSI_CMD_REQUEST_SENSE: u8 = 0x03;
pub const SCSI_CMD_INQUIRY: u8 = 0x12;
pub const SCSI_CMD_MODE_SENSE_6: u8 = 0x1A;
pub const SCSI_CMD_START_STOP_UNIT: u8 = 0x1B;
pub const SCSI_CMD_READ_CAPACITY_10: u8 = 0x25;
pub const SCSI_CMD_READ_10: u8 = 0x28;
pub const SCSI_CMD_WRITE_10: u8 = 0x2A;

// --- SCSI Peripheral Device Types (SPC-3) ---
pub const SCSI_DEV_TYPE_DIRECT_ACCESS: u8 = 0x00; // Block device (e.g., Flash drive, HDD)
pub const SCSI_DEV_TYPE_CD_DVD: u8 = 0x05;         // CD/DVD-ROM (MMC)
pub const SCSI_DEV_TYPE_OPTICAL_MEMORY: u8 = 0x07; // Optical memory
pub const SCSI_DEV_TYPE_UNKNOWN: u8 = 0x1F;

// --- Standard Allocation Lengths ---
pub const SCSI_INQUIRY_ALLOCATION_LEN: u32 = 36;
pub const SCSI_READ_CAPACITY_10_LEN: u32 = 8;
pub const SCSI_REQUEST_SENSE_LEN: u32 = 18;

/// SCSI Command Builder utility.
pub struct ScsiCommand;

impl ScsiCommand {
    /// Builds a 6-byte SCSI TEST UNIT READY Command Descriptor Block.
    pub fn test_unit_ready() -> ScsiCdb {
        ScsiCdb::from_slice(&[SCSI_CMD_TEST_UNIT_READY, 0x00, 0x00, 0x00, 0x00, 0x00])
    }

    /// Builds a 6-byte SCSI REQUEST SENSE Command Descriptor Block.
    pub fn request_sense(alloc_length: u8) -> ScsiCdb {
        ScsiCdb::from_slice(&[SCSI_CMD_REQUEST_SENSE, 0x00, 0x00, 0x00, alloc_length, 0x00])
    }

    /// Builds a 6-byte SCSI INQUIRY Command Descriptor Block.
    pub fn inquiry(alloc_length: u8) -> ScsiCdb {
        ScsiCdb::from_slice(&[SCSI_CMD_INQUIRY, 0x00, 0x00, 0x00, alloc_length, 0x00])
    }

    /// Builds a 10-byte SCSI READ CAPACITY (10) Command Descriptor Block.
    pub fn read_capacity_10() -> ScsiCdb {
        ScsiCdb::from_slice(&[
            SCSI_CMD_READ_CAPACITY_10,
            0x00,
            0x00, 0x00, 0x00, 0x00, // LBA (0 = query entire medium)
            0x00, 0x00,
            0x00,                   // PMI bit (0)
            0x00,                   // Control
        ])
    }

    /// Builds a 10-byte SCSI READ (10) Command Descriptor Block.
    pub fn read_10(lba: u32, block_count: u16) -> ScsiCdb {
        let lba_bytes = lba.to_be_bytes();
        let cnt_bytes = block_count.to_be_bytes();
        ScsiCdb::from_slice(&[
            SCSI_CMD_READ_10,
            0x00,                   // Flags / RDPROTECT
            lba_bytes[0], lba_bytes[1], lba_bytes[2], lba_bytes[3],
            0x00,                   // Group number
            cnt_bytes[0], cnt_bytes[1],
            0x00,                   // Control
        ])
    }

    /// Builds a 10-byte SCSI WRITE (10) Command Descriptor Block.
    pub fn write_10(lba: u32, block_count: u16) -> ScsiCdb {
        let lba_bytes = lba.to_be_bytes();
        let cnt_bytes = block_count.to_be_bytes();
        ScsiCdb::from_slice(&[
            SCSI_CMD_WRITE_10,
            0x00,                   // Flags / WRPROTECT
            lba_bytes[0], lba_bytes[1], lba_bytes[2], lba_bytes[3],
            0x00,                   // Group number
            cnt_bytes[0], cnt_bytes[1],
            0x00,                   // Control
        ])
    }
}

/// Parsed Standard SCSI INQUIRY Response Data (36 bytes minimum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScsiInquiryData {
    pub peripheral_device_type: u8,
    pub is_removable: bool,
    pub version: u8,
    pub response_data_format: u8,
    pub vendor_id: [u8; 8],
    pub product_id: [u8; 16],
    pub product_rev: [u8; 4],
}

impl ScsiInquiryData {
    /// Parses standard 36-byte SCSI INQUIRY response payload.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 36 {
            return None;
        }

        let peripheral_device_type = buf[0] & 0x1F;
        let is_removable = (buf[1] & 0x80) != 0;
        let version = buf[2];
        let response_data_format = buf[3] & 0x0F;

        let mut vendor_id = [0u8; 8];
        vendor_id.copy_from_slice(&buf[8..16]);

        let mut product_id = [0u8; 16];
        product_id.copy_from_slice(&buf[16..32]);

        let mut product_rev = [0u8; 4];
        product_rev.copy_from_slice(&buf[32..36]);

        Some(Self {
            peripheral_device_type,
            is_removable,
            version,
            response_data_format,
            vendor_id,
            product_id,
            product_rev,
        })
    }

    /// Returns true if this device is a direct-access block device (disk/flash drive).
    pub fn is_direct_access(&self) -> bool {
        self.peripheral_device_type == SCSI_DEV_TYPE_DIRECT_ACCESS
    }

    /// Formats ASCII Vendor ID as a trimmed String.
    pub fn vendor_str(&self) -> String {
        let mut s = String::new();
        for &b in &self.vendor_id {
            if b.is_ascii_graphic() || b == b' ' {
                s.push(b as char);
            }
        }
        String::from(s.trim())
    }

    /// Formats ASCII Product ID as a trimmed String.
    pub fn product_str(&self) -> String {
        let mut s = String::new();
        for &b in &self.product_id {
            if b.is_ascii_graphic() || b == b' ' {
                s.push(b as char);
            }
        }
        String::from(s.trim())
    }

    /// Formats ASCII Product Revision as a trimmed String.
    pub fn revision_str(&self) -> String {
        let mut s = String::new();
        for &b in &self.product_rev {
            if b.is_ascii_graphic() || b == b' ' {
                s.push(b as char);
            }
        }
        String::from(s.trim())
    }
}

/// Parsed SCSI READ CAPACITY (10) Response Data (8 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiReadCapacity10Data {
    /// Returned Logical Block Address (maximum valid LBA).
    pub returned_lba: u32,
    /// Block length in bytes (typically 512).
    pub block_length: u32,
}

impl ScsiReadCapacity10Data {
    /// Parses the 8-byte big-endian READ CAPACITY (10) response buffer.
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 8 {
            return None;
        }

        let returned_lba = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let block_length = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);

        Some(Self {
            returned_lba,
            block_length,
        })
    }

    /// Total addressable sector / block count.
    pub fn total_blocks(&self) -> u64 {
        (self.returned_lba as u64) + 1
    }

    /// Total media capacity in bytes.
    pub fn capacity_bytes(&self) -> u64 {
        self.total_blocks() * (self.block_length as u64)
    }
}

/// Parsed SCSI REQUEST SENSE Response Data (18 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiSenseData {
    pub response_code: u8,
    pub sense_key: u8,
    pub asc: u8,
    pub ascq: u8,
}

impl ScsiSenseData {
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < 18 {
            return None;
        }
        Some(Self {
            response_code: buf[0] & 0x7F,
            sense_key: buf[2] & 0x0F,
            asc: buf[12],
            ascq: buf[13],
        })
    }

    pub fn is_no_sense(&self) -> bool {
        self.sense_key == 0x00
    }
}

/// Verification self-test for SCSI command building and response decoding.
pub fn verify_scsi_commands() -> bool {
    // 1. Verify TEST UNIT READY CDB
    let tur_cdb = ScsiCommand::test_unit_ready();
    if tur_cdb.length != 6 || tur_cdb.bytes[0] != SCSI_CMD_TEST_UNIT_READY {
        return false;
    }

    // 2. Verify INQUIRY CDB & Response decoding
    let inq_cdb = ScsiCommand::inquiry(36);
    if inq_cdb.length != 6 || inq_cdb.bytes[0] != SCSI_CMD_INQUIRY || inq_cdb.bytes[4] != 36 {
        return false;
    }

    let mut inq_buf = [0u8; 36];
    inq_buf[0] = SCSI_DEV_TYPE_DIRECT_ACCESS; // Direct-access block device
    inq_buf[1] = 0x80;                        // Removable media bit set
    inq_buf[2] = 0x04;                        // SPC-2 compliance
    inq_buf[3] = 0x02;                        // Response data format
    inq_buf[8..16].copy_from_slice(b"SanDisk ");
    inq_buf[16..32].copy_from_slice(b"Cruzer Blade    ");
    inq_buf[32..36].copy_from_slice(b"1.00");

    let inq_data = match ScsiInquiryData::parse(&inq_buf) {
        Some(d) => d,
        None => return false,
    };

    if !inq_data.is_direct_access() || !inq_data.is_removable {
        return false;
    }
    if inq_data.vendor_str() != "SanDisk" {
        return false;
    }
    if inq_data.product_str() != "Cruzer Blade" {
        return false;
    }
    if inq_data.revision_str() != "1.00" {
        return false;
    }

    // 3. Verify READ CAPACITY (10) CDB & Response decoding
    let rc_cdb = ScsiCommand::read_capacity_10();
    if rc_cdb.length != 10 || rc_cdb.bytes[0] != SCSI_CMD_READ_CAPACITY_10 {
        return false;
    }

    let mut rc_buf = [0u8; 8];
    // Set returned LBA = 0x01E8_47FF (31,999,999 -> 32,000,000 blocks * 512 = ~16 GB)
    rc_buf[0..4].copy_from_slice(&0x01E8_47FFu32.to_be_bytes());
    // Set block size = 512 bytes
    rc_buf[4..8].copy_from_slice(&512u32.to_be_bytes());

    let rc_data = match ScsiReadCapacity10Data::parse(&rc_buf) {
        Some(d) => d,
        None => return false,
    };

    if rc_data.returned_lba != 0x01E8_47FF || rc_data.block_length != 512 {
        return false;
    }
    if rc_data.total_blocks() != 32_000_000 {
        return false;
    }
    if rc_data.capacity_bytes() != 16_384_000_000 {
        return false;
    }

    // 4. Verify READ (10) and WRITE (10) CDB building
    let read_cdb = ScsiCommand::read_10(0x0002_0000, 8);
    if read_cdb.length != 10 || read_cdb.bytes[0] != SCSI_CMD_READ_10 {
        return false;
    }
    let expected_lba = 0x0002_0000u32.to_be_bytes();
    if read_cdb.bytes[2..6] != expected_lba || read_cdb.bytes[7..9] != 8u16.to_be_bytes() {
        return false;
    }

    let write_cdb = ScsiCommand::write_10(0x0004_1234, 16);
    if write_cdb.length != 10 || write_cdb.bytes[0] != SCSI_CMD_WRITE_10 {
        return false;
    }
    let exp_write_lba = 0x0004_1234u32.to_be_bytes();
    if write_cdb.bytes[2..6] != exp_write_lba || write_cdb.bytes[7..9] != 16u16.to_be_bytes() {
        return false;
    }

    true
}

// ============================================================================
// USB MSC Block Device Abstraction & Sector DMA Engine
// ============================================================================

/// Block Storage Device abstraction for USB Mass Storage Class Drives (e.g., /dev/sdb).
#[derive(Debug, Clone)]
pub struct UsbMscBlockDevice {
    pub transport: BotTransport,
    pub vendor: String,
    pub product: String,
    pub revision: String,
    pub block_count: u64,
    pub block_size: u32,
    pub is_removable: bool,
    pub is_initialized: bool,
}

impl UsbMscBlockDevice {
    /// Constructs a new USB Mass Storage Block Device attached to the given BOT transport.
    pub fn new(transport: BotTransport) -> Self {
        Self {
            transport,
            vendor: String::new(),
            product: String::new(),
            revision: String::new(),
            block_count: 0,
            block_size: 512,
            is_removable: true,
            is_initialized: false,
        }
    }

    /// Attaches parsed SCSI INQUIRY and READ CAPACITY 10 parameters to this block device.
    pub fn attach_identified_media(&mut self, inq: &ScsiInquiryData, cap: &ScsiReadCapacity10Data) {
        self.vendor = inq.vendor_str();
        self.product = inq.product_str();
        self.revision = inq.revision_str();
        self.is_removable = inq.is_removable;
        self.block_count = cap.total_blocks();
        self.block_size = cap.block_length;
        self.is_initialized = true;
    }

    #[inline(always)]
    pub fn capacity_bytes(&self) -> u64 {
        self.block_count.saturating_mul(self.block_size as u64)
    }

    #[inline(always)]
    pub fn block_count(&self) -> u64 {
        self.block_count
    }

    #[inline(always)]
    pub fn block_size(&self) -> u32 {
        self.block_size
    }

    /// Prepares a SCSI READ (10) command transaction.
    pub fn build_read_transaction(&mut self, lba: u32, count: u16) -> (CommandBlockWrapper, ScsiCdb) {
        let cdb = ScsiCommand::read_10(lba, count);
        let transfer_len = (count as u32) * self.block_size;
        let cbw = self.transport.create_cbw(&cdb, transfer_len, true);
        (cbw, cdb)
    }

    /// Prepares a SCSI WRITE (10) command transaction.
    pub fn build_write_transaction(&mut self, lba: u32, count: u16) -> (CommandBlockWrapper, ScsiCdb) {
        let cdb = ScsiCommand::write_10(lba, count);
        let transfer_len = (count as u32) * self.block_size;
        let cbw = self.transport.create_cbw(&cdb, transfer_len, false);
        (cbw, cdb)
    }

    /// Simulates sector read transfer against backing storage (loopback test & driver validation).
    pub fn execute_simulated_read(
        &mut self,
        backing_store: &[u8],
        lba: u32,
        count: u16,
        dest: &mut [u8],
    ) -> Result<CommandStatusWrapper, BotError> {
        let (cbw, _cdb) = self.build_read_transaction(lba, count);
        let expected_bytes = (count as usize) * (self.block_size as usize);
        if dest.len() < expected_bytes {
            return Err(BotError::BufferLengthMismatch);
        }

        self.transport.enter_data_phase(cbw.data_transfer_length, true);
        let offset = (lba as usize) * (self.block_size as usize);
        if offset + expected_bytes > backing_store.len() {
            return Err(BotError::BufferLengthMismatch);
        }

        dest[..expected_bytes].copy_from_slice(&backing_store[offset..offset + expected_bytes]);

        self.transport.enter_status_phase();
        let csw = CommandStatusWrapper::new(cbw.tag, 0, CSW_STATUS_PASSED);
        self.transport.finish_transaction(&csw, cbw.tag)?;

        Ok(csw)
    }

    /// Simulates sector write transfer into backing storage (loopback test & driver validation).
    pub fn execute_simulated_write(
        &mut self,
        backing_store: &mut [u8],
        lba: u32,
        count: u16,
        src: &[u8],
    ) -> Result<CommandStatusWrapper, BotError> {
        let (cbw, _cdb) = self.build_write_transaction(lba, count);
        let expected_bytes = (count as usize) * (self.block_size as usize);
        if src.len() < expected_bytes {
            return Err(BotError::BufferLengthMismatch);
        }

        self.transport.enter_data_phase(cbw.data_transfer_length, false);
        let offset = (lba as usize) * (self.block_size as usize);
        if offset + expected_bytes > backing_store.len() {
            return Err(BotError::BufferLengthMismatch);
        }

        backing_store[offset..offset + expected_bytes].copy_from_slice(&src[..expected_bytes]);

        self.transport.enter_status_phase();
        let csw = CommandStatusWrapper::new(cbw.tag, 0, CSW_STATUS_PASSED);
        self.transport.finish_transaction(&csw, cbw.tag)?;

        Ok(csw)
    }
}

/// Verification self-test for USB MSC block device engine and SCSI READ/WRITE DMA transactions.
pub fn verify_usb_msc_engine() -> bool {
    let transport = BotTransport::new(1, 1, 2, 512);
    let mut dev = UsbMscBlockDevice::new(transport);

    let mut inq_buf = [0u8; 36];
    inq_buf[0] = SCSI_DEV_TYPE_DIRECT_ACCESS;
    inq_buf[1] = 0x80;
    inq_buf[8..16].copy_from_slice(b"KntonOS ");
    inq_buf[16..32].copy_from_slice(b"USB Flash Drive ");
    inq_buf[32..36].copy_from_slice(b"1.00");
    let inq = match ScsiInquiryData::parse(&inq_buf) {
        Some(i) => i,
        None => return false,
    };

    let mut rc_buf = [0u8; 8];
    rc_buf[0..4].copy_from_slice(&0x000F_FFFFu32.to_be_bytes()); // 1,048,576 blocks (512 MiB)
    rc_buf[4..8].copy_from_slice(&512u32.to_be_bytes());
    let cap = match ScsiReadCapacity10Data::parse(&rc_buf) {
        Some(c) => c,
        None => return false,
    };

    dev.attach_identified_media(&inq, &cap);

    if !dev.is_initialized || dev.vendor != "KntonOS" || dev.product != "USB Flash Drive" {
        return false;
    }
    if dev.block_count() != 1_048_576 || dev.block_size() != 512 {
        return false;
    }
    if dev.capacity_bytes() != 536_870_912 {
        return false;
    }

    // 16 KiB backing store simulation
    let mut backing_store = [0u8; 16384];

    // Write 2 sectors (1024 bytes) at LBA 4
    let mut write_data = [0u8; 1024];
    for i in 0..1024 {
        write_data[i] = ((i * 17 + 43) & 0xFF) as u8;
    }

    if dev.execute_simulated_write(&mut backing_store, 4, 2, &write_data).is_err() {
        return false;
    }

    // Read back 2 sectors at LBA 4
    let mut read_data = [0u8; 1024];
    if dev.execute_simulated_read(&backing_store, 4, 2, &mut read_data).is_err() {
        return false;
    }

    if read_data != write_data {
        return false;
    }

    true
}
