# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A push-to-talk voice typing CLI for Linux. Hold right CTRL to speak, release to transcribe via the Gemini API, and type the result into the active window using ydotool.

## Build & Run

```bash
# Build (requires: alsa-lib-devel; a C compiler for cpal)
cargo build --release

# Set GEMINI_API_KEY to a Google AI Studio API key before running
export GEMINI_API_KEY=your-key-here

# Run
target/release/stt-typer
```

There are no tests in this project currently.

## Architecture

Five source files, each with a single responsibility:

- **`src/main.rs`** — CLI entry point using `clap`. Parses args, builds the Gemini client once, then loops: wait for right CTRL press, record audio until release, transcribe, type result via `ydotool`. Also handles ydotool socket detection and plays a beep on recording start.

- **`src/audio.rs`** — Audio capture via `cpal`. `record()` opens the default input device and records for a fixed duration. `record_until_stopped()` records until an `AtomicBool` is set. Both return mono 16kHz f32 samples (what the transcription API expects). Supports F32 and I16 sample formats.

- **`src/keyboard.rs`** — Keyboard input via `evdev`. `find_keyboard_devices()` scans for devices supporting KEY_RIGHTCTRL. `wait_for_right_ctrl()` and `wait_for_right_ctrl_release()` poll for key press/release in non-blocking mode.

- **`src/transcribe.rs`** — Transcription via the Gemini API. `create_context` builds a reusable `GeminiClient` (API key + model + pooled HTTP agent); `transcribe_with_context` encodes the samples to WAV, POSTs them to `generateContent`, and returns the transcript.

- **`src/wav.rs`** — Encodes 16 kHz mono `f32` samples to an in-memory 16-bit PCM WAV for upload.

## Key Dependencies

- `ureq` — blocking HTTP client used to call the Gemini `generateContent` API
- `serde` / `serde_json` — Gemini request/response (de)serialization
- `base64` — encodes the WAV audio for inline upload
- `cpal` — Cross-platform audio input (requires alsa-lib-devel on Linux)
- `evdev` — Linux input event device reading (requires user in `input` group for `/dev/input` access)
- `clap` — CLI argument parsing
