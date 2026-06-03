mod audio;
mod keyboard;
mod transcribe;
mod wav;

use anyhow::{Context, Result, bail};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "stt-typer", about = "Hold right CTRL to speak, release to transcribe and type into the active window")]
struct Args {
    /// Maximum seconds to record (safety cap if key is held too long)
    #[arg(short, long, default_value_t = 30)]
    max_duration: u32,

    /// Language hint passed to Gemini (default: "en")
    #[arg(short, long, default_value = "en")]
    language: String,

    /// Gemini model to use for transcription
    #[arg(short = 'M', long, default_value = "gemini-3.5-flash")]
    model: String,

    /// Substring of the input device name to wait for and record from (case-insensitive).
    /// e.g. `--device ACG2502` waits for that USB device to appear before recording.
    /// Without this flag, uses the system default input device with no waiting.
    #[arg(short = 'd', long)]
    device: Option<String>,
}

/// Play a short beep (800Hz for 200ms) to signal recording start.
fn play_beep() {
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            eprintln!("[stt-typer] no audio output device for beep");
            return;
        }
    };
    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[stt-typer] failed to get output config: {e}");
            return;
        }
    };

    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;
    let stream_config: cpal::StreamConfig = config.clone().into();

    let freq = 800.0_f32;
    let duration = Duration::from_millis(200);
    let total_samples = (sample_rate * duration.as_secs_f32()) as usize;
    let phase = Arc::new(Mutex::new(0usize));
    let done = Arc::new(Mutex::new(false));

    let phase_c = phase.clone();
    let done_c = done.clone();

    let build_result = match config.sample_format() {
        SampleFormat::F32 => device.build_output_stream(
            &stream_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut p = phase_c.lock().unwrap();
                for frame in data.chunks_mut(channels) {
                    let val = if *p < total_samples {
                        (2.0 * std::f32::consts::PI * freq * *p as f32 / sample_rate).sin() * 0.3
                    } else {
                        *done_c.lock().unwrap() = true;
                        0.0
                    };
                    for sample in frame.iter_mut() {
                        *sample = val;
                    }
                    *p += 1;
                }
            },
            |e| eprintln!("[stt-typer] beep stream error: {e}"),
            None,
        ),
        _ => return,
    };

    let stream = match build_result {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[stt-typer] failed to build beep stream: {e}");
            return;
        }
    };

    let _ = stream.play();

    // Wait for beep to finish
    loop {
        std::thread::sleep(Duration::from_millis(10));
        if *done.lock().unwrap() {
            break;
        }
    }
    // Small tail to let the audio buffer flush
    std::thread::sleep(Duration::from_millis(50));
    drop(stream);
}

fn type_text(text: &str) -> Result<()> {
    let status = Command::new("ydotool")
        .args(["type", "--clearmodifiers", "--", text])
        .status()
        .context("failed to run ydotool — is it installed? (sudo dnf install ydotool)")?;

    if !status.success() {
        bail!("ydotool exited with status {status}");
    }
    Ok(())
}

/// Find the ydotoold socket, overriding YDOTOOL_SOCKET if it points to a missing path.
fn detect_ydotool_socket() {
    // If already set and valid, keep it
    if let Ok(existing) = std::env::var("YDOTOOL_SOCKET") {
        if std::path::Path::new(&existing).exists() {
            eprintln!("[stt-typer] using ydotool socket at {existing}");
            return;
        }
        eprintln!("[stt-typer] YDOTOOL_SOCKET={existing} does not exist, searching...");
    }

    // Common socket locations depending on how ydotoold was started
    let mut candidates = vec![
        "/tmp/.ydotool_socket".to_string(),
        "/run/ydotoold/socket".to_string(),
    ];
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        candidates.push(format!("{dir}/.ydotool_socket"));
    }

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            // SAFETY: called at single-threaded startup before any threads are spawned
            unsafe { std::env::set_var("YDOTOOL_SOCKET", path) };
            eprintln!("[stt-typer] found ydotool socket at {path}");
            return;
        }
    }

    eprintln!("[stt-typer] warning: could not find ydotool socket — is ydotoold running?");
}

