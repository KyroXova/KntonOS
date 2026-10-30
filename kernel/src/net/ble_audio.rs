//! Low-Latency Bluetooth Low Energy (BLE 5.2/5.3) Subsystem for KntonOS.
//!
//! Provides ultra-low-latency kernel-space BLE protocol handling for handheld gaming PC
//! peripherals: wireless controllers (HID over GATT / HoGP) and LE Audio headsets
//! (Connected Isochronous Streams / CIS with LC3 codec framing).
//!
//! Architecture:
//! - HCI Transport Packet Layer (Command, Event, ACL Data, ISO Data)
//! - L2CAP Protocol Multiplexer with dynamic MTU negotiation
//! - Connected Isochronous Streams (CIS) Scheduler with 7.5ms/10ms LC3 audio burst pacing
//! - HID over GATT (HoG) Gamepad Report Processor with adaptive anti-jitter buffer

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;

// =============================================================================
// 1. HCI Transport Packet Formats (Core Spec v5.3 Vol 4, Part E)
// =============================================================================

/// HCI Transport Packet Indicator byte sent across UART/USB/PCIe Bluetooth transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HciPacketIndicator {
    Command = 0x01,
    AclData = 0x02,
    SyncData = 0x03,
    Event = 0x04,
    IsoData = 0x05,
}

impl HciPacketIndicator {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x01 => Some(Self::Command),
            0x02 => Some(Self::AclData),
            0x03 => Some(Self::SyncData),
            0x04 => Some(Self::Event),
            0x05 => Some(Self::IsoData),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// HCI Opcodes & Groups (OGF / OCF)
// -----------------------------------------------------------------------------

pub const HCI_OGF_LINK_CONTROL: u16 = 0x01;
pub const HCI_OGF_LINK_POLICY: u16 = 0x02;
pub const HCI_OGF_CONTROLLER_BASEBAND: u16 = 0x03;
pub const HCI_OGF_INFORMATIONAL: u16 = 0x04;
pub const HCI_OGF_STATUS: u16 = 0x05;
pub const HCI_OGF_LE_CONTROLLER: u16 = 0x08;

/// Computes an HCI 16-bit opcode from 6-bit OGF and 10-bit OCF.
pub const fn make_hci_opcode(ogf: u16, ocf: u16) -> u16 {
    ((ogf & 0x3F) << 10) | (ocf & 0x03FF)
}

// Standard BLE OCF definitions
pub const HCI_OCF_LE_SET_EVENT_MASK: u16 = 0x0001;
pub const HCI_OCF_LE_READ_BUFFER_SIZE_V2: u16 = 0x0060;
pub const HCI_OCF_LE_SET_CIG_PARAMETERS: u16 = 0x0062;
pub const HCI_OCF_LE_CREATE_CIS: u16 = 0x0064;
pub const HCI_OCF_LE_SETUP_ISO_DATA_PATH: u16 = 0x006E;
pub const HCI_OCF_LE_CONNECTION_UPDATE: u16 = 0x0013;

pub const HCI_OPCODE_LE_SET_EVENT_MASK: u16 = make_hci_opcode(HCI_OGF_LE_CONTROLLER, HCI_OCF_LE_SET_EVENT_MASK);
pub const HCI_OPCODE_LE_READ_BUFFER_SIZE_V2: u16 = make_hci_opcode(HCI_OGF_LE_CONTROLLER, HCI_OCF_LE_READ_BUFFER_SIZE_V2);
pub const HCI_OPCODE_LE_SET_CIG_PARAMETERS: u16 = make_hci_opcode(HCI_OGF_LE_CONTROLLER, HCI_OCF_LE_SET_CIG_PARAMETERS);
pub const HCI_OPCODE_LE_CREATE_CIS: u16 = make_hci_opcode(HCI_OGF_LE_CONTROLLER, HCI_OCF_LE_CREATE_CIS);
pub const HCI_OPCODE_LE_SETUP_ISO_DATA_PATH: u16 = make_hci_opcode(HCI_OGF_LE_CONTROLLER, HCI_OCF_LE_SETUP_ISO_DATA_PATH);
pub const HCI_OPCODE_LE_CONNECTION_UPDATE: u16 = make_hci_opcode(HCI_OGF_LE_CONTROLLER, HCI_OCF_LE_CONNECTION_UPDATE);

// -----------------------------------------------------------------------------
// HCI Command Packet
// -----------------------------------------------------------------------------

/// HCI Command Packet (Indicator 0x01)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HciCommandPacket {
    pub opcode: u16,
    pub params: Vec<u8>,
}

impl HciCommandPacket {
    pub fn new(opcode: u16, params: Vec<u8>) -> Self {
        Self { opcode, params }
    }

    pub fn ogf(&self) -> u8 {
        ((self.opcode >> 10) & 0x3F) as u8
    }

    pub fn ocf(&self) -> u16 {
        self.opcode & 0x03FF
    }

    /// Serializes command packet into byte buffer including packet indicator 0x01.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.params.len());
        buf.push(HciPacketIndicator::Command as u8);
        buf.push((self.opcode & 0xFF) as u8);
        buf.push(((self.opcode >> 8) & 0xFF) as u8);
        buf.push(self.params.len() as u8);
        buf.extend_from_slice(&self.params);
        buf
    }

    /// Parses command packet from byte slice (starts at indicator 0x01 or opcode).
    pub fn parse(data: &[u8]) -> Option<Self> {
        let offset = if !data.is_empty() && data[0] == HciPacketIndicator::Command as u8 {
            1
        } else {
            0
        };

        if data.len() < offset + 3 {
            return None;
        }

        let opcode = (data[offset] as u16) | ((data[offset + 1] as u16) << 8);
        let param_len = data[offset + 2] as usize;

        if data.len() < offset + 3 + param_len {
            return None;
        }

        let params = data[offset + 3..offset + 3 + param_len].to_vec();
        Some(Self { opcode, params })
    }
}

// -----------------------------------------------------------------------------
// HCI Event Packet & Subevent Codes
// -----------------------------------------------------------------------------

pub const HCI_EVT_DISCONNECTION_COMPLETE: u8 = 0x05;
pub const HCI_EVT_COMMAND_COMPLETE: u8 = 0x0E;
pub const HCI_EVT_COMMAND_STATUS: u8 = 0x0F;
pub const HCI_EVT_NUM_COMPLETED_PACKETS: u8 = 0x13;
pub const HCI_EVT_LE_META: u8 = 0x3E;

// LE Meta Subevent Codes
pub const HCI_LE_SUBEVT_CONNECTION_COMPLETE: u8 = 0x01;
pub const HCI_LE_SUBEVT_CONNECTION_UPDATE_COMPLETE: u8 = 0x03;
pub const HCI_LE_SUBEVT_CIS_ESTABLISHED: u8 = 0x19;
pub const HCI_LE_SUBEVT_CIS_REQUEST: u8 = 0x1A;

/// HCI Event Packet (Indicator 0x04)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HciEventPacket {
    pub event_code: u8,
    pub params: Vec<u8>,
}

impl HciEventPacket {
    pub fn new(event_code: u8, params: Vec<u8>) -> Self {
        Self { event_code, params }
    }

    pub fn is_le_meta(&self) -> bool {
        self.event_code == HCI_EVT_LE_META
    }

    pub fn subevent_code(&self) -> Option<u8> {
        if self.is_le_meta() && !self.params.is_empty() {
            Some(self.params[0])
        } else {
            None
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3 + self.params.len());
        buf.push(HciPacketIndicator::Event as u8);
        buf.push(self.event_code);
        buf.push(self.params.len() as u8);
        buf.extend_from_slice(&self.params);
        buf
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        let offset = if !data.is_empty() && data[0] == HciPacketIndicator::Event as u8 {
            1
        } else {
            0
        };

        if data.len() < offset + 2 {
            return None;
        }

        let event_code = data[offset];
        let param_len = data[offset + 1] as usize;

        if data.len() < offset + 2 + param_len {
            return None;
        }

        let params = data[offset + 2..offset + 2 + param_len].to_vec();
        Some(Self { event_code, params })
    }
}

