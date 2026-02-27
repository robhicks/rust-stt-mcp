use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Record audio from the default input device for `duration` seconds.
/// Returns 16kHz mono f32 samples suitable for Whisper.
pub fn record(duration: Duration) -> Result<Vec<f32>> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("no audio input device available")?;

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
    std::thread::sleep(duration);
    drop(stream);

    let raw = samples.lock().unwrap().clone();

    // Convert to mono if stereo
    let mono = if channels >= 2 {
        raw.chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        raw
    };

    // Resample to 16kHz if needed
    if device_rate == 16000 {
        Ok(mono)
    } else {
        Ok(resample(&mono, device_rate, 16000))
    }
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
