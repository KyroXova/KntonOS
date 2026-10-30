//! Dynamic Power Governor & Thermal Energy Meter Subsystem for KntonOS.
//!
//! Provides Intel RAPL MSR energy metering, AMD SMU power telemetry,
//! digital thermal sensor monitoring, and closed-loop PID TDP governors.

pub mod governor;
pub mod rapl;
pub mod parking;
pub mod cstate;
pub mod epb;
pub mod freeze;
pub mod s3_resume;
pub mod fan;
pub mod thermal;
pub mod hibernate;
pub mod lz4_restore;
pub mod battery;
pub mod charger;

pub use hibernate::*;
pub use lz4_restore::*;
pub use battery::*;
pub use charger::*;

pub use s3_resume::*;
pub use fan::*;
pub use thermal::*;

pub use freeze::*;

pub use governor::{
    verify_pid_power_governor, verify_thermal_monitoring, CoreThermalStatus, GovernorAction,
    GovernorProfile, PidPowerGovernor, ThermalSensorEngine, ThermalThrottleFlags,
    DEFAULT_BALANCED_TDP_MW, DEFAULT_PERFORMANCE_TDP_MW, DEFAULT_POWER_SAVER_TDP_MW,
    DEFAULT_TJ_MAX_CELSIUS, IA32_TEMPERATURE_TARGET, IA32_THERM_INTERRUPT, IA32_THERM_STATUS,
    MAX_TDP_LIMIT_MW, MIN_TDP_LIMIT_MW, MSR_PACKAGE_THERM_STATUS, THERMAL_CRITICAL_THRESHOLD_CELSIUS,
    THERMAL_THROTTLE_THRESHOLD_CELSIUS, THERMAL_WARN_THRESHOLD_CELSIUS, THERM_STATUS_CRITICAL,
    THERM_STATUS_CRITICAL_LOG, THERM_STATUS_PROCHOT, THERM_STATUS_PROCHOT_LOG,
    THERM_STATUS_READOUT_MASK, THERM_STATUS_THROTTLE, THERM_STATUS_THROTTLE_LOG,
    THERM_STATUS_VALID,
};

pub use parking::*;
pub use cstate::*;
pub use epb::*;
pub use rapl::{
    verify_rapl_and_smu_structures, AmdSmuMailbox, CpuPowerVendor, EnergyCounter, PkgPowerLimit,
    RaplUnits, AMD_SMU_MSG_GET_CORE_TEMP, AMD_SMU_MSG_GET_PACKAGE_POWER,
    AMD_SMU_MSG_SET_FAST_PPT_LIMIT, AMD_SMU_MSG_SET_SLOW_PPT_LIMIT, AMD_SMU_MSG_TEST_MESSAGE,
    AMD_SMU_RES_BUSY, AMD_SMU_RES_FAILED, AMD_SMU_RES_OK, AMD_SMU_RES_UNKNOWN_CMD,
    MSR_DRAM_ENERGY_STATUS, MSR_DRAM_POWER_LIMIT, MSR_PKG_ENERGY_STATUS, MSR_PKG_PERF_STATUS,
    MSR_PKG_POWER_LIMIT, MSR_PP0_ENERGY_STATUS, MSR_RAPL_POWER_UNIT,
};

/// Comprehensive verification self-test suite for Dynamic Power Governor & Thermal Energy Meter.
pub fn test_power_governor_subsystem() -> bool {
    // 1. Verify Intel RAPL MSR structures, energy units, and AMD SMU mailbox
    if !verify_rapl_and_smu_structures() {
        return false;
    }

    // 2. Verify Digital Thermal Sensor (DTS) monitoring and thermal throttle flags
    if !verify_thermal_monitoring() {
        return false;
    }

    // 3. Verify closed-loop PID power governor and TDP limits (5W-30W)
    if !verify_pid_power_governor() {
        return false;
    }

    // 4. Integrated loopback test across steady, high-load, and thermal throttling regimes
    let units = RaplUnits::default_intel();
    let mut energy_counter = EnergyCounter::new();
    let mut thermal_engine = ThermalSensorEngine::new(true);
    let mut governor = PidPowerGovernor::new(GovernorProfile::Balanced);

    // Initial energy reading at boot
    energy_counter.update(0x1000_0000, &units);

    // Step 1: Steady 15W load (15,000 mW) at 65°C over 100ms
    let ticks_15w = 98_304u32;
    let delta_uj = energy_counter.update(0x1000_0000 + ticks_15w, &units);
    if delta_uj == 0 {
        return false;
    }

    thermal_engine.update_simulated_temp(0, 65);
    let action_steady = governor.step(15_000, thermal_engine.max_temperature(), 100);
    if action_steady.emergency_thermal_throttle || action_steady.throttle_duty_percent != 0 {
        return false;
    }

    // Step 2: High load excursion (26,000 mW measured @ 88°C)
    thermal_engine.update_simulated_temp(0, 88);
    let action_high = governor.step(26_000, thermal_engine.max_temperature(), 100);
    if action_high.target_pl1_mw >= 15_000 || action_high.throttle_duty_percent == 0 {
        return false;
    }

    // Step 3: Critical thermal surge (98°C) -> emergency override
    thermal_engine.update_simulated_temp(0, 98);
    let action_trip = governor.step(22_000, thermal_engine.max_temperature(), 100);
    if !action_trip.emergency_thermal_throttle {
        return false;
    }
    if action_trip.target_pl1_mw != MIN_TDP_LIMIT_MW || action_trip.throttle_duty_percent != 75 {
        return false;
    }

    // 5. Emit boot sentinel
    crate::serial_write_str("[KNT-POWER-GOV-PASS] Dynamic power governor, RAPL energy meter, and thermal monitor operational\r\n");

    true
}

