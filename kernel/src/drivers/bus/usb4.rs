//! Unified Handheld PC USB4/Thunderbolt 4 Dynamic PCIe Tunneling & Hot-Plug eGPU Governor.
//!
//! Provides bare-metal transport and bus routing for external graphics enclosures (eGPUs)
//! over 40 Gbps USB4/Thunderbolt 4 links featuring:
//! - USB4 Router Configuration Spaces and Transport Adapter Descriptors.
//! - Dynamic PCIe Gen 4x4 tunneling with credit-based flow control.
//! - Dynamic bandwidth balancing between DisplayPort video and PCIe graphics/storage payloads.
//! - Surprise hot-unplug safety barrier intercepting sudden cable disconnects with 0xFFFFFFFF MMIO fallback.

/// USB4 and Thunderbolt 4 Router and Adapter Configuration Space registers.
pub mod usb4_registers {
    /// Router Vendor & Device ID register.
    pub const ROUTER_DEV_VENDOR_ID: u8 = 0x00;
    /// Router revision and protocol version register.
    pub const ROUTER_REVISION: u8 = 0x04;
    /// Router global control and topology initialization.
    pub const ROUTER_CONTROL: u8 = 0x08;
    /// Router status and physical link state register.
    pub const ROUTER_STATUS: u8 = 0x0C;

    /// Adapter Configuration Space: Protocol Adapter Type & Capabilities.
    pub const ADAPTER_TYPE_REG: u8 = 0x10;
    /// Adapter Configuration Space: Credit allocations and buffer depth.
    pub const ADAPTER_CREDIT_REG: u8 = 0x14;
    /// Adapter Configuration Space: Configured link rate and lane width.
    pub const ADAPTER_LINK_RATE_REG: u8 = 0x18;

    /// PCIe Tunnel Adapter Control Register.
    pub const PCIE_TUNNEL_CTRL: u8 = 0x30;
    /// PCIe Tunnel Adapter Status Register.
    pub const PCIE_TUNNEL_STATUS: u8 = 0x34;
    /// DisplayPort Tunnel Control Register.
    pub const DP_TUNNEL_CTRL: u8 = 0x38;

    // Router Control & Status bitflags
    pub const ROUTER_CTRL_LINK_ENABLE: u32 = 1 << 0;
    pub const ROUTER_CTRL_PCIE_TUNNEL_EN: u32 = 1 << 1;
    pub const ROUTER_CTRL_DP_TUNNEL_EN: u32 = 1 << 2;
    pub const ROUTER_STATUS_LINK_40G_ACTIVE: u32 = 1 << 0;
    pub const ROUTER_STATUS_SURPRISE_REMOVAL: u32 = 1 << 1;
    pub const ROUTER_STATUS_CREDIT_EXHAUSTED: u32 = 1 << 2;
}

/// Identifies the protocol persona mapped to a USB4 transport adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolAdapterType {
    /// Inactive or unconfigured physical adapter.
    Unused,
    /// Physical Link (Lane 0 / Lane 1 Dual-Lane PHY).
    LaneAdapter,
    /// PCIe Upstream Adapter (facing Host Root Complex).
    PcieUpstream,
    /// PCIe Downstream Adapter (facing external eGPU endpoint).
    PcieDownstream,
    /// DisplayPort In Adapter (capturing video stream from GPU).
    DisplayPortIn,
    /// DisplayPort Out Adapter (driving external display connected to eGPU).
    DisplayPortOut,
    /// USB3 xHCI SuperSpeed Tunnel Adapter.
    Usb3Tunnel,
}

/// USB4 transport adapter descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usb4AdapterDescriptor {
    /// Physical or logical adapter index (0..63).
    pub adapter_id: u8,
    /// Protocol adapter persona.
    pub adapter_type: ProtocolAdapterType,
    /// Maximum credit capacity for flow control.
    pub max_credits: u16,
    /// Currently available flow-control transmit credits.
    pub available_credits: u16,
    /// Allocated physical bandwidth in Megabits per second (Mbps).
    pub allocated_bandwidth_mbps: u32,
    /// True if the adapter is enabled and tunneling packets.
    pub enabled: bool,
}

