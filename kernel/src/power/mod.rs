//! Dynamic Power Governor & Thermal Energy Meter Subsystem for KntonOS.
//!
//! Provides Intel RAPL MSR energy metering, AMD SMU power telemetry,
//! digital thermal sensor monitoring, and closed-loop PID TDP governors.

pub mod rapl;

pub use rapl::{
    verify_rapl_and_smu_structures, AmdSmuMailbox, CpuPowerVendor, EnergyCounter, PkgPowerLimit,
    RaplUnits, AMD_SMU_MSG_GET_CORE_TEMP, AMD_SMU_MSG_GET_PACKAGE_POWER,
    AMD_SMU_MSG_SET_FAST_PPT_LIMIT, AMD_SMU_MSG_SET_SLOW_PPT_LIMIT, AMD_SMU_MSG_TEST_MESSAGE,
    AMD_SMU_RES_BUSY, AMD_SMU_RES_FAILED, AMD_SMU_RES_OK, AMD_SMU_RES_UNKNOWN_CMD,
    MSR_DRAM_ENERGY_STATUS, MSR_DRAM_POWER_LIMIT, MSR_PKG_ENERGY_STATUS, MSR_PKG_PERF_STATUS,
    MSR_PKG_POWER_LIMIT, MSR_PP0_ENERGY_STATUS, MSR_RAPL_POWER_UNIT,
};
