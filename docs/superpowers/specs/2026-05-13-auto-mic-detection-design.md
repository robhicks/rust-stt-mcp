# Automatic Microphone Detection — Design

## Problem

`stt-typer` works correctly when launched after a microphone is already connected, but if launched with no input device present and a microphone is plugged in afterwards, pressing right-CTRL produces no recording and no visible feedback. The user must restart the CLI to recover.

The root cause is that `audio::start_recording()` constructs `cpal::default_host()` and asks for `default_input_device()` on every press, but the program enters its main key-wait loop unconditionally at startup and never communicates that no input device is available. When the press path then fails, the error is easy to miss and the user has no signal that the program is "waiting" for hardware.

## Goal

The program should:

1. Refuse to enter the ready state at startup until a usable default input device exists, surfacing a clear "waiting for microphone..." message in the meantime.
2. Re-check device presence at the start of every recording so a mid-session unplug recovers automatically once the device is plugged back in.
3. Not add new runtime dependencies or platform-specific code paths.

## Non-Goals

- Subscribing to kernel/udev hot-plug events.
- Choosing a non-default input device when multiple are present.
- Detecting silent/broken devices that enumerate successfully but produce no audio.
- Adding a test suite (the project has none today; manual verification is sufficient).

## Approach

Polling-based detection inside `src/audio.rs`. `cpal::default_host()` is constructed fresh on each probe so any host-level caching of "no device" state is bypassed. Polling interval is fixed at 2 seconds — cheap enumeration, invisible to a human reaction time, and matches the existing keyboard-reconnection cadence in `src/main.rs:209-218`.

A udev-based approach was rejected because PipeWire / PulseAudio routing changes are user-space events udev does not surface, so it would not be reliably better than polling for this CLI.

## Components

### `src/audio.rs`

Two new public functions:

```rust
pub fn has_input_device() -> bool {
    cpal::default_host().default_input_device().is_some()
}

pub fn wait_for_input_device(poll_interval: Duration);
```

`wait_for_input_device` semantics:

- If `has_input_device()` is true on first check, return immediately and print nothing.
- Otherwise, print `[stt-typer] waiting for microphone...` once, then loop sleeping `poll_interval` between checks.
- When the device finally appears, print `[stt-typer] microphone detected` and return.

The function never errors and never returns until a device is present — the only way to abort the wait is Ctrl-C, which is standard for a CLI.

### `src/main.rs`

Two call sites added:

1. After the ydotool/whisper preflight and after the keyboard device enumeration, but before the `[stt-typer] ready` line:
   ```rust
   audio::wait_for_input_device(Duration::from_secs(2));
   ```
2. Inside the main loop, after a right-CTRL press is detected and before `play_beep()`:
   ```rust
   audio::wait_for_input_device(Duration::from_secs(2));
   ```

The existing error path that prints `[stt-typer] recording failed: ...` and `continue`s is retained unchanged. If `start_recording()` fails despite enumeration showing a device (e.g., stream build error), the loop falls through to the next press, and the call-site re-check on the next press will block until the device is healthy again.

## Data Flow

```
startup
  ├─ load whisper model
  ├─ detect ydotool socket
  ├─ enumerate keyboard devices
  ├─ wait_for_input_device(2s)          ← new
  └─ print "ready", enter loop

loop
  ├─ wait_for_right_ctrl
  ├─ wait_for_input_device(2s)          ← new (no-op if mic present)
  ├─ play_beep
  ├─ record_until_stopped
  ├─ transcribe
  └─ type_text
```

## Error Handling

- `cpal::default_host().default_input_device()` returns `Option`, not `Result`. The `None` case is the polling trigger; there is no enumeration-error variant to handle.
- `start_recording()` continues to return `Result` and is handled by the existing `[stt-typer] recording failed: ...` path. The wait-loop on the *next* press is the recovery mechanism — no per-press retry inside `record_until_stopped`.
- Mid-recording unplug is not specifically detected; the existing path that prints `(empty transcription)` or yields garbage samples remains. This is acceptable for v1.

## Testing

Manual verification steps:

1. Start `stt-typer` with no USB mic connected. Expect: `[stt-typer] waiting for microphone...` and no `ready` line yet.
2. Plug in a USB mic. Within ~2s expect: `[stt-typer] microphone detected` followed by the normal `[stt-typer] ready` line.
3. Press right-CTRL, speak, release. Recording, transcription, and typing all work normally.
4. With the program running and mic connected, unplug the mic. Press right-CTRL. Expect: `[stt-typer] waiting for microphone...` until the mic is reconnected, then beep and recording proceed.

## Out of Scope / Future Work

- udev-driven instant detection if 2s latency becomes a complaint.
- Selecting a specific device by name via a `--device` flag.
- Streaming-time disconnect detection.