fn main() -> Result<()> {
    let args = Args::parse();

    let api_key = std::env::var("GEMINI_API_KEY").map_err(|_| {
        anyhow::anyhow!(
            "GEMINI_API_KEY is not set — export your Google AI Studio API key:\n  \
             export GEMINI_API_KEY=your-key-here"
        )
    })?;

    // Preflight checks
    detect_ydotool_socket();

    eprintln!("[stt-typer] using Gemini model {}", args.model);
    let ctx = transcribe::create_context(api_key, args.model.clone())
        .context("failed to initialize Gemini client")?;

    // Check ydotool is available
    let ydotool_check = Command::new("ydotool")
        .args(["type", "--", ""])
        .status();
    match ydotool_check {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("[stt-typer] warning: ydotool test exited with {s} — is ydotoold running? (sudo systemctl start ydotool)"),
        Err(e) => bail!("ydotool not found: {e}\nInstall with: sudo dnf install ydotool && sudo systemctl enable --now ydotool"),
    }

    let devices = keyboard::find_keyboard_devices();
    if devices.is_empty() {
        bail!("no keyboard device found — ensure you are in the 'input' group");
    }
    eprintln!("[stt-typer] found {} keyboard device(s)", devices.len());

    // We need two independent device handles: one for the wait-for-press thread,
    // one for the wait-for-release thread. Re-enumerate to get separate handles.
    let mut press_devices = keyboard::find_keyboard_devices();
    let mut release_devices = keyboard::find_keyboard_devices();
    drop(devices);

    let max_duration = Duration::from_secs(args.max_duration as u64);
    let lang = args.language;
    let device_match = args.device.unwrap_or_default();

    audio::wait_for_input_device(&device_match, Duration::from_secs(2));

    eprintln!("[stt-typer] ready — hold right CTRL to speak, release to stop ({lang}, max {}s)",
             args.max_duration);

    loop {
        // Wait for right CTRL press (no timeout — wait forever)
        match keyboard::wait_for_right_ctrl(&mut press_devices, Duration::from_secs(86400)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                eprintln!("[stt-typer] keyboard error: {e}");
                eprintln!("[stt-typer] re-enumerating keyboard devices...");
                std::thread::sleep(Duration::from_secs(2));
                press_devices = keyboard::find_keyboard_devices();
                release_devices = keyboard::find_keyboard_devices();
                if press_devices.is_empty() {
                    eprintln!("[stt-typer] no keyboard devices found, retrying in 5s...");
                    std::thread::sleep(Duration::from_secs(5));
                }
                continue;
            }
        }

        audio::wait_for_input_device(&device_match, Duration::from_secs(2));

        eprintln!("[stt-typer] recording... (release right CTRL to stop)");
        play_beep();

        // Start recording, stop when key is released or max_duration reached
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_key = stop.clone();

        // Spawn thread to wait for key release
        let mut rel_devs = std::mem::take(&mut release_devices);
        let key_thread = std::thread::spawn(move || {
            let result =
                keyboard::wait_for_right_ctrl_release(&mut rel_devs, Duration::from_secs(86400));
            stop_for_key.store(true, Ordering::Relaxed);
            (rel_devs, result)
        });

        let samples = match audio::record_until_stopped(&device_match, stop, max_duration) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[stt-typer] recording failed: {e}");
                let (devs, key_result) = key_thread.join().unwrap();
                release_devices = devs;
                if let Err(ref ke) = key_result {
                    eprintln!("[stt-typer] key release error: {ke}");
                    press_devices = keyboard::find_keyboard_devices();
                    release_devices = keyboard::find_keyboard_devices();
                }
                continue;
            }
        };

        let (devs, key_result) = key_thread.join().unwrap();
        release_devices = devs;

        // If the release thread hit a device error, re-enumerate
        if let Err(ref e) = key_result {
            eprintln!("[stt-typer] key release error: {e}");
            eprintln!("[stt-typer] re-enumerating keyboard devices...");
            press_devices = keyboard::find_keyboard_devices();
            release_devices = keyboard::find_keyboard_devices();
        }

        if samples.is_empty() {
            eprintln!("[stt-typer] no audio captured, skipping");
            continue;
        }

        let duration_secs = samples.len() as f32 / 16000.0;
        eprintln!("[stt-typer] recorded {duration_secs:.1}s, transcribing...");

        let text = match transcribe::transcribe_with_context(&ctx, &samples, &lang) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[stt-typer] transcription failed: {e}");
                continue;
            }
        };

        if text.is_empty() {
            eprintln!("[stt-typer] (empty transcription)");
            continue;
        }

        eprintln!("[stt-typer] typing: {text}");
        if let Err(e) = type_text(&text) {
            eprintln!("[stt-typer] typing failed: {e}");
        }
    }
}
