//! Type-C Configuration Channel (CC) controller and DisplayPort Alternate Mode engine.

#![allow(dead_code)]

extern crate alloc;

/// Standard VESA DisplayPort SVID (Standard / Vendor ID).
pub const DP_SVID: u16 = 0xFF01;

// Type-C CC line states and plug orientation.

/// Configuration Channel (CC) line electrical termination state.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CcPinState {
    /// No termination detected (unconnected).
    Open = 0,
    /// 5.1 kOhm pull-down (Device/Sink present).
    Rd = 1,
    /// ~1 kOhm pull-down (Powered cable present).
    Ra = 2,
    /// Voltage level safe 0V (Discharged).
    VSafe0V = 3,
    /// Voltage level safe 5V (Active VBUS).
    VSafe5V = 4,
}

/// Physical USB Type-C plug orientation.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlugOrientation {
    Unplugged = 0,
    /// Plugged in standard orientation (CC1 active).
    Normal = 1,
    /// Plugged in 180-degree inverted orientation (CC2 active).
    Flipped = 2,
}

/// DisplayPort Pin Assignment configuration over Type-C high-speed lanes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpPinAssignment {
    None = 0,
    /// Pin Assignment C: All 4 high-speed lanes allocated to DisplayPort (no concurrent USB 3.x).
    PinAssignmentC = 1,
    /// Pin Assignment D: 2 lanes DisplayPort + 2 lanes USB 3.2 Gen2 (multi-function dock).
    PinAssignmentD = 2,
    /// Pin Assignment E: 4 lanes DisplayPort (standard Type-C to DisplayPort cable).
    PinAssignmentE = 3,
}

impl DpPinAssignment {
    #[inline(always)]
    pub const fn dp_lane_count(&self) -> u8 {
        match self {
            Self::PinAssignmentC | Self::PinAssignmentE => 4,
            Self::PinAssignmentD => 2,
            Self::None => 0,
        }
    }

    #[inline(always)]
    pub const fn supports_concurrent_usb3(&self) -> bool {
        matches!(self, Self::PinAssignmentD)
    }
}

// USB Power Delivery (USB PD) structured VDM definitions.

/// VDM Command Types.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VdmCommandType {
    Initiator = 0,
    ResponderAck = 1,
    ResponderNak = 2,
    ResponderBusy = 3,
}

/// Structured VDM Command Codes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VdmCommand {
    DiscoverIdentity = 1,
    DiscoverSvids = 2,
    DiscoverModes = 3,
    EnterMode = 4,
    ExitMode = 5,
    DpStatusUpdate = 0x10,
    DpConfigure = 0x11,
}

/// Structured VDM Header (32-bit USB PD message header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VdmHeader {
    pub svid: u16,
    pub is_structured: bool,
    pub command_type: VdmCommandType,
    pub object_position: u8,
    pub command: u8,
}

impl VdmHeader {
    pub const fn new(
        svid: u16,
        command_type: VdmCommandType,
        object_position: u8,
        command: u8,
    ) -> Self {
        Self {
            svid,
            is_structured: true,
            command_type,
            object_position,
            command,
        }
    }

    pub fn to_u32(&self) -> u32 {
        (self.svid as u32) << 16
            | (if self.is_structured { 1 << 15 } else { 0 })
            | ((self.command_type as u32 & 0x03) << 6)
            | ((self.object_position as u32 & 0x07) << 8)
            | (self.command as u32 & 0x1F)
    }

    pub fn from_u32(raw: u32) -> Self {
        Self {
            svid: (raw >> 16) as u16,
            is_structured: (raw & (1 << 15)) != 0,
            command_type: match (raw >> 6) & 0x03 {
                1 => VdmCommandType::ResponderAck,
                2 => VdmCommandType::ResponderNak,
                3 => VdmCommandType::ResponderBusy,
                _ => VdmCommandType::Initiator,
            },
            object_position: ((raw >> 8) & 0x07) as u8,
            command: (raw & 0x1F) as u8,
        }
    }
}

/// DisplayPort Capabilities Vendor Defined Object (VDO).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpCapabilitiesVdo {
    pub dp_version: u8,
    pub supports_pin_c: bool,
    pub supports_pin_d: bool,
    pub supports_pin_e: bool,
    pub dfp_d_capable: bool,
    pub ufp_d_capable: bool,
}

impl DpCapabilitiesVdo {
    pub const fn new_standard_dock() -> Self {
        Self {
            dp_version: 0x14, // DP 1.4
            supports_pin_c: true,
            supports_pin_d: true,
            supports_pin_e: false,
            dfp_d_capable: false,
            ufp_d_capable: true, // Dock is Upstream-Facing Display Port Sink
        }
    }

    pub fn to_u32(&self) -> u32 {
        let mut vdo: u32 = (self.dp_version as u32) & 0x0F;
        if self.supports_pin_c {
            vdo |= 1 << 10;
        }
        if self.supports_pin_d {
            vdo |= 1 << 11;
        }
        if self.supports_pin_e {
            vdo |= 1 << 12;
        }
        if self.dfp_d_capable {
            vdo |= 1 << 1;
        }
        if self.ufp_d_capable {
            vdo |= 1 << 2;
        }
        vdo
    }
}

