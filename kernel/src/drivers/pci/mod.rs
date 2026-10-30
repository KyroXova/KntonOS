//! PCI and PCI Express Subsystem.

pub mod flr;
pub mod hotplug;
pub mod egpu;

pub use flr::*;
pub use hotplug::*;
pub use egpu::*;

/// Comprehensive verification self-test for PCIe Native Hot-Plug & External eGPU Hot-Connect Controller.
///
/// Emits sentinel [KNT-PCI-HOTPLUG-PASS] upon successful validation.
pub fn test_pci_hotplug_subsystem() -> bool {
    // 1. Verify PCIe Base Spec Section 6.7 native hot-plug slot controller and presence state machine
    if !verify_pcie_hotplug_controller() {
        return false;
    }

    // 2. Verify dynamic Type 1 bridge secondary/subordinate bus renumbering and MMIO BAR window allocation
    if !verify_bridge_renumbering_and_bar_allocations() {
        return false;
    }

    // 3. Verify external eGPU hot-connect pipeline, scanout migration, and graceful/surprise removal
    if !verify_egpu_hot_connect_and_scanout_migration() {
        return false;
    }

    // 4. End-to-end integration loopback test:
    // Full Thunderbolt 4 external eGPU hot-attach, bus renumbering, MMIO resource configuration,
    // display scanout migration, attention button press (ABP), and orderly detachment.
    let mut hotplug_daemon = PcieHotplugDaemon::new();
    let slot = PcieSlotController::new(0, 28, 0, 0x40, 1);
    hotplug_daemon.register_slot(slot);

    let mut pool = PciResourceWindowPool::new_default();
    let mut bus_alloc = PciBusAllocator::new(12, 32);

    let igpu_fb = 0x8000_0000u64;
    let mut egpu_manager = EgpuHotConnectManager::new(
        EgpuInterconnectType::Thunderbolt4,
        1,
        (0, 28, 0),
        igpu_fb,
    );

    // Event 1: External Thunderbolt 4 cable connected (Presence Detect Changed: Card Present)
    let pci_insert = PCIE_SLOTSTA_PDC | PCIE_SLOTSTA_PDS_PRESENT;
    let (ev, action) = match hotplug_daemon.dispatch_slot_event(1, pci_insert) {
        Some(res) => res,
        None => return false,
    };

    if ev != (HotplugEvent::PresenceChanged { card_present: true })
        || action != HotplugAction::PowerOnAndTrainLink
    {
        return false;
    }

    // Power on slot and complete link training
    let slot_ref = &mut hotplug_daemon.slots[0];
    if slot_ref.power_on_slot().is_err() {
        return false;
    }
    slot_ref.complete_link_training(4, 4); // Gen 4 x4 Thunderbolt/USB4 tunnel

    // Attach high-end eGPU (e.g. NVIDIA RTX 4090: Vendor 0x10DE, Device 0x2684, 16 GiB VRAM)
    let vram_size = 16 * 1024 * 1024 * 1024u64;
    if egpu_manager
        .attach_egpu(0x10DE, 0x2684, vram_size, &mut pool, &mut bus_alloc)
        .is_err()
    {
        return false;
    }

    // Migrate compositor display scanout to discrete eGPU
    if egpu_manager.migrate_scanout_to_egpu(0).is_err() {
        return false;
    }
    if egpu_manager.current_scanout.source != GpuScanoutSource::ExternalDiscreteGpu {
        return false;
    }

    // Event 2: User presses chassis Attention Button (ABP) to request ejection
    let (abp_ev, abp_action) = match hotplug_daemon.dispatch_slot_event(1, PCIE_SLOTSTA_ABP) {
        Some(res) => res,
        None => return false,
    };

    if abp_ev != HotplugEvent::AttentionButtonPressed
        || abp_action != HotplugAction::QuiesceAndPowerOff
    {
        return false;
    }

    // Execute graceful eject and fallback scanout to iGPU
    if egpu_manager.request_graceful_eject().is_err() {
        return false;
    }
    if egpu_manager.current_scanout.source != GpuScanoutSource::InternalIntegratedGpu {
        return false;
    }

    // Power off slot
    let slot_ref = &mut hotplug_daemon.slots[0];
    if slot_ref.power_off_slot().is_err() {
        return false;
    }

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-PCI-HOTPLUG-PASS] PCIe native hot-plug and external eGPU hot-connect verified\r\n");

    true
}
