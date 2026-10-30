//! DisplayPort Configuration Data (DPCD) Parser & AUX Channel Engine.
//!
//! Implements VESA DisplayPort 1.4+ AUX Channel & Receiver Configuration (DPCD):
//! - 1Mbps Manchester AUX Channel transaction engine (Native Read/Write, I2C over AUX)
//! - Standard DPCD register map definitions (0x00000..0x00207)
//! - Receiver capability parsing (DPCD revision, Max Link Rate, Max Lane Count, Enhanced Framing)
//! - Link rate definitions: RBR (1.62 Gbps), HBR (2.7 Gbps), HBR2 (5.4 Gbps), HBR3 (8.1 Gbps)

#![allow(dead_code)]

extern crate alloc;

// -----------------------------------------------------------------------------
// AUX Channel Command & Reply Definitions
// -----------------------------------------------------------------------------

/// AUX Channel Command types (VESA DP specification).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxCommand {
    I2CWrite = 0x0,
    I2CRead = 0x1,
    I2CWriteStatus = 0x2,
    I2CMot = 0x4, // Middle-of-Transaction flag
    NativeWrite = 0x8,
    NativeRead = 0x9,
}

/// AUX Channel Reply Status Codes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxReplyStatus {
    Ack = 0x0,
    Nack = 0x1,
    Defer = 0x2,
    I2CNack = 0x4,
    I2CDefer = 0x8,
}

/// A single AUX Channel request packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuxTransactionRequest {
    pub command: AuxCommand,
    pub address: u32,
    pub length: u8,
    pub data: [u8; 16],
}

impl AuxTransactionRequest {
    pub const fn new_native_read(address: u32, length: u8) -> Self {
        Self {
            command: AuxCommand::NativeRead,
            address,
            length,
            data: [0; 16],
        }
    }

    pub fn new_native_write(address: u32, data_slice: &[u8]) -> Self {
        let mut data = [0u8; 16];
        let len = data_slice.len().min(16) as u8;
        data[..len as usize].copy_from_slice(&data_slice[..len as usize]);
        Self {
            command: AuxCommand::NativeWrite,
            address,
            length: len,
            data,
        }
    }
}

/// A single AUX Channel reply packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuxTransactionReply {
    pub status: AuxReplyStatus,
    pub length: u8,
    pub data: [u8; 16],
}

// -----------------------------------------------------------------------------
// DisplayPort Link Rates
// -----------------------------------------------------------------------------

/// DisplayPort physical link signaling rates.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DpLinkRate {
    /// Reduced Bit Rate: 1.62 Gbps per lane.
    Rbr = 0x06,
    /// High Bit Rate: 2.70 Gbps per lane.
    Hbr = 0x0A,
    /// High Bit Rate 2: 5.40 Gbps per lane.
    Hbr2 = 0x14,
    /// High Bit Rate 3: 8.10 Gbps per lane.
    Hbr3 = 0x1E,
}

impl DpLinkRate {
    pub const fn from_raw(val: u8) -> Option<Self> {
        match val {
            0x06 => Some(Self::Rbr),
            0x0A => Some(Self::Hbr),
            0x14 => Some(Self::Hbr2),
            0x1E => Some(Self::Hbr3),
            _ => None,
        }
    }

    /// Link signaling bandwidth per lane in Megabits per second.
    #[inline(always)]
    pub const fn bandwidth_mbps(&self) -> u32 {
        match self {
            Self::Rbr => 1620,
            Self::Hbr => 2700,
            Self::Hbr2 => 5400,
            Self::Hbr3 => 8100,
        }
    }
}

// -----------------------------------------------------------------------------
// Standard DPCD Register Addresses
// -----------------------------------------------------------------------------

pub const DPCD_REV: u32 = 0x00000;
pub const DPCD_MAX_LINK_RATE: u32 = 0x00001;
pub const DPCD_MAX_LANE_COUNT: u32 = 0x00002;
pub const DPCD_MAX_DOWNSPREAD: u32 = 0x00003;
pub const DPCD_NORP: u32 = 0x00004;
pub const DPCD_DOWNSTREAMPORT_PRESENT: u32 = 0x00005;
pub const DPCD_MAIN_LINK_CHANNEL_CODING: u32 = 0x00006;
pub const DPCD_TRAINING_AUX_RD_INTERVAL: u32 = 0x0000E;

pub const DPCD_LINK_BW_SET: u32 = 0x00100;
pub const DPCD_LANE_COUNT_SET: u32 = 0x00101;
pub const DPCD_TRAINING_PATTERN_SET: u32 = 0x00102;
pub const DPCD_TRAINING_LANE0_SET: u32 = 0x00103;

pub const DPCD_SINK_STATUS: u32 = 0x00200;
pub const DPCD_LANE0_1_STATUS: u32 = 0x00202;
pub const DPCD_LANE2_3_STATUS: u32 = 0x00203;
pub const DPCD_LANE_ALIGN_STATUS_UPDATED: u32 = 0x00204;
pub const DPCD_ADJUST_REQUEST_LANE0_1: u32 = 0x00206;
pub const DPCD_ADJUST_REQUEST_LANE2_3: u32 = 0x00207;

// -----------------------------------------------------------------------------
// DPCD Receiver Capabilities
// -----------------------------------------------------------------------------

/// Parsed sink / receiver capabilities from DPCD addresses 0x00000..0x0000F.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpcdReceiverCaps {
    pub dpcd_rev: u8,
    pub max_link_rate: DpLinkRate,
    pub max_lane_count: u8,
    pub enhanced_framing: bool,
    pub tps3_supported: bool,
    pub downstream_port_present: bool,
}

