//! Milestone 100: KntonOS Micro-Kernel Unified Gaming Subsystem Health Matrix & Benchmark Supervisor.
//!
//! Provides comprehensive health monitoring, telemetry metrics, readiness scoring,
//! and continuous microsecond latency benchmark profiling across all core gaming subsystems.

#![allow(dead_code)]

pub mod health;

/// Unit verification for subsystem health descriptors and latency metrics.
pub fn test_sys_subsystem() -> bool {
    health::test_subsystem_descriptors_and_latency()
}
