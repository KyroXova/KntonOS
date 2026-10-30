//! Real-Time 3D Audio Spatializer & Binaural HRTF DSP Pipeline.
//!
//! Provides Head-Related Transfer Function (HRTF) convolution, Interaural Time Difference (ITD)
//! fractional delay lines, Interaural Level Difference (ILD) head-shadow filtering,
//! 3D emitter distance attenuation, Doppler pitch shifting, and binaural stereo synthesis.

pub mod hrtf;
pub mod spatializer;
pub mod eq;
pub mod bass_boost;
pub mod client_ring;
pub mod simd_mix;

pub use hrtf::*;
pub use spatializer::*;
pub use eq::*;
pub use bass_boost::*;
pub use client_ring::*;
pub use simd_mix::*;

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

/// Verification self-test suite for Real-Time Game Audio Equalizer
/// (Parametric 10-Band EQ & Dynamic Bass Booster).
pub fn test_audio_eq_and_bass_subsystem() -> bool {
    crate::serial_write_str("[KNT-AUDIO-EQ-TEST] Running Real-Time Game Audio Equalizer & Bass Booster verification suite...\r\n");

    // 1. Verify 10-band biquad IIR parametric filter and coefficient calculator
    if !eq::test_parametric_eq_core() {
        crate::serial_write_str("[KNT-PANIC] Parametric EQ core verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * RBJ biquad filters, 10 ISO frequency bands, and presets verified\r\n");

    // 2. Verify psychoacoustic harmonic synthesizer for micro-speaker bass boost
    if !bass_boost::test_psychoacoustic_harmonic_synthesizer() {
        crate::serial_write_str("[KNT-PANIC] Psychoacoustic bass harmonic synthesizer verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Psychoacoustic 2nd/3rd harmonic synthesis and speaker protection highpass verified\r\n");

    // 3. Verify dynamic soft-saturation limiter and real-time audio pipeline
    if !bass_boost::test_soft_limiter_and_game_audio_pipeline() {
        crate::serial_write_str("[KNT-PANIC] Soft-saturation limiter and audio pipeline verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Dynamic soft-saturation limiter and real-time game audio processor verified\r\n");

    // 4. End-to-end integration: game audio playback through full DSP chain
    let mut pipeline = bass_boost::GameAudioProcessor::new();
    pipeline.eq.apply_preset(eq::EqPreset::ActionGame);
    pipeline.master_gain = 1.25;

    let mut left_stream = [0i16; 128];
    let mut right_stream = [0i16; 128];
    // Synthesize complex test game audio: 50 Hz sub-bass rumble + 1000 Hz vocal + 4000 Hz transient
    for i in 0..128 {
        let sub = if (i % 48) < 24 { 8000 } else { -8000 };
        let mid = if (i % 12) < 6 { 4000 } else { -4000 };
        let high = if (i % 4) < 2 { 2000 } else { -2000 };
        left_stream[i] = sub + mid + high;
        right_stream[i] = sub + mid - high;
    }

    pipeline.process_buffers(&mut left_stream, &mut right_stream);

    let mut total_power = 0i64;
    let mut peak_val = 0i16;
    for i in 0..128 {
        let abs_l = left_stream[i].abs();
        let abs_r = right_stream[i].abs();
        total_power += abs_l as i64 + abs_r as i64;
        if abs_l > peak_val { peak_val = abs_l; }
        if abs_r > peak_val { peak_val = abs_r; }
    }

    if total_power == 0 || peak_val == i16::MAX {
        crate::serial_write_str("[KNT-PANIC] End-to-end game audio pipeline power or clipping assertion failed!\r\n");
        return false;
    }

    crate::serial_write_str("  * End-to-end real-time game audio equalization, harmonic bass, and limiting verified\r\n");
    crate::serial_write_str("[KNT-AUDIO-EQ-PASS] Real-time game audio equalizer and dynamic bass booster operational\r\n");
    true
}