// DisplayPort Alternate Mode controller state machine.

/// Connection and negotiation state machine for Type-C DisplayPort Alt Mode.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpAltModeState {
    Disconnected = 0,
    Attached = 1,
    DiscoveringSvids = 2,
    DiscoveringModes = 3,
    EnteringMode = 4,
    ModeActive = 5,
    Configured = 6,
}

/// Controller managing Type-C CC line detection and DP Alt Mode VDM negotiation.
#[derive(Debug)]
pub struct DpAltModeController {
    pub state: DpAltModeState,
    pub orientation: PlugOrientation,
    pub negotiated_pin_assignment: DpPinAssignment,
    pub remote_capabilities: Option<DpCapabilitiesVdo>,
    pub cc1_state: CcPinState,
    pub cc2_state: CcPinState,
}

impl DpAltModeController {
    pub const fn new() -> Self {
        Self {
            state: DpAltModeState::Disconnected,
            orientation: PlugOrientation::Unplugged,
            negotiated_pin_assignment: DpPinAssignment::None,
            remote_capabilities: None,
            cc1_state: CcPinState::Open,
            cc2_state: CcPinState::Open,
        }
    }

    /// Evaluates CC line termination to detect cable connection and orientation.
    pub fn update_cc_pins(&mut self, cc1: CcPinState, cc2: CcPinState) -> PlugOrientation {
        self.cc1_state = cc1;
        self.cc2_state = cc2;

        if cc1 == CcPinState::Rd && (cc2 == CcPinState::Open || cc2 == CcPinState::Ra) {
            self.orientation = PlugOrientation::Normal;
            self.state = DpAltModeState::Attached;
        } else if cc2 == CcPinState::Rd && (cc1 == CcPinState::Open || cc1 == CcPinState::Ra) {
            self.orientation = PlugOrientation::Flipped;
            self.state = DpAltModeState::Attached;
        } else if cc1 == CcPinState::Open && cc2 == CcPinState::Open {
            self.orientation = PlugOrientation::Unplugged;
            self.state = DpAltModeState::Disconnected;
            self.negotiated_pin_assignment = DpPinAssignment::None;
            self.remote_capabilities = None;
        }
        self.orientation
    }

    /// Advances the VDM negotiation sequence.
    pub fn process_step(&mut self, preferred_pin: DpPinAssignment) -> Result<DpAltModeState, &'static str> {
        match self.state {
            DpAltModeState::Disconnected => Err("Cannot negotiate VDM while disconnected"),
            DpAltModeState::Attached => {
                // Step 1: Discover SVIDs
                self.state = DpAltModeState::DiscoveringSvids;
                Ok(self.state)
            }
            DpAltModeState::DiscoveringSvids => {
                // Step 2: SVID 0xFF01 found, query capabilities (Discover Modes)
                self.state = DpAltModeState::DiscoveringModes;
                self.remote_capabilities = Some(DpCapabilitiesVdo::new_standard_dock());
                Ok(self.state)
            }
            DpAltModeState::DiscoveringModes => {
                // Step 3: Enter Mode
                self.state = DpAltModeState::EnteringMode;
                Ok(self.state)
            }
            DpAltModeState::EnteringMode => {
                // Step 4: Mode Active, proceed to Configure
                self.state = DpAltModeState::ModeActive;
                Ok(self.state)
            }
            DpAltModeState::ModeActive => {
                // Step 5: Configure Pin Assignment
                let caps = self.remote_capabilities.ok_or("Missing remote DP capabilities")?;
                let selected_pin = match preferred_pin {
                    DpPinAssignment::PinAssignmentD if caps.supports_pin_d => DpPinAssignment::PinAssignmentD,
                    DpPinAssignment::PinAssignmentC if caps.supports_pin_c => DpPinAssignment::PinAssignmentC,
                    _ if caps.supports_pin_d => DpPinAssignment::PinAssignmentD,
                    _ if caps.supports_pin_c => DpPinAssignment::PinAssignmentC,
                    _ => return Err("No mutually supported DP pin assignment found"),
                };
                self.negotiated_pin_assignment = selected_pin;
                self.state = DpAltModeState::Configured;
                Ok(self.state)
            }
            DpAltModeState::Configured => Ok(self.state),
        }
    }

    /// Total DisplayPort lanes currently mapped onto the Type-C cable.
    #[inline(always)]
    pub fn active_dp_lane_count(&self) -> u8 {
        if self.state == DpAltModeState::Configured {
            self.negotiated_pin_assignment.dp_lane_count()
        } else {
            0
        }
    }

    /// Whether USB 3.x high-speed data is operating concurrently with DisplayPort.
    #[inline(always)]
    pub fn is_concurrent_usb3_active(&self) -> bool {
        if self.state == DpAltModeState::Configured {
            self.negotiated_pin_assignment.supports_concurrent_usb3()
        } else {
            false
        }
    }
}
