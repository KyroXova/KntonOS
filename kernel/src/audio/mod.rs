//! Real-Time 3D Audio Spatializer & Binaural HRTF DSP Pipeline.
//!
//! Provides Head-Related Transfer Function (HRTF) convolution, Interaural Time Difference (ITD)
//! fractional delay lines, Interaural Level Difference (ILD) head-shadow filtering,
//! 3D emitter distance attenuation, Doppler pitch shifting, and binaural stereo synthesis.

pub mod hrtf;
pub mod spatializer;

pub use hrtf::*;
pub use spatializer::*;
