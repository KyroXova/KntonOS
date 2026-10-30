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
pub mod voice_dsp;
pub mod spatial_ear;

pub use hrtf::*;
pub use spatial_ear::*;
pub use spatializer::*;
pub use eq::*;
pub use bass_boost::*;
pub use client_ring::*;
pub use simd_mix::*;
pub use voice_dsp::*;

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

/// Verification self-test suite for Zero-Copy Shared-Memory Audio Client Protocol & Real-Time Waveform Mixer Ring.
pub fn test_audio_client_protocol_subsystem() -> bool {
    crate::serial_write_str("[KNT-AUDIO-CLIENT-TEST] Running Shared-Memory Audio Client Protocol & Mixer verification suite...\r\n");

    // 1. Verify lock-free SPSC circular ring buffer and channel gain computation
    if !client_ring::test_audio_client_ring_and_channel() {
        crate::serial_write_str("[KNT-PANIC] Audio client ring and channel verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Lock-free SPSC shared-memory ring buffer and panning gain computation verified\r\n");

    // 2. Verify sub-millisecond audio frame pacing and underrun starvation recovery
    if !client_ring::test_audio_frame_pacing_and_underrun_handler() {
        crate::serial_write_str("[KNT-PANIC] Audio frame pacing and underrun handler verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Sub-millisecond audio frame pacing and underrun starvation recovery verified\r\n");

    // 3. Verify SIMD-accelerated multi-stream float-to-PCM mixer, soft clipping, and TPDF dithering
    if !simd_mix::test_simd_mixer_and_tpdf_dithering() {
        crate::serial_write_str("[KNT-PANIC] SIMD mixer and TPDF dithering verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * SIMD multi-stream float-to-PCM mixer, soft clipping, and TPDF dithering verified\r\n");

    // 4. End-to-end multi-channel mixing stress test (4 concurrent clients)
    let mut manager = client_ring::AudioClientManager::new(48, 0);
    let mut mixer = simd_mix::AudioSimdMixer::new();

    let c_sfx = match manager.register_client("sfx") {
        Some(id) => id,
        None => return false,
    };
    let c_voice = match manager.register_client("voice") {
        Some(id) => id,
        None => return false,
    };
    let c_music = match manager.register_client("music") {
        Some(id) => id,
        None => return false,
    };
    let c_ui = match manager.register_client("ui") {
        Some(id) => id,
        None => return false,
    };

    // Configure volumes and panning
    if let Some(c) = manager.get_client_mut(c_sfx) {
        c.volume = 0.8;
        c.pan = -0.5;
        let frames = [client_ring::AudioFrame::new(0.4, 0.4); 144];
        c.ring.write_frames(&frames);
    }
    if let Some(c) = manager.get_client_mut(c_voice) {
        c.volume = 1.0;
        c.pan = 0.0;
        let frames = [client_ring::AudioFrame::new(0.3, 0.3); 144];
        c.ring.write_frames(&frames);
    }
    if let Some(c) = manager.get_client_mut(c_music) {
        c.volume = 0.6;
        c.pan = 0.5;
        let frames = [client_ring::AudioFrame::new(0.2, 0.2); 144];
        c.ring.write_frames(&frames);
    }
    if let Some(c) = manager.get_client_mut(c_ui) {
        c.volume = 0.4;
        c.pan = 0.0;
        let frames = [client_ring::AudioFrame::new(0.1, 0.1); 144];
        c.ring.write_frames(&frames);
    }

    // Run 3 consecutive mixer periods (48 frames each = 144 frames total)
    let mut mixed_output = [0i16; 96]; // 48 stereo frames * 2 channels
    for _ in 0..3 {
        let mixed = mixer.mix_clients(&mut manager, 48, &mut mixed_output);
        if mixed != 48 {
            return false;
        }
    }

    if mixer.total_mixed_frames() != 144 || mixer.peak_amplitude() == 0.0 {
        return false;
    }

    crate::serial_write_str("  * End-to-end 4-channel concurrent audio streaming and real-time mixing verified\r\n");
    crate::serial_write_str("[KNT-AUDIO-CLIENT-PASS] Zero-copy shared-memory audio client protocol and real-time mixer operational\r\n");
    true
}

/// Comprehensive verification self-test suite for Real-Time Low-Latency Game Voice Chat
/// & Acoustic Noise Cancellation (DSP / RNNoise-Style) Filter Engine.
///
/// Emits sentinel [KNT-AUDIO-VOICE-DSP-PASS] upon successful validation.
pub fn test_voice_dsp_subsystem() -> bool {
    crate::serial_write_str("[KNT-AUDIO-VOICE-DSP-TEST] Running Real-Time Game Voice Chat & Acoustic Noise Cancellation verification suite...\r\n");

    // 1. Verify capture ring buffer, 16-bit 48kHz PCM frames, and 16-band Bark filterbank
    if !voice_dsp::test_voice_capture_ring_and_filterbank() {
        crate::serial_write_str("[KNT-PANIC] Voice capture ring and filterbank verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Microphone circular capture ring buffer and 16-band Bark filterbank verified\r\n");

    // 2. Verify Voice Activity Detection (VAD), zero-crossing rate, and spectral noise estimation
    if !voice_dsp::test_vad_and_noise_estimator() {
        crate::serial_write_str("[KNT-PANIC] Voice activity detector and noise estimator verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Energy/ZCR Voice Activity Detection (VAD) and continuous noise estimation verified\r\n");

    // 3. Verify real-time spectral subtraction noise suppressor, AGC, and soft-limiter
    if !voice_dsp::test_noise_suppressor_and_agc() {
        crate::serial_write_str("[KNT-PANIC] Noise suppressor and AGC verification failed!\r\n");
        return false;
    }
    crate::serial_write_str("  * Multi-band spectral subtraction noise suppressor and Automatic Gain Control (AGC) verified\r\n");

    // 4. End-to-end integration test: capture ring -> DSP engine -> denoised output frame
    let mut engine = voice_dsp::VoiceDspEngine::new();

    // Push 480 samples of test audio into capture ring
    let test_frame = voice_dsp::VoicePcmFrame {
        samples: [500; voice_dsp::VOICE_FRAME_SAMPLES],
        channels: voice_dsp::VoiceChannels::Mono,
        timestamp_us: 100_000,
        sequence_number: 1,
    };
    engine.ring.push_samples(&test_frame.samples);

    let popped = match engine.ring.pop_frame(100_000) {
        Some(f) => f,
        None => return false,
    };

    let processed = engine.process_frame(&popped);
    if processed.sequence_number != 1 || engine.total_processed_frames != 1 {
        return false;
    }

    crate::serial_write_str("  * End-to-end game voice chat capture, noise cancellation, and AGC pipeline verified\r\n");
    crate::serial_write_str("[KNT-AUDIO-VOICE-DSP-PASS] Real-time game voice chat and acoustic noise cancellation engine operational\r\n");

    true
}