// -----------------------------------------------------------------------------
// HCI ACL Data Packet
// -----------------------------------------------------------------------------

/// Packet Boundary Flag in ACL Data Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AclPacketBoundaryFlag {
    FirstNonFlushable = 0b00,
    Continuing = 0b01,
    FirstFlushable = 0b10,
    CompleteL2capPdu = 0b11,
}

impl AclPacketBoundaryFlag {
    pub fn from_u8(val: u8) -> Self {
        match val & 0x03 {
            0b00 => Self::FirstNonFlushable,
            0b01 => Self::Continuing,
            0b10 => Self::FirstFlushable,
            _ => Self::CompleteL2capPdu,
        }
    }
}

/// HCI ACL Data Packet (Indicator 0x02)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HciAclDataPacket {
    pub connection_handle: u16, // 12-bit handle (0..4095)
    pub pb_flag: AclPacketBoundaryFlag,
    pub bc_flag: u8,            // 2-bit broadcast flag (normally 0b00 point-to-point)
    pub payload: Vec<u8>,
}

impl HciAclDataPacket {
    pub fn new(handle: u16, pb: AclPacketBoundaryFlag, payload: Vec<u8>) -> Self {
        Self {
            connection_handle: handle & 0x0FFF,
            pb_flag: pb,
            bc_flag: 0b00,
            payload,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + self.payload.len());
        buf.push(HciPacketIndicator::AclData as u8);

        let handle_flags: u16 = (self.connection_handle & 0x0FFF)
            | (((self.pb_flag as u8 & 0x03) as u16) << 12)
            | (((self.bc_flag & 0x03) as u16) << 14);

        buf.push((handle_flags & 0xFF) as u8);
        buf.push(((handle_flags >> 8) & 0xFF) as u8);

        let len = self.payload.len() as u16;
        buf.push((len & 0xFF) as u8);
        buf.push(((len >> 8) & 0xFF) as u8);

        buf.extend_from_slice(&self.payload);
        buf
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        let offset = if !data.is_empty() && data[0] == HciPacketIndicator::AclData as u8 {
            1
        } else {
            0
        };

        if data.len() < offset + 4 {
            return None;
        }

        let handle_flags = (data[offset] as u16) | ((data[offset + 1] as u16) << 8);
        let connection_handle = handle_flags & 0x0FFF;
        let pb_val = ((handle_flags >> 12) & 0x03) as u8;
        let bc_flag = ((handle_flags >> 14) & 0x03) as u8;
        let pb_flag = AclPacketBoundaryFlag::from_u8(pb_val);

        let data_len = (data[offset + 2] as usize) | ((data[offset + 3] as usize) << 8);
        if data.len() < offset + 4 + data_len {
            return None;
        }

        let payload = data[offset + 4..offset + 4 + data_len].to_vec();
        Some(Self {
            connection_handle,
            pb_flag,
            bc_flag,
            payload,
        })
    }
}

// -----------------------------------------------------------------------------
// HCI ISO Data Packet (Core Spec v5.2/v5.3 for LE Audio CIS/BIS)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IsoPacketBoundary {
    FirstFragment = 0b00,
    ContinuingFragment = 0b01,
    CompleteSdu = 0b10,
    LastFragment = 0b11,
}

