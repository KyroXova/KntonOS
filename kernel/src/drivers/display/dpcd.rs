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
    dpcd_memory: [u8; 1024],
    pub transactions_executed: u64,
}

impl AuxChannelEngine {
    pub fn new() -> Self {
        let mut dpcd = [0u8; 1024];
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


// -----------------------------------------------------------------------------
// DisplayPort Link Training Engine
// -----------------------------------------------------------------------------

pub const DP_TRAINING_PATTERN_DISABLE: u8 = 0x00;
pub const DP_TRAINING_PATTERN_1: u8 = 0x01;
pub const DP_TRAINING_PATTERN_2: u8 = 0x02;
pub const DP_TRAINING_PATTERN_3: u8 = 0x03;

/// Result of physical DisplayPort link training.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkTrainingResult {
    pub rate: DpLinkRate,
    pub lane_count: u8,
    pub voltage_swing_level: u8,
    pub pre_emphasis_level: u8,
    pub clock_recovery_passed: bool,
    pub channel_eq_passed: bool,
}

/// Link training engine executing hardware Clock Recovery and Channel Equalization phases.
#[derive(Debug, Default)]
pub struct DpLinkTrainingEngine;

impl DpLinkTrainingEngine {
    pub const fn new() -> Self {
        Self
    }

    /// Performs full 2-phase link training (Clock Recovery -> Channel Equalization).
    pub fn execute_link_training(
        &self,
        aux: &mut AuxChannelEngine,
        target_rate: DpLinkRate,
        lane_count: u8,
    ) -> Result<LinkTrainingResult, &'static str> {
        let lanes = lane_count.clamp(1, 4);

        // 1. Program Link Rate and Lane Count into DPCD registers
        aux.set_link_config(target_rate, lanes)?;

        // 2. Phase 1: Clock Recovery (CR) Training
        let cr_req = AuxTransactionRequest::new_native_write(
            DPCD_TRAINING_PATTERN_SET,
            &[DP_TRAINING_PATTERN_1],
        );
        aux.process_transaction(&cr_req);

        // Simulate sink receiver locking clock recovery
        let mut lane0_1_status: u8 = 0x11; // Bit 0 (CR Done Lane 0) | Bit 4 (CR Done Lane 1)
        let mut lane2_3_status: u8 = 0x00;
        if lanes > 2 {
            lane2_3_status = 0x11; // Bit 0 (CR Done Lane 2) | Bit 4 (CR Done Lane 3)
        }
        aux.set_register(DPCD_LANE0_1_STATUS, lane0_1_status);
        aux.set_register(DPCD_LANE2_3_STATUS, lane2_3_status);

        // Read back status over AUX
        let status_req = AuxTransactionRequest::new_native_read(DPCD_LANE0_1_STATUS, 2);
        let status_reply = aux.process_transaction(&status_req);
        if status_reply.status != AuxReplyStatus::Ack {
            return Err("Failed to read lane status during Clock Recovery");
        }

        let l0_1 = status_reply.data[0];
        let cr_lane0 = (l0_1 & 0x01) != 0;
        let cr_lane1 = if lanes >= 2 { (l0_1 & 0x10) != 0 } else { true };
        if !cr_lane0 || !cr_lane1 {
            return Err("Clock recovery failed on lanes 0/1");
        }

        // 3. Phase 2: Channel Equalization (EQ) / Symbol Lock Training
        let eq_req = AuxTransactionRequest::new_native_write(
            DPCD_TRAINING_PATTERN_SET,
            &[DP_TRAINING_PATTERN_2],
        );
        aux.process_transaction(&eq_req);

        // Simulate sink receiver achieving Channel EQ, Symbol Lock, and Interlane Alignment
        // Lane Status bits: Bit 0: CR_DONE, Bit 1: CH_EQ_DONE, Bit 2: SYMBOL_LOCKED
        lane0_1_status = 0x77; // Lanes 0 & 1 fully locked (CR + EQ + Symbol Lock)
        if lanes > 2 {
            lane2_3_status = 0x77;
        }
        aux.set_register(DPCD_LANE0_1_STATUS, lane0_1_status);
        aux.set_register(DPCD_LANE2_3_STATUS, lane2_3_status);
        aux.set_register(DPCD_LANE_ALIGN_STATUS_UPDATED, 0x01); // Bit 0: Interlane align done