/// Comprehensive verification self-test suite for Dynamic Core Parking,
/// C-State Governor, and Energy-Performance Bias (EPB / EPP).
pub fn test_core_parking_and_cstate_subsystem() -> bool {
    // 1. Verify CPU core parking topology table, runqueue load evaluation, and park/unpark transitions
    if !verify_core_parking_topology_and_load() {
        return false;
    }

    // 2. Verify autonomous C-state residency selection and break-even latency analysis
    if !verify_cstate_governor_and_latency() {
        return false;
    }

    // 3. Verify Energy-Performance Bias (EPB) and EPP hardware MSR governor profiles
    if !verify_epb_and_epp_profiles() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-POWER-PARK-PASS] Dynamic core parking, C-state governor, and EPB/EPP profiles verified\r\n");

    true
}

/// Comprehensive verification self-test suite for High-Performance S0i3 / S3
/// Instant-Resume Freeze and Wakeup Controller.
///
/// Emits sentinel [KNT-SUSPEND-RESUME-PASS] upon successful validation.
pub fn test_s0i3_s3_suspend_resume_subsystem() -> bool {
    // 1. Verify S0i3/S3 state machine, device quiescence phases, and suspend hooks
    if !verify_quiescence_state_machine() {
        return false;
    }

    // 2. Verify CPU core freeze, context checksum, DRAM self-refresh, and ACPI sleep registers
    if !verify_cpu_freeze_and_dram_self_refresh() {
        return false;
    }

    // 3. Verify sub-450ms atomic wakeup resumption and device context restoration pipeline
    if !verify_atomic_wakeup_pipeline() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-SUSPEND-RESUME-PASS] S0i3/S3 instant-resume freeze and wakeup controller verified\r\n");

    true
}

/// Comprehensive verification self-test suite for Handheld PC Thermal & Fan Profile Controller,
/// Multi-Point Curves, Thermal Hysteresis Damping, and Acoustic Pacing Governor.
///
/// Emits sentinel [KNT-THERMAL-FAN-PASS] upon successful validation.
pub fn test_handheld_thermal_and_fan_subsystem() -> bool {
    // 1. Verify multi-point fan interpolation curves (Linear & Cubic S-curve) and safety clamps
    if !verify_fan_curve_interpolation() {
        return false;
    }

    // 2. Verify thermal hysteresis damping (3°C) and acoustic rate-of-change limiter (ramp up 20%/s, ramp down 5%/s)
    if !verify_thermal_hysteresis_and_rate_limiter() {
        return false;
    }

    // 3. Verify multi-zone thermal aggregation (CPU/GPU/Batt/Skin) and acoustic pacing governor profiles
    if !verify_thermal_governor_and_zones() {
        return false;
    }

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-THERMAL-FAN-PASS] Handheld multi-point fan curves and acoustic thermal governor verified\r\n");

    true
}