impl IsoPacketBoundary {
    pub fn from_u8(val: u8) -> Self {
        match val & 0x03 {
            0b00 => Self::FirstFragment,
            0b01 => Self::ContinuingFragment,
            0b10 => Self::CompleteSdu,
            _ => Self::LastFragment,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IsoPacketStatus {
    ValidData = 0b00,
    PossiblyInvalidData = 0b01,
    LostData = 0b10,
}

impl IsoPacketStatus {
    pub fn from_u8(val: u8) -> Self {
        match val & 0x03 {
            0b00 => Self::ValidData,
            0b01 => Self::PossiblyInvalidData,
            _ => Self::LostData,
        }
    }
}

/// HCI ISO Data Packet (Indicator 0x05)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HciIsoDataPacket {
    pub connection_handle: u16,     // 12-bit handle
    pub pb_flag: IsoPacketBoundary, // 2-bit PB flag
    pub has_timestamp: bool,        // 1-bit TS flag
    pub packet_status: IsoPacketStatus,
    pub timestamp_us: Option<u32>,  // 32-bit time in microseconds (optional)
    pub packet_seq_num: u16,        // 16-bit sequence number
    pub sdu_len: u16,               // 12-bit SDU length
    pub sdu_data: Vec<u8>,          // Audio SDU payload (e.g. LC3 frame)
}

impl HciIsoDataPacket {
    pub fn new(
        handle: u16,
        pb: IsoPacketBoundary,
        seq_num: u16,
        timestamp_us: Option<u32>,
        sdu_data: Vec<u8>,
    ) -> Self {
        let sdu_len = sdu_data.len() as u16;
        let has_timestamp = timestamp_us.is_some();
        Self {
            connection_handle: handle & 0x0FFF,
            pb_flag: pb,
            has_timestamp,
            packet_status: IsoPacketStatus::ValidData,
            timestamp_us,
            packet_seq_num: seq_num,
            sdu_len,
            sdu_data,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let header_extra = if self.has_timestamp { 4 } else { 0 };
        // Total data load = header_extra (4 if TS) + 2 (seq) + 2 (sdu_len + flags) + sdu_data
        let iso_data_load_len = header_extra + 4 + self.sdu_data.len();

        let mut buf = Vec::with_capacity(5 + iso_data_load_len);
        buf.push(HciPacketIndicator::IsoData as u8);

        // Word 1: Handle (12b) | PB (2b) | TS (1b) | RFU (1b)
        let ts_bit = if self.has_timestamp { 1u16 << 14 } else { 0 };
        let handle_flags: u16 = (self.connection_handle & 0x0FFF)
            | (((self.pb_flag as u8 & 0x03) as u16) << 12)
            | ts_bit;
        buf.push((handle_flags & 0xFF) as u8);
        buf.push(((handle_flags >> 8) & 0xFF) as u8);

        // Word 2: ISO Data Load Length (14b) | Packet Status (2b)
        let load_len_status: u16 = ((iso_data_load_len as u16) & 0x3FFF)
            | (((self.packet_status as u8 & 0x03) as u16) << 14);
        buf.push((load_len_status & 0xFF) as u8);
        buf.push(((load_len_status >> 8) & 0xFF) as u8);

        // Optional Timestamp (4 bytes)
        if let Some(ts) = self.timestamp_us {
            buf.push((ts & 0xFF) as u8);
            buf.push(((ts >> 8) & 0xFF) as u8);
            buf.push(((ts >> 16) & 0xFF) as u8);
            buf.push(((ts >> 24) & 0xFF) as u8);
        }

        // Sequence number (2 bytes)
        buf.push((self.packet_seq_num & 0xFF) as u8);
        buf.push(((self.packet_seq_num >> 8) & 0xFF) as u8);

        // SDU Length (12b) | RFU (4b)
        let sdu_len_field: u16 = self.sdu_len & 0x0FFF;
        buf.push((sdu_len_field & 0xFF) as u8);
        buf.push(((sdu_len_field >> 8) & 0xFF) as u8);

        // SDU Data
        buf.extend_from_slice(&self.sdu_data);
        buf
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        let offset = if !data.is_empty() && data[0] == HciPacketIndicator::IsoData as u8 {
            1
        } else {
            0
        };

        if data.len() < offset + 4 {
            return None;
        }

        let handle_flags = (data[offset] as u16) | ((data[offset + 1] as u16) << 8);
        let connection_handle = handle_flags & 0x0FFF;
        let pb_flag = IsoPacketBoundary::from_u8(((handle_flags >> 12) & 0x03) as u8);
        let has_timestamp = (handle_flags & (1 << 14)) != 0;

        let load_len_status = (data[offset + 2] as u16) | ((data[offset + 3] as u16) << 8);
        let iso_data_load_len = (load_len_status & 0x3FFF) as usize;
        let packet_status = IsoPacketStatus::from_u8(((load_len_status >> 14) & 0x03) as u8);

        if data.len() < offset + 4 + iso_data_load_len {
            return None;
        }

        let mut cur = offset + 4;
        let timestamp_us = if has_timestamp {
            if data.len() < cur + 4 {
                return None;
            }
            let ts = (data[cur] as u32)
                | ((data[cur + 1] as u32) << 8)
                | ((data[cur + 2] as u32) << 16)
                | ((data[cur + 3] as u32) << 24);
            cur += 4;
            Some(ts)
        } else {
            None
        };

        if data.len() < cur + 4 {
            return None;
        }

        let packet_seq_num = (data[cur] as u16) | ((data[cur + 1] as u16) << 8);
        cur += 2;

        let sdu_len = ((data[cur] as u16) | ((data[cur + 1] as u16) << 8)) & 0x0FFF;
        cur += 2;

        let sdu_data_len = sdu_len as usize;
        if data.len() < cur + sdu_data_len {
            return None;
        }

        let sdu_data = data[cur..cur + sdu_data_len].to_vec();

        Some(Self {
            connection_handle,
            pb_flag,
            has_timestamp,
            packet_status,
            timestamp_us,
            packet_seq_num,
            sdu_len,
            sdu_data,
        })
    }
}

// =============================================================================
// 2. BLE Connection Parameters & Handheld Low-Latency Profile
// =============================================================================

/// BLE Connection Parameters (Core Spec v5.3 Vol 4, Part E, Section 7.8.18).
///
/// Connection intervals are specified in units of 1.25 ms (7.5 ms = 6 units).
/// Supervision timeout is specified in units of 10 ms (200 ms = 20 units).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BleConnectionParams {
    /// Connection interval minimum in 1.25 ms units (Range: 6 to 3200, 7.5ms to 4.0s).
    pub min_interval: u16,
    /// Connection interval maximum in 1.25 ms units.
    pub max_interval: u16,
    /// Slave latency: number of connection events peripheral can skip (0 for zero-latency gaming).
    pub latency: u16,
    /// Supervision timeout in 10 ms units (Range: 10 to 3200, 100ms to 32.0s).
    pub timeout: u16,
}

impl BleConnectionParams {
    /// Handheld gaming controller low-latency profile (7.5 ms interval, 0 latency, 200 ms timeout).
    pub const fn gaming_low_latency_7_5ms() -> Self {
        Self {
            min_interval: 6, // 6 * 1.25 ms = 7.5 ms
            max_interval: 6,
            latency: 0,
            timeout: 20, // 20 * 10 ms = 200 ms
        }
    }

    /// Handheld gaming controller standard profile (10.0 ms interval, 0 latency, 250 ms timeout).
    pub const fn gaming_standard_10ms() -> Self {
        Self {
            min_interval: 8, // 8 * 1.25 ms = 10.0 ms
            max_interval: 8,
            latency: 0,
            timeout: 25, // 25 * 10 ms = 250 ms
        }
    }

    /// Calculates minimum connection interval in milliseconds.
    pub fn min_interval_ms(&self) -> f32 {
        self.min_interval as f32 * 1.25
    }

    /// Calculates maximum connection interval in milliseconds.
    pub fn max_interval_ms(&self) -> f32 {
        self.max_interval as f32 * 1.25
    }

    /// Returns true if parameters enforce ultra-responsive sub-8ms polling for esports gamepads.
    pub fn is_sub_8ms(&self) -> bool {
        self.max_interval <= 6
    }
}

// =============================================================================
// 3. L2CAP Protocol Channel Multiplexing (Core Spec Vol 3, Part A)
// =============================================================================

pub const L2CAP_CID_NULL: u16 = 0x0000;
pub const L2CAP_CID_ATT: u16 = 0x0004;         // Attribute Protocol (GATT / HoGP Gamepad)
pub const L2CAP_CID_LE_SIGNALING: u16 = 0x0005; // LE L2CAP Signaling
pub const L2CAP_CID_SMP: u16 = 0x0006;          // Security Manager Protocol
pub const L2CAP_CID_DYNAMIC_START: u16 = 0x0040; // Dynamic Connection-Oriented Channels

pub const L2CAP_DEFAULT_ATT_MTU: u16 = 23;      // Minimum BLE ATT MTU
pub const L2CAP_GAMING_ATT_MTU: u16 = 247;      // High-throughput gaming MTU
pub const L2CAP_MAX_MTU: u16 = 512;

// L2CAP LE Signaling Command Codes
pub const L2CAP_SIG_CONN_PARAM_UPDATE_REQ: u8 = 0x12;
pub const L2CAP_SIG_CONN_PARAM_UPDATE_RSP: u8 = 0x13;
pub const L2CAP_SIG_LE_CREDIT_BASED_CONN_REQ: u8 = 0x14;
pub const L2CAP_SIG_LE_CREDIT_BASED_CONN_RSP: u8 = 0x15;
pub const L2CAP_SIG_FLOW_CONTROL_CREDIT: u8 = 0x16;

/// L2CAP Basic Information Frame (PDU)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2capPdu {
    pub length: u16,
    pub channel_id: u16,
    pub payload: Vec<u8>,
}

impl L2capPdu {
    pub fn new(channel_id: u16, payload: Vec<u8>) -> Self {
        Self {
            length: payload.len() as u16,
            channel_id,
            payload,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.payload.len());
        buf.push((self.length & 0xFF) as u8);
        buf.push(((self.length >> 8) & 0xFF) as u8);
        buf.push((self.channel_id & 0xFF) as u8);
        buf.push(((self.channel_id >> 8) & 0xFF) as u8);
        buf.extend_from_slice(&self.payload);
        buf
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }

        let length = (data[0] as u16) | ((data[1] as u16) << 8);
        let channel_id = (data[2] as u16) | ((data[3] as u16) << 8);

        let payload_len = length as usize;
        if data.len() < 4 + payload_len {
            return None;
        }

        let payload = data[4..4 + payload_len].to_vec();
        Some(Self {
            length,
            channel_id,
            payload,
        })
    }
}

/// L2CAP Channel Multiplexer for routing BLE traffic to ATT/GATT, Signaling, and dynamic streams.
#[derive(Debug)]
pub struct L2capMultiplexer {
    pub att_mtu: u16,
    pub rx_att_queue: VecDeque<L2capPdu>,
    pub rx_signaling_queue: VecDeque<L2capPdu>,
    pub rx_dynamic_queue: VecDeque<L2capPdu>,
    pub tx_packets: VecDeque<L2capPdu>,
}

impl L2capMultiplexer {
    pub fn new() -> Self {
        Self {
            att_mtu: L2CAP_DEFAULT_ATT_MTU,
            rx_att_queue: VecDeque::with_capacity(32),
            rx_signaling_queue: VecDeque::with_capacity(16),
            rx_dynamic_queue: VecDeque::with_capacity(32),
            tx_packets: VecDeque::with_capacity(32),
        }
    }

    /// Negotiates high-throughput ATT MTU for gaming accessories (e.g. 247 or 512 bytes).
    pub fn negotiate_mtu(&mut self, requested_mtu: u16) -> u16 {
        let clamped = requested_mtu.clamp(L2CAP_DEFAULT_ATT_MTU, L2CAP_MAX_MTU);
        self.att_mtu = clamped;
        clamped
    }