impl Usb4AdapterDescriptor {
    /// Creates a new adapter descriptor.
    pub const fn new(adapter_id: u8, adapter_type: ProtocolAdapterType, max_credits: u16) -> Self {
        Self {
            adapter_id,
            adapter_type,
            max_credits,
            available_credits: max_credits,
            allocated_bandwidth_mbps: 0,
            enabled: false,
        }
    }
}

/// Packet encapsulation type tunneled across USB4 fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunneledPacketType {
    /// PCIe Transaction Layer Packet (TLP) memory read/write or completion.
    PcieTlp,
    /// DisplayPort Main Link or Secondary Stream video packet.
    DisplayPort,
    /// Native USB4 Router Control packet.
    Control,
}

/// Tunneled Transport Packet framing over USB4 link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usb4TunneledPacket {
    /// Path routing Hop ID (identifies virtual channel and destination adapter).
    pub hop_id: u16,
    /// Packet protocol type.
    pub packet_type: TunneledPacketType,
    /// Payload length in 32-bit double words (DW).
    pub length_dw: u16,
    /// Credit sequence counter (0..255).
    pub sequence_id: u8,
    /// Raw 32-bit header payload preview (e.g. PCIe TLP Header DW0).
    pub tlp_dw0: u32,
}

impl Usb4TunneledPacket {
    /// Creates a PCIe TLP tunneled packet.
    pub const fn new_pcie_tlp(hop_id: u16, length_dw: u16, sequence_id: u8, tlp_dw0: u32) -> Self {
        Self {
            hop_id,
            packet_type: TunneledPacketType::PcieTlp,
            length_dw,
            sequence_id,
            tlp_dw0,
        }
    }

    /// Creates a DisplayPort tunneled video stream packet.
    pub const fn new_dp_packet(hop_id: u16, length_dw: u16, sequence_id: u8) -> Self {
        Self {
            hop_id,
            packet_type: TunneledPacketType::DisplayPort,
            length_dw,
            sequence_id,
            tlp_dw0: 0,
        }
    }
}

/// Represents a USB4 router instance (Host Root Router or eGPU Device Router).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usb4Router {
    /// Router unique topology identifier.
    pub router_id: u8,
    /// True if this is the handheld host silicon router.
    pub is_host: bool,
    /// Negotiated physical link rate in Gbps (e.g. 40 for Gen3x2 / Gen4x1).
    pub link_rate_gbps: u8,
    /// Total link bandwidth capacity in Megabits per second (e.g. 40,000 Mbps).
    pub total_bandwidth_mbps: u32,
    /// Registered transport adapters on this router.
    pub adapters: [Option<Usb4AdapterDescriptor>; 8],
    /// PCIe tunneling enabled flag.
    pub pcie_tunnel_active: bool,
    /// DisplayPort video tunneling enabled flag.
    pub dp_tunnel_active: bool,
}

impl Usb4Router {
    /// Instantiates the Handheld PC Host USB4 Router.
    pub const fn new_host_router() -> Self {
        Self {
            router_id: 0,
            is_host: true,
            link_rate_gbps: 40,
            total_bandwidth_mbps: 40_000,
            adapters: [None; 8],
            pcie_tunnel_active: false,
            dp_tunnel_active: false,
        }
    }

    /// Registers a protocol adapter into the router table.
    pub fn attach_adapter(&mut self, slot: usize, adapter: Usb4AdapterDescriptor) -> bool {
        if slot < self.adapters.len() {
            self.adapters[slot] = Some(adapter);
            true
        } else {
            false
        }
    }
}