/// Comprehensive verification self-test suite for Direct-to-Disk Kernel Hibernation (S4 State)
/// and In-Kernel LZ4 Fast-Restoration Engine.
///
/// Emits sentinel [KNT-POWER-HIBERNATE-PASS] upon successful validation.
pub fn test_power_hibernate_subsystem() -> bool {
    // 1. Verify physical page frame bitmap scanner, memory exclusion, and swap header
    if !verify_page_scanner_and_swap_header() {
        return false;
    }

    // 2. Verify in-kernel LZ4 block compression/decompression and IEEE 802.3 CRC32 verification
    if !verify_lz4_compression_and_crc32() {
        return false;
    }

    // 3. Verify atomic memory restoration trampoline, collision avoidance, and page table rebuild
    if !verify_atomic_restore_trampoline() {
        return false;
    }

    // 4. End-to-end integration loopback test:
    let total_ram_frames = 16_384;
    let swap_lba = 4096;
    let mut controller = HibernateController::new(total_ram_frames, swap_lba);

    if controller.prepare_s4().is_err() {
        return false;
    }

    let active_count = controller.scan_physical_memory(&[(256, 1024), (2048, 1024)]);
    if active_count != 2048 || controller.scanner.excluded_free_frames() != (16_384 - 2048) {
        return false;
    }

    let mut total_compressed_bytes = 0u64;
    let mut image_crc = 0xFFFF_FFFFu32;

    let mut page_a = [0x5Au8; PAGE_SIZE];
    page_a[0..32].copy_from_slice(b"KNTONOS-KERNEL-HEAP-DATA-CHUNK01");
    let comp_a = match Lz4CompressedPage::compress_page(256, &page_a) {
        Ok(c) => c,
        Err(_) => return false,
    };
    total_compressed_bytes += comp_a.compressed_data.len() as u64;
    image_crc = crc32_update(image_crc, &page_a);
    controller.layout_map.allocate_extent(256, comp_a.compressed_data.len() as u32, PAGE_SIZE as u32, comp_a.crc32);

    let mut page_b = [0xA5u8; PAGE_SIZE];
    page_b[0..32].copy_from_slice(b"KNTONOS-KERNEL-STACK-FRAME-DATA2");
    let comp_b = match Lz4CompressedPage::compress_page(2048, &page_b) {
        Ok(c) => c,
        Err(_) => return false,
    };
    total_compressed_bytes += comp_b.compressed_data.len() as u64;
    image_crc = crc32_update(image_crc, &page_b);
    controller.layout_map.allocate_extent(2048, comp_b.compressed_data.len() as u32, PAGE_SIZE as u32, comp_b.crc32);

    let cpu_context = CpuRegisterContext::capture(0x0010_0000, 0x7FFF_E000);
    let entry_rip = 0xFFFF_8000_0010_2000u64;
    let swap_hdr = controller.generate_swap_header(
        cpu_context,
        entry_rip,
        0x0010_0000,
        0x7FFF_E000,
        total_compressed_bytes,
        image_crc,
    );

    if !swap_hdr.is_valid() || swap_hdr.active_pages != 2048 {
        return false;
    }
    if controller.state != HibernateState::PoweringOffS4 {
        return false;
    }

    controller.state = HibernateState::RestoringFromDisk;
    let trampoline_cr3 = 0x0008_0000;
    let mut trampoline_engine = RestoreTrampolineEngine::new(trampoline_cr3);

    controller.state = HibernateState::DecompressingMemory;
    let mut staged_page_a = [0u8; PAGE_SIZE];
    if comp_a.decompress_page(&mut staged_page_a).is_err() || staged_page_a != page_a {
        return false;
    }
    if trampoline_engine.stage_page(0x2000_0000, 256, &staged_page_a).is_err() {
        return false;
    }

    let mut staged_page_b = [0u8; PAGE_SIZE];
    if comp_b.decompress_page(&mut staged_page_b).is_err() || staged_page_b != page_b {
        return false;
    }
    if trampoline_engine.stage_page(0x2000_1000, 2048, &staged_page_b).is_err() {
        return false;
    }

    if trampoline_engine.staging_map.detect_collisions(100, 100) != 0 {
        return false;
    }

    controller.state = HibernateState::AtomicRestoring;
    let mut restored_dest_a = [0u8; PAGE_SIZE];
    if trampoline_engine.execute_atomic_page_restore(&mut restored_dest_a, 256, &staged_page_a, comp_a.crc32).is_err() {
        return false;
    }
    let mut restored_dest_b = [0u8; PAGE_SIZE];
    if trampoline_engine.execute_atomic_page_restore(&mut restored_dest_b, 2048, &staged_page_b, comp_b.crc32).is_err() {
        return false;
    }

    if restored_dest_a != page_a || restored_dest_b != page_b {
        return false;
    }

    if !trampoline_engine.verify_and_prepare_cpu_resume(&swap_hdr.cpu_context) {
        return false;
    }

    controller.state = HibernateState::ResumedS0;

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-POWER-HIBERNATE-PASS] Direct-to-disk kernel hibernation and LZ4 fast-restore verified\r\n");

    true
}