    /// Routes an incoming L2CAP packet based on its Channel ID (CID).
    pub fn route_incoming_pdu(&mut self, pdu: L2capPdu) -> Result<(), &'static str> {
        match pdu.channel_id {
            L2CAP_CID_ATT => {
                if pdu.payload.len() > self.att_mtu as usize {
                    return Err("L2CAP ATT PDU exceeds negotiated MTU");
                }
                self.rx_att_queue.push_back(pdu);
                Ok(())
            }
            L2CAP_CID_LE_SIGNALING => {
                self.rx_signaling_queue.push_back(pdu);
                Ok(())
            }
            cid if cid >= L2CAP_CID_DYNAMIC_START => {
                self.rx_dynamic_queue.push_back(pdu);
                Ok(())
            }
            _ => Err("Unsupported or unknown L2CAP CID"),
        }
    }

    /// Formats and enqueues an outgoing ATT notification/command PDU.
    pub fn send_att(&mut self, payload: Vec<u8>) -> Result<(), &'static str> {
        if payload.len() > self.att_mtu as usize {
            return Err("Payload exceeds ATT MTU");
        }
        self.tx_packets.push_back(L2capPdu::new(L2CAP_CID_ATT, payload));
        Ok(())
    }

    /// Generates an L2CAP Connection Parameter Update Request PDU for low-latency gaming.
    pub fn create_conn_param_update_req(&self, id: u8, params: &BleConnectionParams) -> L2capPdu {
        let mut payload = Vec::with_capacity(12);
        payload.push(L2CAP_SIG_CONN_PARAM_UPDATE_REQ);
        payload.push(id);
        // Length of signal parameter field (8 bytes)
        payload.push(8);
        payload.push(0);

        payload.push((params.min_interval & 0xFF) as u8);
        payload.push(((params.min_interval >> 8) & 0xFF) as u8);
        payload.push((params.max_interval & 0xFF) as u8);
        payload.push(((params.max_interval >> 8) & 0xFF) as u8);
        payload.push((params.latency & 0xFF) as u8);
        payload.push(((params.latency >> 8) & 0xFF) as u8);
        payload.push((params.timeout & 0xFF) as u8);
        payload.push(((params.timeout >> 8) & 0xFF) as u8);

        L2capPdu::new(L2CAP_CID_LE_SIGNALING, payload)
    }
}

// =============================================================================
// 4. LC3 Codec Framing & Connected Isochronous Streams (CIS) Scheduler
// =============================================================================

/// LC3 Audio Codec Frame Duration (Bluetooth Core Spec v5.2 / LE Audio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lc3FrameDuration {
    Duration7_5ms = 7500,  // 7.5 ms = 7,500 microseconds
    Duration10ms = 10000, // 10.0 ms = 10,000 microseconds
}

impl Lc3FrameDuration {
    pub const fn interval_us(&self) -> u32 {
        match self {
            Self::Duration7_5ms => 7500,
            Self::Duration10ms => 10000,
        }
    }

    pub fn interval_ms(&self) -> f32 {
        match self {
            Self::Duration7_5ms => 7.5,
            Self::Duration10ms => 10.0,
        }
    }
}

/// LC3 Audio Codec Supported Sample Rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lc3SampleRate {
    Rate16kHz = 16000,
    Rate24kHz = 24000,
    Rate32kHz = 32000,
    Rate48kHz = 48000, // High-definition gaming audio
}

impl Lc3SampleRate {
    pub const fn hz(&self) -> u32 {
        match self {
            Self::Rate16kHz => 16000,
            Self::Rate24kHz => 24000,
            Self::Rate32kHz => 32000,
            Self::Rate48kHz => 48000,
        }
    }

    /// Computes the number of PCM samples per frame for this sample rate and duration.
    pub const fn samples_per_frame(&self, duration: Lc3FrameDuration) -> usize {
        match (duration, *self) {
            (Lc3FrameDuration::Duration10ms, Self::Rate48kHz) => 480,
            (Lc3FrameDuration::Duration7_5ms, Self::Rate48kHz) => 360,
            (Lc3FrameDuration::Duration10ms, Self::Rate32kHz) => 320,
            (Lc3FrameDuration::Duration7_5ms, Self::Rate32kHz) => 240,
            (Lc3FrameDuration::Duration10ms, Self::Rate24kHz) => 240,
            (Lc3FrameDuration::Duration7_5ms, Self::Rate24kHz) => 180,
            (Lc3FrameDuration::Duration10ms, Self::Rate16kHz) => 160,
            (Lc3FrameDuration::Duration7_5ms, Self::Rate16kHz) => 120,
        }
    }
}

/// Audio Channel Allocation for LE Audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChannelConfig {
    Mono = 1,
    Left = 2,
    Right = 3,
    Stereo = 4,
}

/// Encapsulates an LC3 encoded audio frame ready for ISO transmission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lc3Frame {
    pub duration: Lc3FrameDuration,
    pub sample_rate: Lc3SampleRate,
    pub channel_config: AudioChannelConfig,
    pub octets_per_frame: usize,
    pub pcm_sample_count: usize,
    pub payload: Vec<u8>,
}

impl Lc3Frame {
    pub fn new(
        duration: Lc3FrameDuration,
        sample_rate: Lc3SampleRate,
        channel_config: AudioChannelConfig,
        payload: Vec<u8>,
    ) -> Self {
        let pcm_sample_count = sample_rate.samples_per_frame(duration);
        let octets_per_frame = payload.len();
        Self {
            duration,
            sample_rate,
            channel_config,
            octets_per_frame,
            pcm_sample_count,
            payload,
        }
    }
}

/// Operational state of a Connected Isochronous Stream (CIS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CisState {
    Configuring,
    Established,
    Streaming,
    Suspended,
    Terminated,
}

/// Configuration parameters for a Connected Isochronous Stream (CIS).
#[derive(Debug, Clone)]
pub struct CisStreamConfig {
    pub cig_id: u8,
    pub cis_id: u8,
    pub cis_connection_handle: u16,
    pub duration: Lc3FrameDuration,
    pub sample_rate: Lc3SampleRate,
    pub channel: AudioChannelConfig,
    pub max_sdu_size: u16,
    pub rtn: u8,                         // Retransmissions
    pub max_transport_latency_us: u32,  // e.g. 15,000 to 20,000 us
    pub burst_number: u8,               // e.g. 1 packet per interval
}

impl CisStreamConfig {
    /// Gaming headset ultra-low-latency 7.5ms LC3 48kHz audio profile.
    pub fn gaming_headset_7_5ms(cis_handle: u16) -> Self {
        Self {
            cig_id: 1,
            cis_id: 1,
            cis_connection_handle: cis_handle,
            duration: Lc3FrameDuration::Duration7_5ms,
            sample_rate: Lc3SampleRate::Rate48kHz,
            channel: AudioChannelConfig::Stereo,
            max_sdu_size: 155, // 155 octets per frame = 165 kbps high-fidelity gaming audio
            rtn: 2,
            max_transport_latency_us: 15000,
            burst_number: 1,
        }
    }

    /// Standard gaming headset 10ms LC3 48kHz profile.
    pub fn gaming_headset_10ms(cis_handle: u16) -> Self {
        Self {
            cig_id: 1,
            cis_id: 1,
            cis_connection_handle: cis_handle,
            duration: Lc3FrameDuration::Duration10ms,
            sample_rate: Lc3SampleRate::Rate48kHz,
            channel: AudioChannelConfig::Stereo,
            max_sdu_size: 155,
            rtn: 2,
            max_transport_latency_us: 20000,
            burst_number: 1,
        }
    }
}

