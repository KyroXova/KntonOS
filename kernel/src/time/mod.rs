//! Kernel High-Resolution Timers & Hierarchical Timing Wheel Subsystem.

pub mod hrtimer;
pub mod wheel;

pub use hrtimer::{
    FramePaceResult, FrameRatePreset, GameFramePacer, HrTimeNs, HrTimer, HrTimerCallback,
    HrTimerMode, HrTimerState, PerCpuHrTimerQueue, RbTreeTimerQueue, NS_PER_MS, NS_PER_SEC,
    NS_PER_US,
};
pub use wheel::{
    HardwareTimerMode, HierarchicalTimingWheel, TscDeadlineProgrammer, WheelCallback,
    WheelTimer, LEVEL0_SPAN, LEVEL1_SPAN, WHEEL_LEVEL0_SIZE, WHEEL_UPPER_SIZE,
};