/// Verification helper for Commit 1 structures and USB4 framing descriptors.
pub fn verify_usb4_router_and_tunnel_framing() -> bool {
    // 1. Verify Host Router Initialization
    let mut router = Usb4Router::new_host_router();
    if !router.is_host || router.link_rate_gbps != 40 || router.total_bandwidth_mbps != 40_000 {
        return false;
    }

    // 2. Attach PCIe Upstream Adapter
    let pcie_adapter = Usb4AdapterDescriptor::new(1, ProtocolAdapterType::PcieUpstream, 64);
    if !router.attach_adapter(0, pcie_adapter) {
        return false;
    }

    // 3. Attach DisplayPort In Adapter
    let dp_adapter = Usb4AdapterDescriptor::new(2, ProtocolAdapterType::DisplayPortIn, 128);
    if !router.attach_adapter(1, dp_adapter) {
        return false;
    }

    let a0 = router.adapters[0].unwrap();
    if a0.adapter_type != ProtocolAdapterType::PcieUpstream || a0.max_credits != 64 {
        return false;
    }

    // 4. Verify Tunneled Packet Framing
    let packet = Usb4TunneledPacket::new_pcie_tlp(0x10, 16, 1, 0x4000_0001); // 64-bit Memory Write TLP header
    if packet.packet_type != TunneledPacketType::PcieTlp || packet.hop_id != 0x10 || packet.length_dw != 16 {
        return false;
    }

    let dp_packet = Usb4TunneledPacket::new_dp_packet(0x20, 64, 2);
    if dp_packet.packet_type != TunneledPacketType::DisplayPort || dp_packet.hop_id != 0x20 {
        return false;
    }

    true
}
/// Minimum guaranteed PCIe bandwidth reservation in Mbps to prevent eGPU driver starvation.
pub const PCIE_MINIMUM_QOS_BANDWIDTH_MBPS: u32 = 4_000;
/// Total usable link bandwidth pool after protocol framing and PHY 128b/132b overhead.
pub const USB4_USABLE_LINK_CAPACITY_MBPS: u32 = 38_000;

/// Credit-based flow control manager for USB4 tunneled adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditFlowController {
    /// Maximum buffer credit capacity.
    pub max_credits: u16,
    /// Currently available transmit credits.
    pub available_credits: u16,
    /// Lifetime consumed credits counter.
    pub total_consumed: u32,
    /// Lifetime replenished credits counter.
    pub total_replenished: u32,
    /// Set to true if a flow control stall occurred due to credit exhaustion.
    pub stall_detected: bool,
}

impl CreditFlowController {
    /// Creates a new flow controller with specified maximum credit depth.
    pub const fn new(max_credits: u16) -> Self {
        Self {
            max_credits,
            available_credits: max_credits,
            total_consumed: 0,
            total_replenished: 0,
            stall_detected: false,
        }
    }

    /// Checks whether the adapter has sufficient credits to transmit the packet.
    #[inline(always)]
    pub const fn can_transmit(&self, credits_needed: u16) -> bool {
        self.available_credits >= credits_needed
    }

    /// Consumes credits to transmit a tunneled packet.
    pub fn consume_credits(&mut self, credits_needed: u16) -> Result<(), &'static str> {
        if self.available_credits >= credits_needed {
            self.available_credits -= credits_needed;
            self.total_consumed = self.total_consumed.saturating_add(credits_needed as u32);
            Ok(())
        } else {
            self.stall_detected = true;
            Err("USB4 flow control credit exhaustion: transmission stalled")
        }
    }

    /// Replenishes credits upon receiving a Credit Return packet from the partner router.
    pub fn return_credits(&mut self, credits_returned: u16) {
        let new_avail = (self.available_credits as u32 + credits_returned as u32)
            .min(self.max_credits as u32) as u16;
        self.available_credits = new_avail;
        self.total_replenished = self.total_replenished.saturating_add(credits_returned as u32);
        if self.available_credits > 0 {
            self.stall_detected = false;
        }
    }
}

/// Dynamic Bandwidth Allocation Balancer between DisplayPort and PCIe eGPU tunnels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usb4BandwidthBalancer {
    /// Total usable bandwidth capacity across the 40 Gbps link in Mbps.
    pub total_capacity_mbps: u32,
    /// Active bandwidth allocated to DisplayPort video tunneling in Mbps.
    pub dp_allocated_mbps: u32,
    /// Active bandwidth allocated to PCIe Gen 4x4 eGPU tunneling in Mbps.
    pub pcie_allocated_mbps: u32,
    /// True if an external monitor is active over the DisplayPort tunnel.
    pub dp_stream_active: bool,
}

