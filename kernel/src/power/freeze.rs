//! S0i3 Modern Standby & S3 Deep Sleep System Quiescence Coordinator.
//!
//! Provides the system sleep state machine, device quiescence phases,
//! driver suspend/resume callback registrations, and device state capture
//! for zero-packet-loss / zero-frame-drop instant sleep transitions.

pub const MAX_SUSPEND_DEVICES: usize = 16;

/// High-level system target sleep states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepTarget {
    /// S0i3 Modern Standby / Low Power S0 Idle (sub-state with ultra-fast wakeup).
    S0i3ModernStandby,
    /// S3 Suspend to RAM (system context preserved in DRAM self-refresh).
    S3SuspendToRam,
}

/// System quiescence transition lifecycle phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuiescencePhase {
    /// System is actively running in S0.
    ActiveS0,
    /// Phase 1: Notifying userland processes and synchronizing filesystems (freeze userspace).
    UserlandFreeze,
    /// Phase 2: Quiescing audio DMA streaming engines to prevent buffer underrun buzzing.
    AudioQuiesce,
    /// Phase 3: Flushing and parking NVMe / storage controller command queues.
    StoragePark,
    /// Phase 4: Suspending USB xHCI ports and placing controllers into selective suspend.
    UsbSelectiveSuspend,
    /// Phase 5: Disabling display scanout planes and entering panel self-refresh (PSR).
    DisplayScanoutDisable,
    /// Phase 6: All devices quiesced; ready for CPU core freeze and DRAM self-refresh.
    SystemFrozen,
    /// Resumption Phase 1: Waking up display planes and restoring scanout timings.
    DisplayScanoutRestore,
    /// Resumption Phase 2: Restoring USB host controllers and root ports.
    UsbResume,
    /// Resumption Phase 3: Unparking NVMe queues and submitting doorbells.
    StorageUnpark,
    /// Resumption Phase 4: Restarting audio DMA streaming engines and mixers.
    AudioResume,
    /// Resumption Phase 5: Thawing userland threads and scheduler runqueues.
    UserlandThaw,
}

/// Device classification types for phased suspend/resume sequencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    AudioDma,
    StorageNvme,
    UsbXhci,
    DisplayGpu,
    NetworkNic,
    InputDevice,
}

/// Status of an individual device during power transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePowerState {
    D0Active,
    D1LowPower,
    D2Standby,
    D3Hot,
    D3ColdSuspended,
}

/// Hardware context descriptor captured prior to entering low-power states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceContextSnapshot {
    pub device_id: u16,
    pub class: DeviceClass,
    pub power_state: DevicePowerState,
    pub register_snapshot: [u32; 4],
    pub state_flags: u32,
    pub latency_quiesce_ns: u64,
    pub latency_restore_ns: u64,
}

impl DeviceContextSnapshot {
    pub const fn empty(device_id: u16, class: DeviceClass) -> Self {
        Self {
            device_id,
            class,
            power_state: DevicePowerState::D0Active,
            register_snapshot: [0u32; 4],
            state_flags: 0,
            latency_quiesce_ns: 0,
            latency_restore_ns: 0,
        }
    }
}

/// Device quiescence hook interface.
pub trait SuspendableDevice {
    fn device_id(&self) -> u16;
    fn class(&self) -> DeviceClass;
    fn quiesce(&mut self) -> Result<DeviceContextSnapshot, QuiesceError>;
    fn restore(&mut self, snapshot: &DeviceContextSnapshot) -> Result<(), QuiesceError>;
}

/// Errors occurring during sleep transition quiescence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuiesceError {
    DeviceBusy,
    Timeout,
    InvalidPhaseTransition,
    DeviceNotFound,
    DeviceTableFull,
    HardwareFault,
    RestorationMismatch,
}

/// Registered device entry in the coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredDevice {
    pub device_id: u16,
    pub class: DeviceClass,
    pub power_state: DevicePowerState,
    pub context: DeviceContextSnapshot,
    pub is_registered: bool,
}

