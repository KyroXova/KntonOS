//! Display Drivers and Backlight Control Subsystems for KntonOS.

pub mod pwm;
pub mod backlight;
pub mod dp_alt;
pub mod dpcd;

pub use pwm::*;
pub use backlight::*;
pub use dp_alt::*;
pub use dpcd::*;


use crate::serial_write_str;

/// Verification self-test suite for Handheld Backlight PWM Modulator & Flicker-Free DC Dimming Governor.
pub fn test_backlight_pwm_subsystem() -> bool {
    serial_write_str("[KNT-DISPLAY] Testing backlight PWM modulator & flicker-free DC dimming governor...\r\n");

    // 1. Verify Hardware PWM Frequency Synthesizer & Duty-Cycle Modulator
    if !verify_pwm_frequency_and_duty_cycle() {
        serial_write_str("[KNT-PANIC] Backlight PWM frequency synthesizer / modulator verification failed!\r\n");
        return false;
    }

    // 2. Verify Hybrid DC Dimming Transition Governor & CIE 1931 Perceptual Curve
    if !verify_hybrid_dc_dimming_and_cie1931() {
        serial_write_str("[KNT-PANIC] Hybrid DC dimming / CIE 1931 curve verification failed!\r\n");
        return false;
    }

    // 3. Verify Smooth Fade Pacing, Ambient Minimum Clamp, and OLED ABL Scaling
    if !verify_fade_pacing_ambient_clamp_and_oled_scaling() {
        serial_write_str("[KNT-PANIC] Fade pacing / ambient clamp / OLED scaling verification failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-DISPLAY-PWM-PASS] Unified backlight PWM modulator and DC dimming governor verified\r\n");
    true
}


/// Verification self-test suite for USB-C DisplayPort Alternate Mode and hot-plug daemon.
pub fn test_dp_alt_mode_subsystem() -> bool {
    serial_write_str("[KNT-DISPLAY] Testing USB-C DisplayPort Alternate Mode & Hot-Plug Modeset Daemon...\r\n");

    // 1. Verify Type-C CC line detection and orientation
    let mut controller = DpAltModeController::new();
    if controller.state != DpAltModeState::Disconnected {
        return false;
    }

    // Normal plug orientation: CC1 = Rd, CC2 = Open
    let orient_normal = controller.update_cc_pins(CcPinState::Rd, CcPinState::Open);
    if orient_normal != PlugOrientation::Normal || controller.state != DpAltModeState::Attached {
        return false;
    }

    // Flipped plug orientation: CC1 = Open, CC2 = Rd
    let orient_flipped = controller.update_cc_pins(CcPinState::Open, CcPinState::Rd);
    if orient_flipped != PlugOrientation::Flipped || controller.state != DpAltModeState::Attached {
        return false;
    }

    // 2. Verify USB PD VDM Negotiation sequence
    // Discover SVIDs
    if controller.process_step(DpPinAssignment::PinAssignmentD).is_err() || controller.state != DpAltModeState::DiscoveringSvids {
        return false;
    }
    // Discover Modes
    if controller.process_step(DpPinAssignment::PinAssignmentD).is_err() || controller.state != DpAltModeState::DiscoveringModes {
        return false;
    }
    // Enter Mode
    if controller.process_step(DpPinAssignment::PinAssignmentD).is_err() || controller.state != DpAltModeState::EnteringMode {
        return false;
    }
    // Mode Active
    if controller.process_step(DpPinAssignment::PinAssignmentD).is_err() || controller.state != DpAltModeState::ModeActive {
        return false;
    }
    // Configure DP Pin Assignment (Preferred Pin D for multi-function dock)
    if controller.process_step(DpPinAssignment::PinAssignmentD).is_err() || controller.state != DpAltModeState::Configured {
        return false;
    }
    if controller.negotiated_pin_assignment != DpPinAssignment::PinAssignmentD {
        return false;
    }
    if controller.active_dp_lane_count() != 2 || !controller.is_concurrent_usb3_active() {
        return false;
    }

    // 3. Verify AUX Channel Transactions & DPCD Receiver Capability Parser
    let mut aux = AuxChannelEngine::new();
    let caps = match aux.query_receiver_caps() {
        Ok(c) => c,
        Err(_) => return false,
    };
    if caps.dpcd_rev != 0x14 || caps.max_link_rate != DpLinkRate::Hbr2 || caps.max_lane_count != 4 || !caps.enhanced_framing {
        return false;
    }
    if caps.max_raw_bandwidth_mbps() != 5400 * 4 {
        return false;
    }

    // 4. Verify DisplayPort Link Training (Clock Recovery + Channel Equalization)
    let link_engine = DpLinkTrainingEngine::new();
    let train_res = match link_engine.execute_link_training(&mut aux, DpLinkRate::Hbr2, controller.active_dp_lane_count()) {
        Ok(res) => res,
        Err(_) => return false,
    };
    if !train_res.clock_recovery_passed || !train_res.channel_eq_passed || train_res.lane_count != 2 {
        return false;
    }

    // 5. Verify Hot-Plug Detect (HPD) Daemon & Dynamic Multi-Display Modesetting
    let mut hpd_daemon = HotPlugModesetDaemon::new();
    if hpd_daemon.topology != DisplayTopologyMode::InternalOnly || !hpd_daemon.is_internal_scanout_active() || hpd_daemon.is_external_scanout_active() {
        return false;
    }

    // Dock event: connect external 4K60 display in ExternalOnly mode
    hpd_daemon.handle_hpd_connect(DisplayModeDescriptor::external_4k60(), DisplayTopologyMode::ExternalOnly);
    if !hpd_daemon.is_external_connected || hpd_daemon.is_internal_scanout_active() || !hpd_daemon.is_external_scanout_active() {
        return false;
    }

    // Undock event: disconnect external display and restore internal handheld panel
    hpd_daemon.handle_hpd_disconnect();
    if hpd_daemon.is_external_connected || !hpd_daemon.is_internal_scanout_active() || hpd_daemon.is_external_scanout_active() {
        return false;
    }
    if hpd_daemon.modeset_handoff_count != 2 {
        return false;
    }

    // 6. Emit deterministic boot sentinel
    serial_write_str("[KNT-DISPLAY-DP-PASS] USB-C DisplayPort Alternate Mode and hot-plug modeset daemon verified\r\n");

    true
}