impl Usb4BandwidthBalancer {
    /// Instantiates the bandwidth balancer with default 38 Gbps usable capacity.
    pub const fn new() -> Self {
        Self {
            total_capacity_mbps: USB4_USABLE_LINK_CAPACITY_MBPS,
            dp_allocated_mbps: 0,
            pcie_allocated_mbps: USB4_USABLE_LINK_CAPACITY_MBPS,
            dp_stream_active: false,
        }
    }

    /// Recalculates dynamic bandwidth allocation between isochronous DisplayPort and PCIe.
    ///
    /// Isochronous DisplayPort receives guaranteed requested bandwidth (e.g. 18,000 Mbps for DP 1.4 HBR3 4K@144Hz)
    /// while PCIe receives all remaining bandwidth, clamped to a guaranteed minimum QoS floor (4,000 Mbps).
    pub fn recalculate_allocation(
        &mut self,
        dp_active: bool,
        requested_dp_mbps: u32,
    ) -> (u32, u32) {
        self.dp_stream_active = dp_active;

        if !dp_active || requested_dp_mbps == 0 {
            // No external monitor: 100% of usable link bandwidth granted to PCIe eGPU
            self.dp_allocated_mbps = 0;
            self.pcie_allocated_mbps = self.total_capacity_mbps;
        } else {
            // DisplayPort active: Cap requested DP to preserve minimum PCIe QoS floor
            let max_allowable_dp = self.total_capacity_mbps.saturating_sub(PCIE_MINIMUM_QOS_BANDWIDTH_MBPS);
            let granted_dp = requested_dp_mbps.min(max_allowable_dp);

            self.dp_allocated_mbps = granted_dp;
            self.pcie_allocated_mbps = self.total_capacity_mbps.saturating_sub(granted_dp);
        }

        (self.dp_allocated_mbps, self.pcie_allocated_mbps)
    }
}

/// Verification helper for Commit 2 credit flow control and bandwidth balancing.
pub fn verify_credit_flow_control_and_bandwidth_balancer() -> bool {
    // 1. Verify Credit Flow Controller
    let mut fc = CreditFlowController::new(64);
    if fc.available_credits != 64 || !fc.can_transmit(16) {
        return false;
    }

    // Consume 32 credits
    if fc.consume_credits(32).is_err() || fc.available_credits != 32 {
        return false;
    }

    // Attempting to consume 40 credits (> 32) should fail and trip stall
    if fc.consume_credits(40).is_ok() || !fc.stall_detected {
        return false;
    }

    // Return 32 credits -> available back to 64, stall cleared
    fc.return_credits(32);
    if fc.available_credits != 64 || fc.stall_detected {
        return false;
    }

    // 2. Verify Dynamic Bandwidth Balancer without DP (100% PCIe)
    let mut balancer = Usb4BandwidthBalancer::new();
    let (dp0, pcie0) = balancer.recalculate_allocation(false, 0);
    if dp0 != 0 || pcie0 != USB4_USABLE_LINK_CAPACITY_MBPS {
        return false;
    }

    // 3. Verify Bandwidth Balancer with 4K DP Stream (18,000 Mbps)
    let (dp1, pcie1) = balancer.recalculate_allocation(true, 18_000);
    if dp1 != 18_000 || pcie1 != (USB4_USABLE_LINK_CAPACITY_MBPS - 18_000) {
        return false;
    }

    // 4. Verify Over-Subscription Clamping to Preserve PCIe QoS floor (4,000 Mbps)
    let (dp_over, pcie_over) = balancer.recalculate_allocation(true, 36_000);
    if pcie_over < PCIE_MINIMUM_QOS_BANDWIDTH_MBPS || dp_over > (USB4_USABLE_LINK_CAPACITY_MBPS - PCIE_MINIMUM_QOS_BANDWIDTH_MBPS) {
        return false;
    }

    true
}