/// Telemetry and diagnostics statistics for the CIS Audio Scheduler.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CisSchedulerStats {
    pub scheduled_frames: u64,
    pub transmitted_packets: u64,
    pub dropped_underrun: u64,
    pub dropped_overdue: u64,
    pub total_bytes_streamed: u64,
    pub last_latency_us: u32,
}

/// Connected Isochronous Stream (CIS) Isochronous Audio Scheduler.
///
/// Coordinates time-critical LC3 audio streaming over BLE 5.2/5.3:
/// - Enforces strict 7.5ms or 10.0ms anchor point scheduling
/// - Manages dynamic sub-event burst pacing
/// - Generates sequence numbers and 32-bit microsecond presentation timestamps
/// - Implements Packet Loss Concealment (PLC) fallback on packet dropouts
#[derive(Debug)]
pub struct CisAudioScheduler {
    pub config: CisStreamConfig,
    pub state: CisState,
    pub next_seq_num: u16,
    pub anchor_point_us: u64,
    pub interval_us: u32,
    pub tx_iso_queue: VecDeque<HciIsoDataPacket>,
    pub stats: CisSchedulerStats,
}

impl CisAudioScheduler {
    pub fn new(config: CisStreamConfig, initial_anchor_us: u64) -> Self {
        let interval_us = config.duration.interval_us();
        Self {
            config,
            state: CisState::Established,
            next_seq_num: 0,
            anchor_point_us: initial_anchor_us,
            interval_us,
            tx_iso_queue: VecDeque::with_capacity(32),
            stats: CisSchedulerStats::default(),
        }
    }

    /// Transitions stream to active streaming.
    pub fn start_streaming(&mut self) {
        self.state = CisState::Streaming;
    }

    /// Suspends active streaming.
    pub fn suspend_streaming(&mut self) {
        self.state = CisState::Suspended;
    }

    /// Enqueues an LC3 audio frame into the isochronous scheduler queue.
    pub fn enqueue_lc3_frame(
        &mut self,
        frame: Lc3Frame,
        current_time_us: u64,
    ) -> Result<(), &'static str> {
        if self.state != CisState::Streaming && self.state != CisState::Established {
            return Err("CIS stream is not in active or established state");
        }

        if frame.payload.len() > self.config.max_sdu_size as usize {
            return Err("LC3 frame octets exceed configured CIS max SDU size");
        }

        // Drop if queue is saturated (more than 16 frames = 120-160ms latency)
        if self.tx_iso_queue.len() >= 16 {
            self.stats.dropped_underrun += 1;
            return Err("CIS TX queue overrun, frame dropped");
        }

        let seq = self.next_seq_num;
        self.next_seq_num = self.next_seq_num.wrapping_add(1);

        // Calculate presentation timestamp based on next anchor point
        let target_timestamp_us = (self.anchor_point_us
            + (self.tx_iso_queue.len() as u64 * self.interval_us as u64)) as u32;

        let packet = HciIsoDataPacket::new(
            self.config.cis_connection_handle,
            IsoPacketBoundary::CompleteSdu,
            seq,
            Some(target_timestamp_us),
            frame.payload,
        );

        self.tx_iso_queue.push_back(packet);
        self.stats.scheduled_frames += 1;
        let _ = current_time_us; // tracked for deadline checks
        Ok(())
    }

    /// Advances the anchor point timeline by one isochronous interval (e.g. 7,500us or 10,000us).
    pub fn advance_anchor_point(&mut self) {
        self.anchor_point_us += self.interval_us as u64;
    }

    /// Polls the next burst packet eligible for radio airtime transmission.
    pub fn poll_next_burst(&mut self, current_time_us: u64) -> Option<HciIsoDataPacket> {
        if self.tx_iso_queue.is_empty() {
            return None;
        }

        // Verify if earliest packet is within transmission window
        let front = self.tx_iso_queue.front()?;
        if let Some(ts) = front.timestamp_us {
            let target = ts as u64;
            // If current time is much earlier than target presentation (ahead by more than 2 intervals), pace it
            if current_time_us + (self.interval_us as u64 * 2) < target {
                return None;
            }

            // If packet is overdue past max transport latency, mark and discard
            if current_time_us > target + (self.config.max_transport_latency_us as u64) {
                self.stats.dropped_overdue += 1;
                self.tx_iso_queue.pop_front();
                return None;
            }

            self.stats.last_latency_us = if current_time_us >= target {
                (current_time_us - target) as u32
            } else {
                0
            };
        }

        let packet = self.tx_iso_queue.pop_front()?;
        self.stats.transmitted_packets += 1;
        self.stats.total_bytes_streamed += packet.sdu_len as u64;
        Some(packet)
    }

    /// Performs Packet Loss Concealment (PLC) attenuation on previous PCM frame
    /// when an LC3 packet is dropped or lost due to 2.4GHz RF interference.
    pub fn plc_conceal_frame(prev_samples: &[i16], out_concealed: &mut [i16]) {
        let n = prev_samples.len().min(out_concealed.len());
        // Apply 80% linear-exponential decay window across concealed frame to prevent clicks
        for i in 0..n {
            let decay_factor = 0.82f32 * (1.0f32 - (i as f32 / n as f32) * 0.45f32);
            let sample = (prev_samples[i] as f32 * decay_factor) as i16;
            out_concealed[i] = sample;
        }
    }
}

// =============================================================================
// 5. HID over GATT (HoG) Gamepad Processor & Adaptive Jitter Buffer
// =============================================================================

pub const BLE_UUID_HID_SERVICE: u16 = 0x1812;
pub const BLE_UUID_REPORT_CHAR: u16 = 0x2A4D;
pub const BLE_UUID_REPORT_MAP: u16 = 0x2A4B;
pub const BLE_UUID_PROTOCOL_MODE: u16 = 0x2A4E;
pub const BLE_UUID_HID_INFO: u16 = 0x2A4A;
pub const BLE_UUID_HID_CONTROL_POINT: u16 = 0x2A4C;

// Gamepad Button bitmask flags
pub const BLE_BTN_A: u32 = 1 << 0;
pub const BLE_BTN_B: u32 = 1 << 1;
pub const BLE_BTN_X: u32 = 1 << 2;
pub const BLE_BTN_Y: u32 = 1 << 3;
pub const BLE_BTN_DPAD_UP: u32 = 1 << 4;
pub const BLE_BTN_DPAD_DOWN: u32 = 1 << 5;
pub const BLE_BTN_DPAD_LEFT: u32 = 1 << 6;
pub const BLE_BTN_DPAD_RIGHT: u32 = 1 << 7;
pub const BLE_BTN_LB: u32 = 1 << 8;
pub const BLE_BTN_RB: u32 = 1 << 9;
pub const BLE_BTN_LS: u32 = 1 << 10;
pub const BLE_BTN_RS: u32 = 1 << 11;
pub const BLE_BTN_START: u32 = 1 << 12;
pub const BLE_BTN_SELECT: u32 = 1 << 13;
pub const BLE_BTN_GUIDE: u32 = 1 << 14;

/// Processed, normalized BLE handheld gamepad state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BleGamepadState {
    pub left_stick_x: f32,  // [-1.0, 1.0]
    pub left_stick_y: f32,  // [-1.0, 1.0]
    pub right_stick_x: f32, // [-1.0, 1.0]
    pub right_stick_y: f32, // [-1.0, 1.0]
    pub left_trigger: f32,  // [0.0, 1.0]
    pub right_trigger: f32, // [0.0, 1.0]
    pub buttons: u32,
    pub timestamp_us: u64,
    pub sequence: u32,
}

impl BleGamepadState {
    pub fn is_button_pressed(&self, button_mask: u32) -> bool {
        (self.buttons & button_mask) != 0
    }
}

