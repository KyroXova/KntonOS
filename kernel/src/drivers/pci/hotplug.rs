//! PCIe Native Hot-Plug Controller & Slot Event State Machine.
//!
//! Implements PCIe Base Specification Section 6.7 Native Hot-Plug architecture:
//! - Slot Capabilities (SlotCap), Control (SlotCtl), and Status (SlotSta) registers
//! - Presence Detect Change (PDC) and Attention Button Press (ABP) interrupt processing
//! - Slot Power Controller state machine (Power On / Off, Link Training, Inrush Limiting)
//! - Attention and Power indicator control (On, Blinking, Off)

use alloc::vec::Vec;

// -----------------------------------------------------------------------------
// PCIe Native Hot-Plug Register Definitions (PCIe Spec Section 6.7 / 7.8)
// -----------------------------------------------------------------------------

/// Offset of Slot Capabilities register relative to PCIe Capability base
pub const PCIE_SLOTCAP_OFFSET: u8 = 0x14;
/// Offset of Slot Control register relative to PCIe Capability base
pub const PCIE_SLOTCTL_OFFSET: u8 = 0x18;
/// Offset of Slot Status register relative to PCIe Capability base
pub const PCIE_SLOTSTA_OFFSET: u8 = 0x1A;

// Slot Capabilities (SlotCap) Flags
pub const PCIE_SLOTCAP_ABP: u32 = 1 << 0;  // Attention Button Present
pub const PCIE_SLOTCAP_PCP: u32 = 1 << 1;  // Power Controller Present
pub const PCIE_SLOTCAP_MSP: u32 = 1 << 2;  // MRL Sensor Present
pub const PCIE_SLOTCAP_AIP: u32 = 1 << 3;  // Attention Indicator Present
pub const PCIE_SLOTCAP_PIP: u32 = 1 << 4;  // Power Indicator Present
pub const PCIE_SLOTCAP_HPS: u32 = 1 << 5;  // Hot-Plug Surprise
pub const PCIE_SLOTCAP_HPC: u32 = 1 << 6;  // Hot-Plug Capable
pub const PCIE_SLOTCAP_EIP: u32 = 1 << 17; // Electromechanical Interlock Present
pub const PCIE_SLOTCAP_PHYS_SLOT_MASK: u32 = 0x1FFF_0000;

// Slot Control (SlotCtl) Flags
pub const PCIE_SLOTCTL_ABPE: u16 = 1 << 0;  // Attention Button Pressed Enable
pub const PCIE_SLOTCTL_PFDE: u16 = 1 << 1;  // Power Fault Detected Enable
pub const PCIE_SLOTCTL_MSCE: u16 = 1 << 2;  // MRL Sensor Changed Enable
pub const PCIE_SLOTCTL_PDCE: u16 = 1 << 3;  // Presence Detect Changed Enable
pub const PCIE_SLOTCTL_CCIE: u16 = 1 << 4;  // Command Completed Interrupt Enable
pub const PCIE_SLOTCTL_HPIE: u16 = 1 << 5;  // Hot-Plug Interrupt Enable
pub const PCIE_SLOTCTL_AIC_MASK: u16 = 0x00C0; // Attention Indicator Control
pub const PCIE_SLOTCTL_PIC_MASK: u16 = 0x0300; // Power Indicator Control
pub const PCIE_SLOTCTL_PCC_OFF: u16 = 1 << 10; // Power Controller: 0=On, 1=Off
pub const PCIE_SLOTCTL_DLLSCE: u16 = 1 << 12; // Data Link Layer State Changed Enable

// Slot Status (SlotSta) Flags (Write-1-to-Clear events)
pub const PCIE_SLOTSTA_ABP: u16 = 1 << 0;   // Attention Button Pressed
pub const PCIE_SLOTSTA_PFD: u16 = 1 << 1;   // Power Fault Detected
pub const PCIE_SLOTSTA_MSC: u16 = 1 << 2;   // MRL Sensor Changed
pub const PCIE_SLOTSTA_PDC: u16 = 1 << 3;   // Presence Detect Changed
pub const PCIE_SLOTSTA_CC: u16 = 1 << 4;    // Command Completed
pub const PCIE_SLOTSTA_MSS_OPEN: u16 = 1 << 5; // MRL Sensor State (0=Closed, 1=Open)
pub const PCIE_SLOTSTA_PDS_PRESENT: u16 = 1 << 6; // Presence Detect State (1=Present)
pub const PCIE_SLOTSTA_EIS: u16 = 1 << 7;   // Electromechanical Interlock Status
pub const PCIE_SLOTSTA_DLLSC: u16 = 1 << 8; // Data Link Layer State Changed

