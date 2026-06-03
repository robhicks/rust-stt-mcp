# stt-typer

A push-to-talk voice typing CLI for Linux. Hold **right CTRL** to speak, release to transcribe and type the result into the active window using [ydotool](https://github.com/ReimuNotMoe/ydotool). Transcription is powered by Google's [Gemini API](https://ai.google.dev/gemini-api/docs).

## Prerequisites

Fedora 43 (or similar) with a working microphone. Install the build and runtime dependencies:

```bash
# Build dependencies
sudo dnf install alsa-lib-devel gcc

# Runtime dependency — virtual keyboard for typing output
sudo dnf install ydotool
sudo systemctl enable --now ydotool
```

You need access to `/dev/input/event*` devices for push-to-talk. Add yourself to the `input` group:

```bash
sudo usermod -aG input $USER
# Log out and back in for the group change to take effect
```

You also need a Rust toolchain. If you don't have one:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Set your Gemini API key

stt-typer transcribes audio with the Gemini API. Create an API key in
[Google AI Studio](https://aistudio.google.com/apikey) and export it:

```bash
export GEMINI_API_KEY=your-key-here
```

Each utterance's audio is uploaded to the Gemini API for transcription, so a
network connection is required.

## Build

```bash
cargo build --release
```

The binary is written to `target/release/stt-typer`.

## Usage

```bash
target/release/stt-typer
```

Hold **right CTRL** to speak. A beep signals that recording has started. Release the key to stop recording — the audio is transcribed and typed into the active window.

### Options

```
-m, --max-duration <SECS>   Maximum seconds to record (default: 30)
-l, --language <LANG>       Language hint passed to Gemini (default: "en")
-M, --model <NAME>          Gemini model to use (default: "gemini-3.5-flash")
-d, --device <SUBSTR>       Input device name substring to record from
```

### Example

```bash
# Use a preview model and set Spanish as the language
target/release/stt-typer --model gemini-3-flash-preview --language es
```

## Running as a systemd user service

stt-typer runs as *you* — it needs your input devices, audio device, and the
ydotool socket — so deploy it as a **user** service, not a system one.

Put your key in an env file readable only by you:

```bash
mkdir -p ~/.config/stt-typer
install -m 600 /dev/null ~/.config/stt-typer/env
printf 'GEMINI_API_KEY=your-key-here\n' >> ~/.config/stt-typer/env
```

Create `~/.config/systemd/user/stt-typer.service`:

```ini
[Unit]
Description=Push-to-talk voice typing
After=graphical-session.target

[Service]
EnvironmentFile=%h/.config/stt-typer/env
ExecStart=%h/.local/bin/stt-typer
Restart=on-failure

[Install]
WantedBy=default.target
```

Enable and follow it:

```bash
systemctl --user daemon-reload
systemctl --user enable --now stt-typer.service
journalctl --user -u stt-typer -f
```

Adjust `ExecStart` to wherever you installed the binary. The key stays in the
0600 env file, never in the unit itself (which `systemctl show` would expose).
