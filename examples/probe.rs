use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let host = cpal::default_host();

    println!("=== input devices ===");
    match host.input_devices() {
        Ok(devs) => {
            for d in devs {
                let name = d.name().unwrap_or_else(|e| format!("<name error: {e}>"));
                let cfg = d
                    .default_input_config()
                    .map(|c| format!("{:?} {}Hz {}ch", c.sample_format(), c.sample_rate().0, c.channels()))
                    .unwrap_or_else(|e| format!("<config error: {e}>"));
                println!("  {name}  [{cfg}]");
            }
        }
        Err(e) => println!("  <input_devices error: {e}>"),
    }

    println!();
    println!("=== default input device ===");
    let device = match host.default_input_device() {
        Some(d) => {
            let name = d.name().unwrap_or_else(|e| format!("<err: {e}>"));
            println!("  name: {name}");
            d
        }
        None => {
            println!("  <none>");
            return;
        }
    };

    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            println!("  default_input_config error: {e}");
            return;
        }
    };
    println!("  format: {:?} {}Hz {}ch",
             supported.sample_format(),
             supported.sample_rate().0,
             supported.channels());

    println!();
    println!("=== probe stream for 500ms ===");
    let cfg: cpal::StreamConfig = supported.clone().into();
    let callback_count = Arc::new(AtomicUsize::new(0));
    let sample_count = Arc::new(AtomicUsize::new(0));
    let nonzero_count = Arc::new(AtomicUsize::new(0));

    let cc = callback_count.clone();
    let sc = sample_count.clone();
    let nz = nonzero_count.clone();

    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &cfg,
            move |data: &[f32], _| {
                cc.fetch_add(1, Ordering::Relaxed);
                sc.fetch_add(data.len(), Ordering::Relaxed);
                nz.fetch_add(data.iter().filter(|s| s.abs() > 1e-6).count(), Ordering::Relaxed);
            },
            |e| eprintln!("  stream error: {e}"),
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &cfg,
            move |data: &[i16], _| {
                cc.fetch_add(1, Ordering::Relaxed);
                sc.fetch_add(data.len(), Ordering::Relaxed);
                nz.fetch_add(data.iter().filter(|s| **s != 0).count(), Ordering::Relaxed);
            },
            |e| eprintln!("  stream error: {e}"),
            None,
        ),
        fmt => {
            println!("  unsupported sample format: {fmt:?}");
            return;
        }
    };

    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            println!("  build_input_stream error: {e}");
            return;
        }
    };

    if let Err(e) = stream.play() {
        println!("  stream.play error: {e}");
        return;
    }

    std::thread::sleep(Duration::from_millis(500));
    drop(stream);

    println!("  callbacks fired: {}", callback_count.load(Ordering::Relaxed));
    println!("  samples received: {}", sample_count.load(Ordering::Relaxed));
    println!("  non-zero samples: {}", nonzero_count.load(Ordering::Relaxed));
}