// Indicator States (AIC / PIC)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorState {
    Reserved = 0b00,
    On = 0b01,
    Blink = 0b10,
    Off = 0b11,
}

/// Slot Power Controller state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotPowerState {
    /// Slot power is deasserted (Vcc = 0V).
    PowerOff,
    /// Inrush current limiting and rail stabilization delay in progress.
    PoweringOn,
    /// Main power rails active (3.3V / 12V stable).
    PowerOn,
    /// Orderly device power-down sequence in progress.
    PoweringOff,
}

/// Slot card presence lifecycle state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotPresenceState {
    /// Slot physically unoccupied.
    Empty,
    /// Presence pin grounded; card detected, debouncing in progress.
    CardDetected,
    /// Slot powered, PCIe link trained, device active and operating.
    CardConfigured,
    /// Attention button pressed or software disconnect requested; preparing removal.
    EjectPending,
    /// Device isolated, slot unpowered, safe for user extraction.
    ReadyForRemoval,
}

/// High-level PCIe Hot-Plug Event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugEvent {
    /// Presence Detect Changed: physical insertion or extraction detected.
    PresenceChanged { card_present: bool },
    /// Physical attention button pressed on slot chassis.
    AttentionButtonPressed,
    /// Overcurrent or power rail fault detected on slot.
    PowerFault,
    /// Mechanical Retention Latch (MRL) opened or closed.
    LatchChanged { is_open: bool },
    /// PCIe Data Link Layer transitioned between active and down.
    LinkStateChanged { link_active: bool },
}

/// Action determined by the hot-plug state machine in response to an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugAction {
    None,
    PowerOnAndTrainLink,
    QuiesceAndPowerOff,
    TriggerScanoutMigration,
    BlinkAttentionIndicator,
}

/// PCIe Slot Controller managing hardware slot registers and power state.
#[derive(Debug, Clone)]
pub struct PcieSlotController {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub pcie_cap_offset: u8,
    pub slot_number: u32,
    pub slot_cap: u32,
    pub slot_ctl: u16,
    pub slot_sta: u16,
    pub power_state: SlotPowerState,
    pub presence_state: SlotPresenceState,
    pub link_active: bool,
    pub link_speed_gen: u8,
    pub link_width_lanes: u8,
}

impl PcieSlotController {
    /// Creates a new slot controller for a hot-plug capable PCIe Root Port or Downstream Switch Port.
    pub fn new(
        bus: u8,
        device: u8,
        function: u8,
        pcie_cap_offset: u8,
        slot_number: u32,
    ) -> Self {
        let slot_cap = PCIE_SLOTCAP_ABP
            | PCIE_SLOTCAP_PCP
            | PCIE_SLOTCAP_AIP
            | PCIE_SLOTCAP_PIP
            | PCIE_SLOTCAP_HPC
            | ((slot_number & 0x1FFF) << 19);

        Self {
            bus,
            device,
            function,
            pcie_cap_offset,
            slot_number,
            slot_cap,
            slot_ctl: PCIE_SLOTCTL_PCC_OFF, // Default power off
            slot_sta: 0,
            power_state: SlotPowerState::PowerOff,
            presence_state: SlotPresenceState::Empty,
            link_active: false,
            link_speed_gen: 0,
            link_width_lanes: 0,
        }
    }

    /// Configures Slot Control register to enable all hot-plug interrupts.
    pub fn enable_hotplug_interrupts(&mut self) {
        self.slot_ctl |= PCIE_SLOTCTL_ABPE
            | PCIE_SLOTCTL_PFDE
            | PCIE_SLOTCTL_PDCE
            | PCIE_SLOTCTL_HPIE
            | PCIE_SLOTCTL_DLLSCE;
    }

    /// Sets the Attention Indicator State (AIC).
    pub fn set_attention_indicator(&mut self, state: IndicatorState) {
        self.slot_ctl = (self.slot_ctl & !PCIE_SLOTCTL_AIC_MASK) | ((state as u16) << 6);
    }

    /// Sets the Power Indicator State (PIC).
    pub fn set_power_indicator(&mut self, state: IndicatorState) {
        self.slot_ctl = (self.slot_ctl & !PCIE_SLOTCTL_PIC_MASK) | ((state as u16) << 8);
    }