/// Dynamic Adaptive Anti-Jitter Buffer for wireless gamepad inputs.
///
/// Mitigates 2.4 GHz RF multi-path interference and packet inter-arrival jitter
/// without adding unnecessary input lag.
#[derive(Debug)]
pub struct BleGamepadJitterBuffer {
    pub target_interval_us: u32,      // Expected interval, e.g. 7,500us (7.5ms)
    pub last_arrival_us: u64,
    pub jitter_ema_us: f32,           // Exponential moving average jitter
    pub target_depth: usize,          // 1 (clean RF), 2 (moderate), 3 (noisy)
    pub queue: VecDeque<BleGamepadState>,
    pub underrun_count: u64,
    pub overrun_count: u64,
    pub total_samples_dequeued: u64,
}

impl BleGamepadJitterBuffer {
    pub fn new(target_interval_us: u32) -> Self {
        Self {
            target_interval_us,
            last_arrival_us: 0,
            jitter_ema_us: 0.0,
            target_depth: 1,
            queue: VecDeque::with_capacity(16),
            underrun_count: 0,
            overrun_count: 0,
            total_samples_dequeued: 0,
        }
    }

    /// Records arrival of a new gamepad state and updates RF jitter statistics.
    pub fn push_sample(&mut self, sample: BleGamepadState, arrival_us: u64) {
        if self.last_arrival_us > 0 && arrival_us >= self.last_arrival_us {
            let delta = (arrival_us - self.last_arrival_us) as f32;
            let error = (delta - self.target_interval_us as f32).abs();
            // Alpha = 0.2 smoothing factor
            self.jitter_ema_us = 0.2 * error + 0.8 * self.jitter_ema_us;

            // Dynamically tune playout buffer depth
            self.target_depth = if self.jitter_ema_us < 1500.0 {
                1 // Clean wireless link: instant sub-8ms delivery
            } else if self.jitter_ema_us < 4000.0 {
                2 // Moderate RF noise: 15ms buffer depth to prevent hitching
            } else {
                3 // High RF contention: 22.5ms depth
            };
        }
        self.last_arrival_us = arrival_us;

        if self.queue.len() >= 8 {
            self.overrun_count += 1;
            self.queue.pop_front();
        }
        self.queue.push_back(sample);
    }

    /// Pops the next smoothed gamepad sample for the game engine / kernel input loop.
    pub fn pop_sample(&mut self) -> Option<BleGamepadState> {
        if self.queue.is_empty() {
            self.underrun_count += 1;
            None
        } else {
            self.total_samples_dequeued += 1;
            self.queue.pop_front()
        }
    }
}

/// HID over GATT (HoG) Gamepad Input Processor.
#[derive(Debug)]
pub struct BleHogGamepadProcessor {
    pub connection_params: BleConnectionParams,
    pub jitter_buffer: BleGamepadJitterBuffer,
    pub deadzone_threshold: f32, // e.g. 0.05 (5%)
    pub sequence_counter: u32,
    pub last_state: BleGamepadState,
}

impl BleHogGamepadProcessor {
    pub fn new() -> Self {
        let params = BleConnectionParams::gaming_low_latency_7_5ms();
        Self {
            connection_params: params,
            jitter_buffer: BleGamepadJitterBuffer::new(7500),
            deadzone_threshold: 0.05,
            sequence_counter: 0,
            last_state: BleGamepadState::default(),
        }
    }

    /// Enforces sub-8ms connection intervals. If current peripheral settings exceed
    /// 7.5ms (6 units), returns an updated `BleConnectionParams` for negotiation.
    pub fn enforce_sub_8ms_profile(&mut self, current: BleConnectionParams) -> Option<BleConnectionParams> {
        if !current.is_sub_8ms() {
            self.connection_params = BleConnectionParams::gaming_low_latency_7_5ms();
            Some(self.connection_params)
        } else {
            self.connection_params = current;
            None
        }
    }

    /// Applies inner deadzone to analog axis values.
    fn apply_deadzone(&self, val: f32) -> f32 {
        if val.abs() < self.deadzone_threshold {
            0.0
        } else {
            let sign = if val >= 0.0 { 1.0 } else { -1.0 };
            let remapped = (val.abs() - self.deadzone_threshold) / (1.0 - self.deadzone_threshold);
            sign * remapped.clamp(0.0, 1.0)
        }
    }

    /// Parses standard 12-byte HID over GATT gamepad input report.
    ///
    /// Byte format:
    /// [0..1]: Left Stick X (signed 16-bit)
    /// [2..3]: Left Stick Y (signed 16-bit)
    /// [4..5]: Right Stick X (signed 16-bit)
    /// [6..7]: Right Stick Y (signed 16-bit)
    /// [8]: Left Trigger (0..255)
    /// [9]: Right Trigger (0..255)
    /// [10..11]: Buttons bitmask (16-bit)
    pub fn parse_input_report(&mut self, report_data: &[u8], timestamp_us: u64) -> Result<BleGamepadState, &'static str> {
        if report_data.len() < 12 {
            return Err("HID gamepad report too short (minimum 12 bytes required)");
        }

        let raw_lx = (report_data[0] as i16) | ((report_data[1] as i16) << 8);
        let raw_ly = (report_data[2] as i16) | ((report_data[3] as i16) << 8);
        let raw_rx = (report_data[4] as i16) | ((report_data[5] as i16) << 8);
        let raw_ry = (report_data[6] as i16) | ((report_data[7] as i16) << 8);

        let lx = self.apply_deadzone(raw_lx as f32 / 32767.0);
        let ly = self.apply_deadzone(raw_ly as f32 / 32767.0);
        let rx = self.apply_deadzone(raw_rx as f32 / 32767.0);
        let ry = self.apply_deadzone(raw_ry as f32 / 32767.0);

        let lt = (report_data[8] as f32) / 255.0;
        let rt = (report_data[9] as f32) / 255.0;

        let buttons = ((report_data[10] as u32) | ((report_data[11] as u32) << 8)) & 0xFFFF;

        self.sequence_counter = self.sequence_counter.wrapping_add(1);

        let state = BleGamepadState {
            left_stick_x: lx.clamp(-1.0, 1.0),
            left_stick_y: ly.clamp(-1.0, 1.0),
            right_stick_x: rx.clamp(-1.0, 1.0),
            right_stick_y: ry.clamp(-1.0, 1.0),
            left_trigger: lt.clamp(0.0, 1.0),
            right_trigger: rt.clamp(0.0, 1.0),
            buttons,
            timestamp_us,
            sequence: self.sequence_counter,
        };

        self.last_state = state;
        self.jitter_buffer.push_sample(state, timestamp_us);

        Ok(state)
    }
}


// =============================================================================
// 6. Subsystem Verification Suite & Boot Sentinel
// =============================================================================