impl DpcdReceiverCaps {
    /// Parses raw DPCD receiver capability block (bytes 0x00000..0x0000E).
    pub fn parse(dpcd: &[u8]) -> Result<Self, &'static str> {
        if dpcd.len() < 16 {
            return Err("DPCD capability buffer too small");
        }

        let rev = dpcd[DPCD_REV as usize];
        let max_rate_raw = dpcd[DPCD_MAX_LINK_RATE as usize];
        let max_rate = DpLinkRate::from_raw(max_rate_raw).unwrap_or(DpLinkRate::Hbr2);

        let lane_byte = dpcd[DPCD_MAX_LANE_COUNT as usize];
        let max_lanes = (lane_byte & 0x1F).min(4);
        let enhanced_framing = (lane_byte & (1 << 7)) != 0;
        let tps3_supported = (lane_byte & (1 << 6)) != 0;

        let dport_present = (dpcd[DPCD_DOWNSTREAMPORT_PRESENT as usize] & 0x01) != 0;

        Ok(Self {
            dpcd_rev: rev,
            max_link_rate: max_rate,
            max_lane_count: max_lanes,
            enhanced_framing,
            tps3_supported,
            downstream_port_present: dport_present,
        })
    }

    /// Maximum raw display bandwidth across all lanes in Megabits per second.
    #[inline(always)]
    pub fn max_raw_bandwidth_mbps(&self) -> u32 {
        self.max_link_rate.bandwidth_mbps() * (self.max_lane_count as u32)
    }
}

// -----------------------------------------------------------------------------
// AUX Channel Engine
// -----------------------------------------------------------------------------

/// Controller simulating 1Mbps Manchester AUX channel transactions to the DPCD register bank.
#[derive(Debug)]
pub struct AuxChannelEngine {
    dpcd_memory: [u8; 512],
    pub transactions_executed: u64,
}

impl AuxChannelEngine {
    pub fn new() -> Self {
        let mut dpcd = [0u8; 512];
        // Default DPCD 1.4 sink configuration (e.g. 4K60p dock display)
        dpcd[DPCD_REV as usize] = 0x14; // DPCD 1.4
        dpcd[DPCD_MAX_LINK_RATE as usize] = DpLinkRate::Hbr2 as u8; // 5.40 Gbps
        dpcd[DPCD_MAX_LANE_COUNT as usize] = 4 | (1 << 7) | (1 << 6); // 4 lanes, Enhanced Framing, TPS3
        dpcd[DPCD_DOWNSTREAMPORT_PRESENT as usize] = 0x01; // Display attached

        Self {
            dpcd_memory: dpcd,
            transactions_executed: 0,
        }
    }

    /// Dispatches an AUX Channel transaction.
    pub fn process_transaction(&mut self, req: &AuxTransactionRequest) -> AuxTransactionReply {
        self.transactions_executed += 1;
        let addr = req.address as usize;
        let len = (req.length as usize).min(16);

        if addr + len > self.dpcd_memory.len() {
            return AuxTransactionReply {
                status: AuxReplyStatus::Nack,
                length: 0,
                data: [0; 16],
            };
        }

        match req.command {
            AuxCommand::NativeRead => {
                let mut reply_data = [0u8; 16];
                reply_data[..len].copy_from_slice(&self.dpcd_memory[addr..addr + len]);
                AuxTransactionReply {
                    status: AuxReplyStatus::Ack,
                    length: len as u8,
                    data: reply_data,
                }
            }
            AuxCommand::NativeWrite => {
                self.dpcd_memory[addr..addr + len].copy_from_slice(&req.data[..len]);
                AuxTransactionReply {
                    status: AuxReplyStatus::Ack,
                    length: len as u8,
                    data: [0; 16],
                }
            }
            _ => AuxTransactionReply {
                status: AuxReplyStatus::Nack,
                length: 0,
                data: [0; 16],
            },
        }
    }

    /// Convenience helper to read receiver capabilities over the AUX channel.
    pub fn query_receiver_caps(&mut self) -> Result<DpcdReceiverCaps, &'static str> {
        let req = AuxTransactionRequest::new_native_read(0x00000, 16);
        let reply = self.process_transaction(&req);
        if reply.status != AuxReplyStatus::Ack {
            return Err("AUX transaction failed when reading DPCD caps");
        }
        DpcdReceiverCaps::parse(&reply.data)
    }

    /// Convenience helper to write link configuration registers (0x00100..0x00102).
    pub fn set_link_config(&mut self, rate: DpLinkRate, lanes: u8) -> Result<(), &'static str> {
        let cfg = [rate as u8, lanes & 0x1F, 0x00];
        let req = AuxTransactionRequest::new_native_write(DPCD_LINK_BW_SET, &cfg);
        let reply = self.process_transaction(&req);
        if reply.status != AuxReplyStatus::Ack {
            return Err("Failed to set link configuration over AUX");
        }
        Ok(())
    }

    /// Direct access to simulated register memory for link training status updates.
    #[inline(always)]
    pub fn set_register(&mut self, address: u32, value: u8) {
        if (address as usize) < self.dpcd_memory.len() {
            self.dpcd_memory[address as usize] = value;
        }
    }

    #[inline(always)]
    pub fn get_register(&self, address: u32) -> u8 {
        if (address as usize) < self.dpcd_memory.len() {
            self.dpcd_memory[address as usize]
        } else {
            0
        }
    }
}