    /// Powers on the slot, enabling Vcc and initializing link training.
    pub fn power_on_slot(&mut self) -> Result<(), &'static str> {
        if self.power_state == SlotPowerState::PowerOn {
            return Ok(());
        }

        self.power_state = SlotPowerState::PoweringOn;
        self.set_power_indicator(IndicatorState::Blink);

        // Clear power controller off bit (PCC = 0 -> Power On)
        self.slot_ctl &= !PCIE_SLOTCTL_PCC_OFF;

        // In full hardware: delay 100ms for power rail settling (PCIe Spec)
        self.power_state = SlotPowerState::PowerOn;
        self.set_power_indicator(IndicatorState::On);
        Ok(())
    }

    /// Powers off the slot, cutting Vcc rails safely.
    pub fn power_off_slot(&mut self) -> Result<(), &'static str> {
        self.power_state = SlotPowerState::PoweringOff;
        self.set_power_indicator(IndicatorState::Blink);

        // Set power controller off bit (PCC = 1 -> Power Off)
        self.slot_ctl |= PCIE_SLOTCTL_PCC_OFF;

        self.power_state = SlotPowerState::PowerOff;
        self.set_power_indicator(IndicatorState::Off);
        self.link_active = false;
        self.link_speed_gen = 0;
        self.link_width_lanes = 0;
        Ok(())
    }

    /// Simulates hardware link training completion.
    pub fn complete_link_training(&mut self, speed_gen: u8, width_lanes: u8) {
        self.link_active = true;
        self.link_speed_gen = speed_gen;
        self.link_width_lanes = width_lanes;
        self.presence_state = SlotPresenceState::CardConfigured;
    }

    /// Processes incoming slot status register interrupts and updates state machine.
    pub fn handle_slot_interrupt(&mut self, status_bits: u16) -> (Option<HotplugEvent>, HotplugAction) {
        self.slot_sta |= status_bits;

        // 1. Attention Button Pressed (ABP)
        if (status_bits & PCIE_SLOTSTA_ABP) != 0 {
            // Write-1-to-clear
            self.slot_sta &= !PCIE_SLOTSTA_ABP;
            let event = HotplugEvent::AttentionButtonPressed;

            let action = match self.presence_state {
                SlotPresenceState::CardConfigured => {
                    self.presence_state = SlotPresenceState::EjectPending;
                    self.set_attention_indicator(IndicatorState::Blink);
                    HotplugAction::QuiesceAndPowerOff
                }
                SlotPresenceState::EjectPending | SlotPresenceState::ReadyForRemoval => {
                    // Second press cancels eject or acknowledges removal
                    HotplugAction::None
                }
                SlotPresenceState::Empty | SlotPresenceState::CardDetected => {
                    HotplugAction::BlinkAttentionIndicator
                }
            };
            return (Some(event), action);
        }

        // 2. Presence Detect Changed (PDC)
        if (status_bits & PCIE_SLOTSTA_PDC) != 0 {
            self.slot_sta &= !PCIE_SLOTSTA_PDC;
            let is_present = (self.slot_sta & PCIE_SLOTSTA_PDS_PRESENT) != 0;
            let event = HotplugEvent::PresenceChanged { card_present: is_present };

            let action = if is_present {
                self.presence_state = SlotPresenceState::CardDetected;
                HotplugAction::PowerOnAndTrainLink
            } else {
                self.presence_state = SlotPresenceState::Empty;
                let _ = self.power_off_slot();
                HotplugAction::TriggerScanoutMigration
            };
            return (Some(event), action);
        }

        // 3. Data Link Layer State Changed (DLLSC)
        if (status_bits & PCIE_SLOTSTA_DLLSC) != 0 {
            self.slot_sta &= !PCIE_SLOTSTA_DLLSC;
            let link_up = self.link_active;
            let event = HotplugEvent::LinkStateChanged { link_active: link_up };
            return (Some(event), HotplugAction::None);
        }

        // 4. Power Fault Detected (PFD)
        if (status_bits & PCIE_SLOTSTA_PFD) != 0 {
            self.slot_sta &= !PCIE_SLOTSTA_PFD;
            let _ = self.power_off_slot();
            self.set_attention_indicator(IndicatorState::On);
            return (Some(HotplugEvent::PowerFault), HotplugAction::QuiesceAndPowerOff);
        }

        (None, HotplugAction::None)
    }
}

