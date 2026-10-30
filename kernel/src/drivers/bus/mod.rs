//! Bus and Interconnect Drivers for Handheld PC Architecture.

pub mod usb4;

pub use usb4::*;

/// Verification self-test suite for Handheld PC USB4/Thunderbolt 4 Dynamic PCIe Tunneling & eGPU Governor.
pub fn test_usb4_egpu_governor_subsystem() -> bool {
    crate::serial_write_str("[KNT-BUS] Testing USB4/Thunderbolt 4 dynamic PCIe tunneling & eGPU governor...\r\n");

    // 1. Verify USB4 Router Configuration Spaces, Adapters, and PCIe Framing
    if !verify_usb4_router_and_tunnel_framing() {
        crate::serial_write_str("[KNT-PANIC] USB4 router and tunnel framing verification failed!\r\n");
        return false;
    }

    // 2. Verify Credit-Based Flow Control and DP/PCIe Bandwidth Balancer
    if !verify_credit_flow_control_and_bandwidth_balancer() {
        crate::serial_write_str("[KNT-PANIC] USB4 credit flow control and bandwidth balancer failed!\r\n");
        return false;
    }

    // 3. Verify Surprise Hot-Unplug Safety Barrier and Secondary Bus Reset
    if !verify_surprise_unplug_barrier_and_bridge_reset() {
        crate::serial_write_str("[KNT-PANIC] USB4 surprise unplug barrier and bridge reset failed!\r\n");
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-BUS-USB4-EGPU-PASS] USB4 PCIe tunneling and hot-plug eGPU governor verified\r\n");
    true
}