impl RegisteredDevice {
    pub const fn blank() -> Self {
        Self {
            device_id: 0,
            class: DeviceClass::InputDevice,
            power_state: DevicePowerState::D0Active,
            context: DeviceContextSnapshot::empty(0, DeviceClass::InputDevice),
            is_registered: false,
        }
    }
}

/// System Quiescence Coordinator managing multi-phase device freeze and resume.
pub struct QuiescenceCoordinator {
    pub target: SleepTarget,
    pub current_phase: QuiescencePhase,
    pub devices: [RegisteredDevice; MAX_SUSPEND_DEVICES],
    pub device_count: usize,
    pub userland_frozen: bool,
    pub total_quiesce_latency_us: u32,
    pub total_restore_latency_us: u32,
    pub transition_count: u64,
}

impl QuiescenceCoordinator {
    pub const fn new(target: SleepTarget) -> Self {
        Self {
            target,
            current_phase: QuiescencePhase::ActiveS0,
            devices: [RegisteredDevice::blank(); MAX_SUSPEND_DEVICES],
            device_count: 0,
            userland_frozen: false,
            total_quiesce_latency_us: 0,
            total_restore_latency_us: 0,
            transition_count: 0,
        }
    }

    /// Registers a hardware device into the sleep coordinator table.
    pub fn register_device(&mut self, device_id: u16, class: DeviceClass) -> Result<usize, QuiesceError> {
        if self.device_count >= MAX_SUSPEND_DEVICES {
            return Err(QuiesceError::DeviceTableFull);
        }
        for (i, dev) in self.devices.iter_mut().enumerate() {
            if !dev.is_registered {
                dev.device_id = device_id;
                dev.class = class;
                dev.power_state = DevicePowerState::D0Active;
                dev.context = DeviceContextSnapshot::empty(device_id, class);
                dev.is_registered = true;
                self.device_count += 1;
                return Ok(i);
            }
        }
        Err(QuiesceError::DeviceTableFull)
    }

    /// Executes the complete deterministic device quiescence pipeline.
    ///
    /// Sequencing:
    /// 1. Userland threads frozen & sync
    /// 2. Audio DMA engines halted
    /// 3. NVMe storage queues parked
    /// 4. USB xHCI ports suspended
    /// 5. Display scanout disabled
    /// 6. System entering frozen state ready for CPU suspend
    pub fn execute_quiescence_pipeline(&mut self) -> Result<(), QuiesceError> {
        if self.current_phase != QuiescencePhase::ActiveS0 {
            return Err(QuiesceError::InvalidPhaseTransition);
        }

        // Phase 1: Freeze Userland
        self.current_phase = QuiescencePhase::UserlandFreeze;
        self.userland_frozen = true;

        // Phase 2: Audio DMA Quiesce
        self.current_phase = QuiescencePhase::AudioQuiesce;
        self.quiesce_class(DeviceClass::AudioDma, 0x1100)?;

        // Phase 3: Storage NVMe Park
        self.current_phase = QuiescencePhase::StoragePark;
        self.quiesce_class(DeviceClass::StorageNvme, 0x2200)?;

        // Phase 4: USB Selective Suspend
        self.current_phase = QuiescencePhase::UsbSelectiveSuspend;
        self.quiesce_class(DeviceClass::UsbXhci, 0x3300)?;

        // Phase 5: Display Scanout Disable
        self.current_phase = QuiescencePhase::DisplayScanoutDisable;
        self.quiesce_class(DeviceClass::DisplayGpu, 0x4400)?;

        // Phase 6: System Frozen
        self.current_phase = QuiescencePhase::SystemFrozen;
        self.transition_count += 1;

        Ok(())
    }

    /// Helper to capture context and transition devices of a specific class.
    fn quiesce_class(&mut self, class: DeviceClass, mock_reg_base: u32) -> Result<(), QuiesceError> {
        for dev in self.devices.iter_mut() {
            if dev.is_registered && dev.class == class {
                dev.power_state = DevicePowerState::D3ColdSuspended;
                dev.context.power_state = DevicePowerState::D3ColdSuspended;
                dev.context.register_snapshot = [
                    mock_reg_base,
                    mock_reg_base + 1,
                    mock_reg_base + 2,
                    mock_reg_base + 3,
                ];
                dev.context.state_flags = 0x8000_0001; // Suspended + Valid
                dev.context.latency_quiesce_ns = 50_000; // 50 microseconds
                self.total_quiesce_latency_us += 50;
            }
        }
        Ok(())
    }