/// PCIe Hot-Plug Event Daemon tracking all hot-plug slots across the system.
pub struct PcieHotplugDaemon {
    pub slots: Vec<PcieSlotController>,
    pub total_insertions: u32,
    pub total_extractions: u32,
}

impl PcieHotplugDaemon {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            total_insertions: 0,
            total_extractions: 0,
        }
    }

    pub fn register_slot(&mut self, slot: PcieSlotController) {
        self.slots.push(slot);
    }

    /// Dispatches a simulated or hardware slot interrupt by physical slot number.
    pub fn dispatch_slot_event(
        &mut self,
        slot_number: u32,
        status_bits: u16,
    ) -> Option<(HotplugEvent, HotplugAction)> {
        for slot in self.slots.iter_mut() {
            if slot.slot_number == slot_number {
                let (ev_opt, action) = slot.handle_slot_interrupt(status_bits);
                if let Some(ev) = ev_opt {
                    match ev {
                        HotplugEvent::PresenceChanged { card_present } => {
                            if card_present {
                                self.total_insertions += 1;
                            } else {
                                self.total_extractions += 1;
                            }
                        }
                        _ => {}
                    }
                    return Some((ev, action));
                }
            }
        }
        None
    }
}

/// Unit test verifying PCIe slot controller, presence detect change (PDC), and power state machine.
pub fn verify_pcie_hotplug_controller() -> bool {
    // 1. Initialize slot controller on Bus 0, Device 28 (Root Port 1), Function 0, Slot 1
    let mut slot = PcieSlotController::new(0, 28, 0, 0x40, 1);
    slot.enable_hotplug_interrupts();

    if slot.slot_number != 1 || slot.power_state != SlotPowerState::PowerOff {
        return false;
    }
    if (slot.slot_ctl & PCIE_SLOTCTL_PDCE) == 0 || (slot.slot_ctl & PCIE_SLOTCTL_HPIE) == 0 {
        return false;
    }

    // 2. Simulate physical card insertion: Presence Detect State = 1, Presence Detect Change = 1
    let insert_bits = PCIE_SLOTSTA_PDC | PCIE_SLOTSTA_PDS_PRESENT;
    let (ev_opt, action) = slot.handle_slot_interrupt(insert_bits);

    match ev_opt {
        Some(HotplugEvent::PresenceChanged { card_present: true }) => {}
        _ => return false,
    }
    if action != HotplugAction::PowerOnAndTrainLink {
        return false;
    }
    if slot.presence_state != SlotPresenceState::CardDetected {
        return false;
    }

    // 3. Power on slot and complete PCIe link training (Gen 4 x4)
    if slot.power_on_slot().is_err() || slot.power_state != SlotPowerState::PowerOn {
        return false;
    }
    slot.complete_link_training(4, 4);
    if !slot.link_active || slot.link_speed_gen != 4 || slot.link_width_lanes != 4 {
        return false;
    }
    if slot.presence_state != SlotPresenceState::CardConfigured {
        return false;
    }

    // 4. Simulate user pressing the chassis Attention Button (ABP)
    let (abp_ev, abp_action) = slot.handle_slot_interrupt(PCIE_SLOTSTA_ABP);
    match abp_ev {
        Some(HotplugEvent::AttentionButtonPressed) => {}
        _ => return false,
    }
    if abp_action != HotplugAction::QuiesceAndPowerOff {
        return false;
    }
    if slot.presence_state != SlotPresenceState::EjectPending {
        return false;
    }

    // 5. Simulate card physical extraction after quiescence
    let remove_bits = PCIE_SLOTSTA_PDC; // PDS = 0 (Empty), PDC = 1
    let (rem_ev, rem_action) = slot.handle_slot_interrupt(remove_bits);
    match rem_ev {
        Some(HotplugEvent::PresenceChanged { card_present: false }) => {}
        _ => return false,
    }
    if rem_action != HotplugAction::TriggerScanoutMigration {
        return false;
    }
    if slot.presence_state != SlotPresenceState::Empty || slot.power_state != SlotPowerState::PowerOff {
        return false;
    }

    // 6. Verify PcieHotplugDaemon event routing
    let mut daemon = PcieHotplugDaemon::new();
    daemon.register_slot(slot);

    let res = daemon.dispatch_slot_event(1, PCIE_SLOTSTA_PDC | PCIE_SLOTSTA_PDS_PRESENT);
    if res.is_none() || daemon.total_insertions != 1 {
        return false;
    }

    true
}