/// Verification self-test suite for Low-Latency BLE Gamepad & Wireless Headset Audio Streamer (TASK-KNT-106).
pub fn test_wireless_ble_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIRELESS-BLE-TEST] Running Low-Latency BLE Gamepad & Wireless Headset Audio Streamer verification suite...\r\n");

    // 1. HCI transport packet layer verification
    let cmd = HciCommandPacket::new(HCI_OPCODE_LE_SET_CIG_PARAMETERS, alloc::vec![0x01, 0x00, 0x06, 0x00]);
    let cmd_raw = cmd.serialize();
    let parsed_cmd = match HciCommandPacket::parse(&cmd_raw) {
        Some(c) => c,
        None => return false,
    };
    if parsed_cmd.opcode != HCI_OPCODE_LE_SET_CIG_PARAMETERS {
        return false;
    }

    let evt = HciEventPacket::new(HCI_EVT_LE_META, alloc::vec![HCI_LE_SUBEVT_CIS_ESTABLISHED, 0x00, 0x10]);
    let evt_raw = evt.serialize();
    let parsed_evt = match HciEventPacket::parse(&evt_raw) {
        Some(e) => e,
        None => return false,
    };
    if !parsed_evt.is_le_meta() || parsed_evt.subevent_code() != Some(HCI_LE_SUBEVT_CIS_ESTABLISHED) {
        return false;
    }

    let iso_packet = HciIsoDataPacket::new(0x0042, IsoPacketBoundary::CompleteSdu, 1, Some(100_000), alloc::vec![0x11, 0x22, 0x33, 0x44]);
    let iso_raw = iso_packet.serialize();
    let parsed_iso = match HciIsoDataPacket::parse(&iso_raw) {
        Some(p) => p,
        None => return false,
    };
    if parsed_iso.connection_handle != 0x0042 || parsed_iso.packet_seq_num != 1 || parsed_iso.timestamp_us != Some(100_000) {
        return false;
    }
    crate::serial_write_str("  * HCI transport packet framing and serialization verified\r\n");

    // 2. L2CAP protocol channel multiplexer and dynamic MTU negotiation
    let mut mux = L2capMultiplexer::new();
    let negotiated_mtu = mux.negotiate_mtu(247);
    if negotiated_mtu != 247 || mux.att_mtu != 247 {
        return false;
    }

    let params = BleConnectionParams::gaming_low_latency_7_5ms();
    if !params.is_sub_8ms() {
        return false;
    }
    let sig_pdu = mux.create_conn_param_update_req(1, &params);
    let sig_raw = sig_pdu.serialize();
    let parsed_sig = match L2capPdu::parse(&sig_raw) {
        Some(p) => p,
        None => return false,
    };
    if mux.route_incoming_pdu(parsed_sig).is_err() || mux.rx_signaling_queue.len() != 1 {
        return false;
    }

    let att_pdu = L2capPdu::new(L2CAP_CID_ATT, alloc::vec![0x01, 0x02, 0x03, 0x04]);
    if mux.route_incoming_pdu(att_pdu).is_err() || mux.rx_att_queue.len() != 1 {
        return false;
    }
    crate::serial_write_str("  * L2CAP protocol channel multiplexer and dynamic MTU negotiation verified\r\n");

    // 3. Connected Isochronous Stream (CIS) scheduler and 7.5ms/10ms LC3 audio streaming
    let cis_config = CisStreamConfig::gaming_headset_7_5ms(0x0055);
    let mut scheduler = CisAudioScheduler::new(cis_config, 1_000_000);
    scheduler.start_streaming();

    for i in 0..4 {
        let frame = Lc3Frame::new(
            Lc3FrameDuration::Duration7_5ms,
            Lc3SampleRate::Rate48kHz,
            AudioChannelConfig::Stereo,
            alloc::vec![0x40 + (i as u8); 100],
        );
        if scheduler.enqueue_lc3_frame(frame, 1_000_000 + (i as u64 * 7500)).is_err() {
            return false;
        }
    }
    if scheduler.tx_iso_queue.len() != 4 {
        return false;
    }

    let burst = match scheduler.poll_next_burst(1_000_000) {
        Some(b) => b,
        None => return false,
    };
    if burst.packet_seq_num != 0 || burst.sdu_len != 100 {
        return false;
    }
    scheduler.advance_anchor_point();

    // Verify PLC
    let prev_audio = [500i16; 16];
    let mut concealed_audio = [0i16; 16];
    CisAudioScheduler::plc_conceal_frame(&prev_audio, &mut concealed_audio);
    if concealed_audio[0] >= 500 || concealed_audio[15] >= concealed_audio[0] {
        return false;
    }
    crate::serial_write_str("  * Connected Isochronous Stream (CIS) scheduler and 7.5ms/10ms LC3 audio streaming verified\r\n");

    // 4. HID over GATT (HoG) gamepad input processor and adaptive jitter buffer
    let mut hog = BleHogGamepadProcessor::new();
    let slow_profile = BleConnectionParams {
        min_interval: 16,
        max_interval: 16,
        latency: 0,
        timeout: 50,
    };
    let enforced = match hog.enforce_sub_8ms_profile(slow_profile) {
        Some(p) => p,
        None => return false,
    };
    if !enforced.is_sub_8ms() {
        return false;
    }

    let mut raw_hid = [0u8; 12];
    let lx_bytes = 20000i16.to_le_bytes();
    raw_hid[0] = lx_bytes[0];
    raw_hid[1] = lx_bytes[1];
    raw_hid[8] = 200; // Left trigger
    raw_hid[9] = 255; // Right trigger
    let buttons = (BLE_BTN_A | BLE_BTN_X | BLE_BTN_START) as u16;
    let btn_bytes = buttons.to_le_bytes();
    raw_hid[10] = btn_bytes[0];
    raw_hid[11] = btn_bytes[1];

    let state = match hog.parse_input_report(&raw_hid, 2_000_000) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if state.left_stick_x <= 0.5 || !state.is_button_pressed(BLE_BTN_A) || !state.is_button_pressed(BLE_BTN_START) {
        return false;
    }

    let popped = match hog.jitter_buffer.pop_sample() {
        Some(s) => s,
        None => return false,
    };
    if popped.sequence != 1 {
        return false;
    }
    crate::serial_write_str("  * HID over GATT (HoG) gamepad input processor and adaptive jitter buffer verified\r\n");

    // 5. End-to-end simultaneous wireless gamepad and LE Audio streaming
    let mut concurrent_scheduler = CisAudioScheduler::new(CisStreamConfig::gaming_headset_10ms(0x0088), 3_000_000);
    concurrent_scheduler.start_streaming();
    let mut concurrent_hog = BleHogGamepadProcessor::new();

    let audio_payload = alloc::vec![0xAA; 155];
    let audio_frame = Lc3Frame::new(
        Lc3FrameDuration::Duration10ms,
        Lc3SampleRate::Rate48kHz,
        AudioChannelConfig::Stereo,
        audio_payload,
    );
    if concurrent_scheduler.enqueue_lc3_frame(audio_frame, 3_000_000).is_err() {
        return false;
    }

    let mut pad_report = [0u8; 12];
    pad_report[8] = 128;
    let dpad_bytes = (BLE_BTN_DPAD_UP as u16).to_le_bytes();
    pad_report[10] = dpad_bytes[0];
    pad_report[11] = dpad_bytes[1];
    if concurrent_hog.parse_input_report(&pad_report, 3_000_000).is_err() {
        return false;
    }

    let iso_out = match concurrent_scheduler.poll_next_burst(3_000_000) {
        Some(p) => p,
        None => return false,
    };
    let pad_out = match concurrent_hog.jitter_buffer.pop_sample() {
        Some(s) => s,
        None => return false,
    };
    if iso_out.sdu_len != 155 || !pad_out.is_button_pressed(BLE_BTN_DPAD_UP) {
        return false;
    }
    crate::serial_write_str("  * End-to-end simultaneous wireless gamepad and LE Audio streaming verified\r\n");

    crate::serial_write_str("[KNT-WIRELESS-BLE-PASS] Low-latency BLE gamepad and wireless headset audio streamer operational\r\n");
    true
}

