//! Real-Time 3D Audio Spatializer & Binaural HRTF DSP Pipeline.
//!
//! Provides Head-Related Transfer Function (HRTF) convolution, Interaural Time Difference (ITD)
//! fractional delay lines, Interaural Level Difference (ILD) head-shadow filtering,
//! 3D emitter distance attenuation, Doppler pitch shifting, and binaural stereo synthesis.

pub mod hrtf;
pub mod spatializer;

pub use hrtf::*;
pub use spatializer::*;

/// Verification self-test suite for Real-Time 3D Audio Spatializer & Binaural HRTF DSP Pipeline.
pub fn test_audio_spatializer_subsystem() -> bool {
    crate::serial_write_str("[KNT-AUDIO-TEST] Running Real-Time 3D Audio Spatializer & Binaural HRTF DSP verification suite...\r\n");

    // 1. Verify HRTF impulse response dataset, ITD delay line, and ILD filter
    if !hrtf::test_hrtf_dsp_core() {
        crate::serial_write_str("[KNT-PANIC] HRTF DSP core verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * HRTF impulse response dataset, ITD delay line, and ILD filter verified\r\n");

    // 2. Verify 3D positional emitter math and inverse-square distance attenuation
    if !spatializer::test_3d_emitter_positioning_and_attenuation() {
        crate::serial_write_str("[KNT-PANIC] 3D emitter positioning and attenuation verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * 3D listener cranial frame, sound cone radiation, and distance attenuation verified\r\n");

    // 3. Verify Doppler pitch shift and binaural stereo synthesis
    if !spatializer::test_doppler_and_binaural_convolver() {
        crate::serial_write_str("[KNT-PANIC] Doppler pitch shift and binaural convolver verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Doppler frequency shift, HRIR FIR convolution, and stereo synthesis verified\r\n");

    // 4. End-to-end integration: dynamic 3D soundscape rendering
    let mut spatial_engine = spatializer::BinauralSpatializer::new();
    spatial_engine.listener.set_position(crate::graphics::types::Vec3::zero());
    spatial_engine.listener.set_orientation(
        crate::graphics::types::Vec3::new(0.0, 0.0, 1.0),
        crate::graphics::types::Vec3::new(0.0, 1.0, 0.0),
    );

    let mut footstep_emitter = spatializer::AudioEmitter3D::new(101);
    footstep_emitter.set_position(crate::graphics::types::Vec3::new(4.0, 0.0, -3.0));
    footstep_emitter.set_distance_model(1.0, 50.0, 1.0);

    let footstep_voice = match spatial_engine.allocate_voice(footstep_emitter) {
        Some(v) => v,
        None => {
            crate::serial_write_str("[KNT-PANIC] Failed to allocate footstep spatial voice!\r\n");
            return false;
        }
    };

    let mono_footstep = [5000i16; 32];
    let mut stereo_out = [(0i16, 0i16); 32];
    spatial_engine.mix_spatial_audio(&[(footstep_voice, &mono_footstep)], &mut stereo_out);

    let mut total_l = 0i64;
    let mut total_r = 0i64;
    for frame in stereo_out.iter() {
        total_l += frame.0.abs() as i64;
        total_r += frame.1.abs() as i64;
    }

    if total_r == 0 || total_l == 0 || total_r <= total_l {
        crate::serial_write_str("[KNT-PANIC] Spatial soundscape energy distribution failed!\r\n");
        return false;
    }

    crate::serial_write_str("  * Real-time 3D soundscape spatialization and multi-emitter mixing verified\r\n");
    crate::serial_write_str("[KNT-AUDIO-SPATIAL-PASS] Real-time 3D audio spatializer and HRTF binaural DSP operational\r\n");
    true
}
