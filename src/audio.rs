use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct StreamHandle {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    device_rate: u32,
    channels: usize,
}

fn start_recording(name_substring: &str) -> Result<StreamHandle> {
    let device = find_input_device(name_substring).with_context(|| {
        if name_substring.is_empty() {
            "no audio input device available".to_string()
        } else {
            format!("no audio input device matching '{name_substring}' found")
        }
    })?;

    let supported = device
        .default_input_config()
        .context("failed to get default input config")?;

    let device_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let stream_config: cpal::StreamConfig = supported.clone().into();

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let samples_w = samples.clone();
    let err_flag: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let err_w = err_flag.clone();

    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                samples_w.lock().unwrap().extend_from_slice(data);
            },
            move |e| {
                *err_w.lock().unwrap() = Some(format!("{e}"));
            },
            None,
        )?,
        SampleFormat::I16 => {
            let sw = samples.clone();
            let ew = err_flag.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    sw.lock().unwrap()
                        .extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                },
                move |e| {
                    *ew.lock().unwrap() = Some(format!("{e}"));
                },
                None,
            )?
        }
        fmt => bail!("unsupported sample format: {fmt:?}"),
    };

    stream.play().context("failed to start audio stream")?;

    Ok(StreamHandle {
        _stream: stream,
        samples,
        device_rate,
        channels,
    })
}

fn finish_recording(handle: StreamHandle) -> Vec<f32> {
    drop(handle._stream);
    let raw = handle.samples.lock().unwrap().clone();

    // Convert to mono if stereo
    let mono = if handle.channels >= 2 {
        raw.chunks(handle.channels)
            .map(|frame| frame.iter().sum::<f32>() / handle.channels as f32)
            .collect()
    } else {
        raw
    };

    // Resample to 16kHz if needed
    if handle.device_rate == 16000 {
        mono
    } else {
        resample(&mono, handle.device_rate, 16000)
    }
}

/// Find an input device whose name contains `name_substring` (case-insensitive).
/// If `name_substring` is empty, returns the host's default input device.
pub fn find_input_device(name_substring: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if name_substring.is_empty() {
        return host.default_input_device();
    }
    let needle = name_substring.to_lowercase();
    host.input_devices().ok()?.find(|d| {
        d.name()
            .map(|n| n.to_lowercase().contains(&needle))
            .unwrap_or(false)
    })
}

/// Block until an input device matching `name_substring` is available, polling at `poll_interval`.
/// Empty substring means "any default input device". Prints a one-time waiting message and a
/// "microphone detected" message once one appears. No-op if a match is already present.
pub fn wait_for_input_device(name_substring: &str, poll_interval: Duration) {
    if find_input_device(name_substring).is_some() {
        return;
    }
    if name_substring.is_empty() {
        eprintln!("[stt-typer] waiting for microphone...");
    } else {
        eprintln!("[stt-typer] waiting for microphone matching '{name_substring}'...");
    }
    loop {
        std::thread::sleep(poll_interval);
        if find_input_device(name_substring).is_some() {
            eprintln!("[stt-typer] microphone detected");
            return;
        }
    }
}

/// Record audio until `stop` is set to true, or `max_duration` elapses.
/// Returns 16kHz mono f32 samples suitable for Whisper.
pub fn record_until_stopped(
    name_substring: &str,
    stop: Arc<AtomicBool>,
    max_duration: Duration,
) -> Result<Vec<f32>> {
    let handle = start_recording(name_substring)?;
    let start = Instant::now();

    loop {
        if stop.load(Ordering::Relaxed) || start.elapsed() >= max_duration {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    Ok(finish_recording(handle))
}

/// Simple linear interpolation resampler.
fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (input.len() as f64 / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;
        let sample = if idx + 1 < input.len() {
            input[idx] as f64 * (1.0 - frac) + input[idx + 1] as f64 * frac
        } else {
            input[idx.min(input.len() - 1)] as f64
        };
        output.push(sample as f32);
    }
    output
}