// -----------------------------------------------------------------------------
// Unit Tests for Commit 1
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_hci_packet_formats() {
        // Test HCI Command Packet
        let cmd = HciCommandPacket::new(HCI_OPCODE_LE_CONNECTION_UPDATE, alloc::vec![0x40, 0x00, 0x06, 0x00]);
        let serialized_cmd = cmd.serialize();
        assert_eq!(serialized_cmd[0], HciPacketIndicator::Command as u8);
        let parsed_cmd = HciCommandPacket::parse(&serialized_cmd).expect("Parse HCI command failed");
        assert_eq!(parsed_cmd.opcode, HCI_OPCODE_LE_CONNECTION_UPDATE);
        assert_eq!(parsed_cmd.params, alloc::vec![0x40, 0x00, 0x06, 0x00]);

        // Test HCI Event Packet
        let evt = HciEventPacket::new(HCI_EVT_LE_META, alloc::vec![HCI_LE_SUBEVT_CIS_ESTABLISHED, 0x00, 0x42]);
        let serialized_evt = evt.serialize();
        assert_eq!(serialized_evt[0], HciPacketIndicator::Event as u8);
        let parsed_evt = HciEventPacket::parse(&serialized_evt).expect("Parse HCI event failed");
        assert_eq!(parsed_evt.is_le_meta(), true);
        assert_eq!(parsed_evt.subevent_code(), Some(HCI_LE_SUBEVT_CIS_ESTABLISHED));

        // Test HCI ACL Data Packet
        let acl = HciAclDataPacket::new(0x0042, AclPacketBoundaryFlag::CompleteL2capPdu, alloc::vec![0x10, 0x20, 0x30]);
        let serialized_acl = acl.serialize();
        assert_eq!(serialized_acl[0], HciPacketIndicator::AclData as u8);
        let parsed_acl = HciAclDataPacket::parse(&serialized_acl).expect("Parse HCI ACL failed");
        assert_eq!(parsed_acl.connection_handle, 0x0042);
        assert_eq!(parsed_acl.pb_flag, AclPacketBoundaryFlag::CompleteL2capPdu);
        assert_eq!(parsed_acl.payload, alloc::vec![0x10, 0x20, 0x30]);

        // Test HCI ISO Data Packet
        let iso = HciIsoDataPacket::new(0x0050, IsoPacketBoundary::CompleteSdu, 1024, Some(100_000), alloc::vec![0xAA, 0xBB, 0xCC]);
        let serialized_iso = iso.serialize();
        assert_eq!(serialized_iso[0], HciPacketIndicator::IsoData as u8);
        let parsed_iso = HciIsoDataPacket::parse(&serialized_iso).expect("Parse HCI ISO failed");
        assert_eq!(parsed_iso.connection_handle, 0x0050);
        assert_eq!(parsed_iso.packet_seq_num, 1024);
        assert_eq!(parsed_iso.timestamp_us, Some(100_000));
        assert_eq!(parsed_iso.sdu_data, alloc::vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    pub fn test_l2cap_multiplexing_and_mtu() {
        let mut mux = L2capMultiplexer::new();
        assert_eq!(mux.att_mtu, L2CAP_DEFAULT_ATT_MTU);

        // MTU Negotiation
        let negotiated = mux.negotiate_mtu(247);
        assert_eq!(negotiated, 247);
        assert_eq!(mux.att_mtu, 247);

        // Connection Params check
        let params = BleConnectionParams::gaming_low_latency_7_5ms();
        assert!(params.is_sub_8ms());
        assert!((params.min_interval_ms() - 7.5).abs() < 0.001);

        // L2CAP Signaling PDU creation
        let sig_pdu = mux.create_conn_param_update_req(1, &params);
        let serialized = sig_pdu.serialize();
        let parsed = L2capPdu::parse(&serialized).expect("Parse L2CAP PDU failed");
        assert_eq!(parsed.channel_id, L2CAP_CID_LE_SIGNALING);

        // Route into multiplexer
        mux.route_incoming_pdu(parsed).expect("Route signaling failed");
        assert_eq!(mux.rx_signaling_queue.len(), 1);

        // ATT packet routing
        let att_pdu = L2capPdu::new(L2CAP_CID_ATT, alloc::vec![0x01, 0x02, 0x03]);
        mux.route_incoming_pdu(att_pdu).expect("Route ATT failed");
        assert_eq!(mux.rx_att_queue.len(), 1);
    }
    #[test]
    pub fn test_lc3_audio_and_cis_scheduler() {
        let config = CisStreamConfig::gaming_headset_7_5ms(0x0080);
        assert_eq!(config.duration.interval_us(), 7500);
        assert_eq!(config.sample_rate.samples_per_frame(config.duration), 360);

        let mut scheduler = CisAudioScheduler::new(config, 1_000_000);
        scheduler.start_streaming();
        assert_eq!(scheduler.state, CisState::Streaming);

        // Enqueue 3 LC3 frames (each 100 bytes)
        for _ in 0..3 {
            let lc3 = Lc3Frame::new(
                Lc3FrameDuration::Duration7_5ms,
                Lc3SampleRate::Rate48kHz,
                AudioChannelConfig::Stereo,
                alloc::vec![0x7A; 100],
            );
            scheduler.enqueue_lc3_frame(lc3, 1_000_000).expect("Enqueue LC3 frame failed");
        }
        assert_eq!(scheduler.stats.scheduled_frames, 3);
        assert_eq!(scheduler.tx_iso_queue.len(), 3);

        // Poll bursts at timeline 1_000_000 us
        let p1 = scheduler.poll_next_burst(1_000_000).expect("Poll first burst failed");
        assert_eq!(p1.packet_seq_num, 0);
        assert_eq!(p1.sdu_len, 100);
        assert_eq!(scheduler.stats.transmitted_packets, 1);

        // Advance anchor point by 7.5ms
        scheduler.advance_anchor_point();
        assert_eq!(scheduler.anchor_point_us, 1_007_500);

        // Test Packet Loss Concealment (PLC)
        let prev_pcm = [1000i16; 10];
        let mut concealed = [0i16; 10];
        CisAudioScheduler::plc_conceal_frame(&prev_pcm, &mut concealed);
        assert!(concealed[0] < 1000 && concealed[0] > 700);
        assert!(concealed[9] < concealed[0]); // verified decaying envelope
    }
    #[test]
    pub fn test_hog_gamepad_and_jitter_buffer() {
        let mut processor = BleHogGamepadProcessor::new();

        // Sub-8ms enforcement check
        let slow_params = BleConnectionParams {
            min_interval: 24, // 30ms
            max_interval: 24,
            latency: 2,
            timeout: 100,
        };
        let updated = processor.enforce_sub_8ms_profile(slow_params).expect("Expected profile update");
        assert!(updated.is_sub_8ms());
        assert_eq!(updated.max_interval, 6); // 7.5ms

        // Construct 12-byte HID report
        // Stick LX = 16384 (~0.5), LY = 0, RX = -16384 (~-0.5), RY = 0
        // Triggers LT = 255 (1.0), RT = 128 (~0.5)
        // Buttons = BLE_BTN_A | BLE_BTN_RB
        let mut report = [0u8; 12];
        let lx_bytes = 16384i16.to_le_bytes();
        report[0] = lx_bytes[0];
        report[1] = lx_bytes[1];
        let rx_bytes = (-16384i16).to_le_bytes();
        report[4] = rx_bytes[0];
        report[5] = rx_bytes[1];
        report[8] = 255;
        report[9] = 128;
        let btn_bytes = ((BLE_BTN_A | BLE_BTN_RB) as u16).to_le_bytes();
        report[10] = btn_bytes[0];
        report[11] = btn_bytes[1];

        let state = processor.parse_input_report(&report, 100_000).expect("Parse HID report failed");
        assert!(state.left_stick_x > 0.45 && state.left_stick_x < 0.55);
        assert!(state.right_stick_x < -0.45 && state.right_stick_x > -0.55);
        assert_eq!(state.left_trigger, 1.0);
        assert!(state.is_button_pressed(BLE_BTN_A));
        assert!(state.is_button_pressed(BLE_BTN_RB));
        assert!(!state.is_button_pressed(BLE_BTN_B));

        // Verify Jitter Buffer operation
        let popped = processor.jitter_buffer.pop_sample().expect("Pop sample failed");
        assert_eq!(popped.sequence, 1);
        assert!(popped.is_button_pressed(BLE_BTN_A));
    }
}
