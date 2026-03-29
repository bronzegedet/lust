use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::f32::consts::TAU;
use std::sync::OnceLock;

#[derive(Debug)]
struct AudioState {
    phase: f32,
    freq: f32,
    gain: f32,
    playing: bool,
    sample_rate: f32,
}

struct AudioEngine {
    state: std::sync::Arc<std::sync::Mutex<AudioState>>,
    _stream: cpal::Stream,
}

static AUDIO_ENGINE: OnceLock<std::sync::Mutex<Option<AudioEngine>>> = OnceLock::new();

fn audio_slot() -> &'static std::sync::Mutex<Option<AudioEngine>> {
    AUDIO_ENGINE.get_or_init(|| std::sync::Mutex::new(None))
}

fn write_frame(state: &std::sync::Arc<std::sync::Mutex<AudioState>>) -> f32 {
    let mut state = state.lock().unwrap();
    if !state.playing || state.gain <= 0.0 {
        return 0.0;
    }
    let sample = (state.phase * TAU).sin() * state.gain;
    state.phase = (state.phase + (state.freq / state.sample_rate)).fract();
    sample
}

fn build_stream_f32(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: std::sync::Arc<std::sync::Mutex<AudioState>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |data: &mut [f32], _| {
                for frame in data.chunks_mut(channels) {
                    let sample = write_frame(&state);
                    for out in frame.iter_mut() {
                        *out = sample;
                    }
                }
            },
            move |err| eprintln!("audio stream error: {}", err),
            None,
        )
        .map_err(|e| format!("audio stream build failed: {}", e))
}

fn build_stream_i16(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: std::sync::Arc<std::sync::Mutex<AudioState>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |data: &mut [i16], _| {
                for frame in data.chunks_mut(channels) {
                    let sample = (write_frame(&state) * i16::MAX as f32) as i16;
                    for out in frame.iter_mut() {
                        *out = sample;
                    }
                }
            },
            move |err| eprintln!("audio stream error: {}", err),
            None,
        )
        .map_err(|e| format!("audio stream build failed: {}", e))
}

fn build_stream_u16(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: std::sync::Arc<std::sync::Mutex<AudioState>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |data: &mut [u16], _| {
                for frame in data.chunks_mut(channels) {
                    let sample = (((write_frame(&state) * 0.5) + 0.5) * u16::MAX as f32) as u16;
                    for out in frame.iter_mut() {
                        *out = sample;
                    }
                }
            },
            move |err| eprintln!("audio stream error: {}", err),
            None,
        )
        .map_err(|e| format!("audio stream build failed: {}", e))
}

pub fn audio_init_native() -> Result<(), String> {
    let slot = audio_slot();
    let mut guard = slot.lock().unwrap();
    if guard.is_some() {
        return Ok(());
    }

    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        return Err("audio init failed: no output device".to_string());
    };
    let Ok(default_config) = device.default_output_config() else {
        return Err("audio init failed: no default output config".to_string());
    };

    let sample_rate = default_config.sample_rate().0 as f32;
    let stream_config: cpal::StreamConfig = default_config.config();
    let state = std::sync::Arc::new(std::sync::Mutex::new(AudioState {
        phase: 0.0,
        freq: 440.0,
        gain: 0.2,
        playing: false,
        sample_rate,
    }));

    let stream = match default_config.sample_format() {
        cpal::SampleFormat::F32 => build_stream_f32(&device, &stream_config, state.clone()),
        cpal::SampleFormat::I16 => build_stream_i16(&device, &stream_config, state.clone()),
        cpal::SampleFormat::U16 => build_stream_u16(&device, &stream_config, state.clone()),
        other => Err(format!("unsupported sample format: {:?}", other)),
    }?;

    stream
        .play()
        .map_err(|err| format!("audio init failed: {}", err))?;

    *guard = Some(AudioEngine { state, _stream: stream });
    Ok(())
}

pub fn audio_set_freq_native(freq: f64) -> Result<(), String> {
    let slot = audio_slot();
    let mut guard = slot.lock().unwrap();
    if guard.is_none() {
        drop(guard);
        audio_init_native()?;
        guard = slot.lock().unwrap();
    }
    if let Some(engine) = guard.as_ref() {
        let mut state = engine.state.lock().unwrap();
        state.freq = freq.max(1.0) as f32;
    }
    Ok(())
}

pub fn audio_set_gain_native(gain: f64) -> Result<(), String> {
    let slot = audio_slot();
    let mut guard = slot.lock().unwrap();
    if guard.is_none() {
        drop(guard);
        audio_init_native()?;
        guard = slot.lock().unwrap();
    }
    if let Some(engine) = guard.as_ref() {
        let mut state = engine.state.lock().unwrap();
        state.gain = gain.clamp(0.0, 1.0) as f32;
    }
    Ok(())
}

pub fn audio_note_on_native() -> Result<(), String> {
    let slot = audio_slot();
    let mut guard = slot.lock().unwrap();
    if guard.is_none() {
        drop(guard);
        audio_init_native()?;
        guard = slot.lock().unwrap();
    }
    if let Some(engine) = guard.as_ref() {
        let mut state = engine.state.lock().unwrap();
        state.playing = true;
    }
    Ok(())
}

pub fn audio_note_off_native() -> Result<(), String> {
    let slot = audio_slot();
    if let Some(engine) = slot.lock().unwrap().as_ref() {
        let mut state = engine.state.lock().unwrap();
        state.playing = false;
    }
    Ok(())
}