    /// Resumes devices in reverse order to restore full S0 state.
    pub fn execute_restore_pipeline(&mut self) -> Result<(), QuiesceError> {
        if self.current_phase != QuiescencePhase::SystemFrozen {
            return Err(QuiesceError::InvalidPhaseTransition);
        }

        // Phase 1: Restore Display
        self.current_phase = QuiescencePhase::DisplayScanoutRestore;
        self.restore_class(DeviceClass::DisplayGpu)?;

        // Phase 2: Restore USB
        self.current_phase = QuiescencePhase::UsbResume;
        self.restore_class(DeviceClass::UsbXhci)?;

        // Phase 3: Restore Storage
        self.current_phase = QuiescencePhase::StorageUnpark;
        self.restore_class(DeviceClass::StorageNvme)?;

        // Phase 4: Restore Audio
        self.current_phase = QuiescencePhase::AudioResume;
        self.restore_class(DeviceClass::AudioDma)?;

        // Phase 5: Thaw Userland
        self.current_phase = QuiescencePhase::UserlandThaw;
        self.userland_frozen = false;

        // Active S0 restored
        self.current_phase = QuiescencePhase::ActiveS0;
        Ok(())
    }

    fn restore_class(&mut self, class: DeviceClass) -> Result<(), QuiesceError> {
        for dev in self.devices.iter_mut() {
            if dev.is_registered && dev.class == class {
                if dev.context.state_flags & 0x8000_0000 == 0 {
                    return Err(QuiesceError::RestorationMismatch);
                }
                dev.power_state = DevicePowerState::D0Active;
                dev.context.power_state = DevicePowerState::D0Active;
                dev.context.latency_restore_ns = 40_000; // 40 microseconds
                self.total_restore_latency_us += 40;
            }
        }
        Ok(())
    }
}

/// Verification self-test for S0i3/S3 state machine and device quiescence phases.
pub fn verify_quiescence_state_machine() -> bool {
    let mut coordinator = QuiescenceCoordinator::new(SleepTarget::S3SuspendToRam);

    // Register representative devices across all 4 critical hardware domains
    let audio_id = coordinator.register_device(0x101, DeviceClass::AudioDma);
    let nvme_id = coordinator.register_device(0x202, DeviceClass::StorageNvme);
    let usb_id = coordinator.register_device(0x303, DeviceClass::UsbXhci);
    let gpu_id = coordinator.register_device(0x404, DeviceClass::DisplayGpu);

    if audio_id.is_err() || nvme_id.is_err() || usb_id.is_err() || gpu_id.is_err() {
        return false;
    }
    if coordinator.device_count != 4 {
        return false;
    }

    // Execute Quiescence Pipeline
    if coordinator.execute_quiescence_pipeline().is_err() {
        return false;
    }

    if coordinator.current_phase != QuiescencePhase::SystemFrozen {
        return false;
    }
    if !coordinator.userland_frozen {
        return false;
    }

    // All registered devices must now be in D3ColdSuspended
    for dev in &coordinator.devices[..coordinator.device_count] {
        if dev.power_state != DevicePowerState::D3ColdSuspended {
            return false;
        }
        if dev.context.register_snapshot[0] == 0 {
            return false;
        }
    }

    // Execute Restore Pipeline
    if coordinator.execute_restore_pipeline().is_err() {
        return false;
    }

    if coordinator.current_phase != QuiescencePhase::ActiveS0 {
        return false;
    }
    if coordinator.userland_frozen {
        return false;
    }

    // All registered devices restored to D0Active
    for dev in &coordinator.devices[..coordinator.device_count] {
        if dev.power_state != DevicePowerState::D0Active {
            return false;
        }
    }

    // Check quiesce / restore latencies
    if coordinator.total_quiesce_latency_us == 0 || coordinator.total_restore_latency_us == 0 {
        return false;
    }

    true
}