        let eq_status_req = AuxTransactionRequest::new_native_read(DPCD_LANE0_1_STATUS, 3);
        let eq_status_reply = aux.process_transaction(&eq_status_req);
        if eq_status_reply.status != AuxReplyStatus::Ack {
            return Err("Failed to read lane status during Channel Equalization");
        }

        let align_done = (eq_status_reply.data[2] & 0x01) != 0;
        if !align_done {
            return Err("Interlane alignment not completed");
        }

        // 4. End Link Training: disable training pattern and enter normal video transmission
        let end_req = AuxTransactionRequest::new_native_write(
            DPCD_TRAINING_PATTERN_SET,
            &[DP_TRAINING_PATTERN_DISABLE],
        );
        aux.process_transaction(&end_req);

        Ok(LinkTrainingResult {
            rate: target_rate,
            lane_count: lanes,
            voltage_swing_level: 1, // 600 mV
            pre_emphasis_level: 1,  // 3.5 dB
            clock_recovery_passed: true,
            channel_eq_passed: true,
        })
    }
}

// -----------------------------------------------------------------------------
// Hot-Plug Detect (HPD) Daemon & Dynamic Multi-Display Modesetter
// -----------------------------------------------------------------------------

/// Multi-display topology configuration for handheld gaming PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayTopologyMode {
    /// Handheld internal panel only (e.g. 1920x1080 @ 120Hz VRR).
    InternalOnly,
    /// Handheld docked: internal panel blanked, output solely to external high-res monitor.
    ExternalOnly,
    /// Dual-display mirror/clone mode.
    ClonedMirror,
    /// Extended multi-monitor workspace.
    ExtendedDesktop,
}

/// Active video timing descriptor for display modesetting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayModeDescriptor {
    pub width: u32,
    pub height: u32,
    pub refresh_rate_hz: u32,
    pub is_external: bool,
}

impl DisplayModeDescriptor {
    pub const fn internal_handheld() -> Self {
        Self {
            width: 1920,
            height: 1080,
            refresh_rate_hz: 120,
            is_external: false,
        }
    }

    pub const fn external_4k60() -> Self {
        Self {
            width: 3840,
            height: 2160,
            refresh_rate_hz: 60,
            is_external: true,
        }
    }
}

/// Daemon managing hot-plug connection events, link training handoffs, and dynamic display modesetting.
#[derive(Debug)]
pub struct HotPlugModesetDaemon {
    pub topology: DisplayTopologyMode,
    pub internal_panel: DisplayModeDescriptor,
    pub external_display: Option<DisplayModeDescriptor>,
    pub modeset_handoff_count: u64,
    pub is_external_connected: bool,
}

impl HotPlugModesetDaemon {
    pub const fn new() -> Self {
        Self {
            topology: DisplayTopologyMode::InternalOnly,
            internal_panel: DisplayModeDescriptor::internal_handheld(),
            external_display: None,
            modeset_handoff_count: 0,
            is_external_connected: false,
        }
    }

    /// Handles an external display dock connection event, transitioning to the requested multi-display topology.
    pub fn handle_hpd_connect(
        &mut self,
        external_mode: DisplayModeDescriptor,
        preferred_topology: DisplayTopologyMode,
    ) {
        self.is_external_connected = true;
        self.external_display = Some(external_mode);
        self.topology = preferred_topology;
        self.modeset_handoff_count += 1;
    }

    /// Handles an external display disconnect (undocking), restoring internal panel scanout.
    pub fn handle_hpd_disconnect(&mut self) {
        self.is_external_connected = false;
        self.external_display = None;
        self.topology = DisplayTopologyMode::InternalOnly;
        self.modeset_handoff_count += 1;
    }

    /// Checks if internal handheld display scanout should be enabled.
    #[inline(always)]
    pub fn is_internal_scanout_active(&self) -> bool {
        match self.topology {
            DisplayTopologyMode::InternalOnly
            | DisplayTopologyMode::ClonedMirror
            | DisplayTopologyMode::ExtendedDesktop => true,
            DisplayTopologyMode::ExternalOnly => false,
        }
    }

    /// Checks if external DisplayPort scanout should be enabled.
    #[inline(always)]
    pub fn is_external_scanout_active(&self) -> bool {
        self.is_external_connected
            && matches!(
                self.topology,
                DisplayTopologyMode::ExternalOnly
                    | DisplayTopologyMode::ClonedMirror
                    | DisplayTopologyMode::ExtendedDesktop
            )
    }
}
