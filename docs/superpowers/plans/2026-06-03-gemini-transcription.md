# Gemini Transcription Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the local whisper.cpp transcription backend with the Gemini cloud API, keeping the recording/keyboard layers untouched.

**Architecture:** Audio capture still produces 16 kHz mono `f32` samples. A new `src/wav.rs` encodes those samples to an in-memory 16-bit PCM WAV. `src/transcribe.rs` is rewritten to base64-encode the WAV and POST it to the Gemini `generateContent` API via a blocking `ureq` agent, then parse the transcript from the JSON response. `src/main.rs` reads `GEMINI_API_KEY`, drops all Whisper model-path handling, and adds a `--model` string flag defaulting to `gemini-3.5-flash`.

**Tech Stack:** Rust (edition 2024), `ureq` 3.x (blocking HTTP + JSON), `serde`/`serde_json`, `base64`, `cpal`, `evdev`, `clap`. Removes `whisper-rs` (and with it the cmake + clang build requirement).

---

## File Structure

- **Create `src/wav.rs`** — pure WAV encoder, one public function, unit-tested. No external deps.
- **Rewrite `src/transcribe.rs`** — `GeminiClient` struct + `create_context` / `transcribe_with_context`. Owns the HTTP request/response types and parsing.
- **Modify `src/main.rs`** — register `mod wav;`, read `GEMINI_API_KEY`, swap the `--model` arg from a Whisper path to a Gemini model name, drop model-path plumbing.
- **Modify `Cargo.toml`** — remove `whisper-rs`, add `ureq`, `serde`, `serde_json`, `base64`.
- **Modify `CLAUDE.md` / `README.md`** — remove Whisper setup, document `GEMINI_API_KEY` and the new `--model`.

---

## Task 1: WAV encoder module

A pure, deterministic function with a unit test — built first because it's the only piece we can test in isolation, and `transcribe.rs` will depend on it.

**Files:**
- Create: `src/wav.rs`
- Modify: `src/main.rs:1-3` (add `mod wav;`)

- [ ] **Step 1: Write the failing test**

Create `src/wav.rs` with ONLY the test (no implementation yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_wav_header_and_samples() {
        let samples = vec![0.0_f32, 1.0, -1.0, 0.5];
        let wav = encode_wav_16k_mono(&samples);

        // 44-byte header + 2 bytes per sample
        assert_eq!(wav.len(), 44 + samples.len() * 2);

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");

        // sample rate at offset 24 (LE u32)
        let rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(rate, 16_000);

        // bits per sample at offset 34 (LE u16)
        let bits = u16::from_le_bytes([wav[34], wav[35]]);
        assert_eq!(bits, 16);

        // "data" chunk id at offset 36, data length at offset 40
        assert_eq!(&wav[36..40], b"data");
        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_len, (samples.len() * 2) as u32);

        // first sample 0.0 -> 0i16
        let s0 = i16::from_le_bytes([wav[44], wav[45]]);
        assert_eq!(s0, 0);
        // second sample 1.0 -> i16::MAX
        let s1 = i16::from_le_bytes([wav[46], wav[47]]);
        assert_eq!(s1, i16::MAX);
    }
}
```

Then add `mod wav;` to `src/main.rs` immediately after the existing `mod audio;` line (top of file, line 1-3 area), so the new module is compiled:

```rust
mod audio;
mod keyboard;
mod transcribe;
mod wav;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib wav 2>&1 | head -30`
Expected: compile error — `cannot find function 'encode_wav_16k_mono' in this scope`.

- [ ] **Step 3: Write the implementation**

Add this ABOVE the `#[cfg(test)]` block in `src/wav.rs`:

```rust
/// Encode mono 16 kHz `f32` samples (range roughly [-1.0, 1.0]) as a 16-bit PCM
/// WAV file in memory. Returns the complete WAV bytes (44-byte header + data).
pub fn encode_wav_16k_mono(samples: &[f32]) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const BITS_PER_SAMPLE: u16 = 16;
    const CHANNELS: u16 = 1;

    let byte_rate = SAMPLE_RATE * CHANNELS as u32 * (BITS_PER_SAMPLE as u32 / 8);
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    let data_len = (samples.len() * 2) as u32;

    let mut out = Vec::with_capacity(44 + data_len as usize);

    // RIFF header
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());

    // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let val = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&val.to_le_bytes());
    }

    out
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib wav 2>&1 | tail -15`
Expected: `test wav::tests::writes_valid_wav_header_and_samples ... ok`

- [ ] **Step 5: Commit**

```bash
git add src/wav.rs src/main.rs
git commit -m "feat: add in-memory WAV encoder for 16kHz mono audio"
```

---

## Task 2: Swap the transcription backend to Gemini

This is one atomic commit because the three changes are coupled: removing `whisper-rs` breaks `transcribe.rs`, and rewriting `transcribe.rs`'s function signatures breaks `main.rs`. Doing them together keeps the build green. There is no unit test here (it's network I/O); the WAV encoder carries the unit test and end-to-end is verified manually in Task 4.

**Files:**
- Modify: `Cargo.toml`
- Modify (full rewrite): `src/transcribe.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Update dependencies in `Cargo.toml`**

Replace the entire `[dependencies]` section with:

```toml
[dependencies]
cpal = "0.15"
anyhow = "1"
clap = { version = "4", features = ["derive", "env"] }
evdev = "0.13"
ureq = { version = "3", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"
```

(The `whisper-rs` line is removed; `ureq`/`serde`/`serde_json`/`base64` are added.)

- [ ] **Step 2: Rewrite `src/transcribe.rs`**

Replace the ENTIRE contents of `src/transcribe.rs` with:

```rust
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use ureq::Agent;

use crate::wav::encode_wav_16k_mono;

const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// A reusable client for the Gemini `generateContent` API. Holds the API key,
/// the chosen model, and a `ureq` agent so connections are pooled across
/// utterances.
pub struct GeminiClient {
    api_key: String,
    model: String,
    agent: Agent,
}

/// Build a Gemini client. Replaces the old Whisper model loader — there is no
/// model file to load, just an HTTP agent to configure.
pub fn create_context(api_key: String, model: String) -> Result<GeminiClient> {
    let config = Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    let agent: Agent = config.into();
    Ok(GeminiClient { api_key, model, agent })
}

// ---- request types ----

#[derive(Serialize)]
struct GenerateContentRequest {
    contents: Vec<Content>,
}

#[derive(Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part {
    Text {
        text: String,
    },
    Inline {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
}

#[derive(Serialize)]
struct InlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

// ---- response types ----

#[derive(Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<RespContent>,
}

#[derive(Deserialize)]
struct RespContent {
    #[serde(default)]
    parts: Vec<RespPart>,
}

#[derive(Deserialize)]
struct RespPart {
    #[serde(default)]
    text: Option<String>,
}

/// Transcribe audio via the Gemini API. Same call shape as the old Whisper
/// function: takes the client, 16 kHz mono `f32` samples, and a language hint.
pub fn transcribe_with_context(
    client: &GeminiClient,
    samples: &[f32],
    language: &str,
) -> Result<String> {
    let wav = encode_wav_16k_mono(samples);
    let b64 = STANDARD.encode(&wav);

    let prompt = format!(
        "Transcribe this audio verbatim. Return only the transcription text, \
         with no commentary, labels, or formatting. The spoken language is {language}."
    );

    let request = GenerateContentRequest {
        contents: vec![Content {
            parts: vec![
                Part::Text { text: prompt },
                Part::Inline {
                    inline_data: InlineData {
                        mime_type: "audio/wav".to_string(),
                        data: b64,
                    },
                },
            ],
        }],
    };

    let url = format!("{API_BASE}/{}:generateContent", client.model);

    let mut response = match client
        .agent
        .post(&url)
        .header("x-goog-api-key", &client.api_key)
        .send_json(&request)
    {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) if code == 401 || code == 403 => {
            bail!("Gemini API rejected the request (HTTP {code}) — check GEMINI_API_KEY")
        }
        Err(ureq::Error::StatusCode(code)) => {
            bail!("Gemini API returned HTTP {code}")
        }
        Err(e) => bail!("Gemini API request failed: {e}"),
    };

    let parsed: GenerateContentResponse = response
        .body_mut()
        .read_json()
        .context("failed to parse Gemini response")?;

    let text = parsed
        .candidates
        .into_iter()
        .find_map(|c| c.content)
        .map(|content| {
            content
                .parts
                .into_iter()
                .filter_map(|p| p.text)
                .collect::<String>()
        })
        .unwrap_or_default();

    Ok(text.trim().to_string())
}
```

- [ ] **Step 3: Update `src/main.rs` — imports and constants**

At the top of `src/main.rs`:

- Remove the line `use std::path::PathBuf;`
- Remove the constant `const DEFAULT_MODEL_PATH: &str = ".local/share/stt-mcp/ggml-base.bin";`
- Remove the entire `dirs_path()` function (it existed only to build the default model path).

- [ ] **Step 4: Update `src/main.rs` — the `Args` struct**

Replace the `--language` and `--model` argument fields in `struct Args` with:

```rust
    /// Language hint passed to Gemini (default: "en")
    #[arg(short, long, default_value = "en")]
    language: String,

    /// Gemini model to use for transcription
    #[arg(short = 'M', long, default_value = "gemini-3.5-flash")]
    model: String,
```

(The `model` field changes from `Option<PathBuf>` with `env = "WHISPER_MODEL_PATH"` to a plain `String` with a default. The doc comment on `--language` no longer mentions Whisper.)

- [ ] **Step 5: Update `src/main.rs` — `main()` startup**

In `main()`, replace this block:

```rust
    let model_path = args
        .model
        .unwrap_or_else(|| dirs_path().join(DEFAULT_MODEL_PATH));

    // Preflight checks
    detect_ydotool_socket();

    eprintln!("[stt-typer] loading whisper model from {}", model_path.display());
    let ctx = transcribe::create_context(&model_path)
        .context("failed to load whisper model")?;
    eprintln!("[stt-typer] model loaded");
```

with:

```rust
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
```

(The `transcribe::transcribe_with_context(&ctx, &samples, &lang)` call site later in the loop is unchanged — `ctx` is now a `GeminiClient` but the call shape is identical.)

- [ ] **Step 6: Build and verify whisper is gone from the dependency graph**

Run: `cargo build --release 2>&1 | tail -20`
Expected: `Finished \`release\` profile` with no errors.

Run: `cargo tree 2>/dev/null | grep -i whisper; echo "exit=$?"`
Expected: no whisper lines printed (grep finds nothing) — confirming `whisper-rs` and the cmake/clang toolchain are no longer pulled in.

Run: `cargo test --lib 2>&1 | tail -10`
Expected: the `wav` test still passes.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/transcribe.rs src/main.rs
git commit -m "feat: replace Whisper with the Gemini API for transcription"
```

---

## Task 3: Update documentation

**Files:**
- Modify: `CLAUDE.md`
- Modify: `README.md`

- [ ] **Step 1: Update `CLAUDE.md`**

Make these edits:

1. In **Build & Run**, change the build-deps comment line from:
   `# Build (requires: alsa-lib-devel clang-devel cmake gcc-c++)`
   to:
   `# Build (requires: alsa-lib-devel; a C compiler for cpal)`

2. Replace the two lines about the Whisper model:
   ```
   # The Whisper model must exist at ~/.local/share/stt-mcp/ggml-base.bin
   # or set WHISPER_MODEL_PATH to a custom location
   ```
   with:
   ```
   # Set GEMINI_API_KEY to a Google AI Studio API key before running
   export GEMINI_API_KEY=your-key-here
   ```

3. In **Architecture**, replace the `src/main.rs` bullet's tail "loads the Whisper model once, then loops" wording and the `src/transcribe.rs` bullet:
   - `src/main.rs` bullet: change "loads the Whisper model once" to "builds the Gemini client once".
   - `src/transcribe.rs` bullet: replace its text with:
     `**\`src/transcribe.rs\`** — Transcription via the Gemini API. \`create_context\` builds a reusable \`GeminiClient\` (API key + model + pooled HTTP agent); \`transcribe_with_context\` encodes the samples to WAV, POSTs them to \`generateContent\`, and returns the transcript.`
   - Add a new bullet: `**\`src/wav.rs\`** — Encodes 16 kHz mono \`f32\` samples to an in-memory 16-bit PCM WAV for upload.`

4. In **Key Dependencies**, replace the `whisper-rs` bullet with:
   ```
   - `ureq` — blocking HTTP client used to call the Gemini `generateContent` API
   - `serde` / `serde_json` — Gemini request/response (de)serialization
   - `base64` — encodes the WAV audio for inline upload
   ```

- [ ] **Step 2: Update `README.md`**

1. In the intro paragraph, replace "Transcription is powered by [whisper.cpp](...) running locally." with:
   "Transcription is powered by Google's [Gemini API](https://ai.google.dev/gemini-api/docs)."

2. In **Prerequisites → Build dependencies**, change:
   `sudo dnf install alsa-lib-devel clang-devel cmake gcc-c++`
   to:
   `sudo dnf install alsa-lib-devel gcc`

3. Replace the entire **## Download the Whisper model** section with:

   ```markdown
   ## Set your Gemini API key

   stt-typer transcribes audio with the Gemini API. Create an API key in
   [Google AI Studio](https://aistudio.google.com/apikey) and export it:

   ```bash
   export GEMINI_API_KEY=your-key-here
   ```

   Each utterance's audio is uploaded to the Gemini API for transcription, so a
   network connection is required.
   ```

4. In **### Options**, replace the option list with:

   ```
   -m, --max-duration <SECS>   Maximum seconds to record (default: 30)
   -l, --language <LANG>       Language hint passed to Gemini (default: "en")
   -M, --model <NAME>          Gemini model to use (default: "gemini-3.5-flash")
   -d, --device <SUBSTR>       Input device name substring to record from
   ```

5. Replace the **### Example** block with:

   ```markdown
   ### Example

   ```bash
   # Use a preview model and set Spanish as the language
   target/release/stt-typer --model gemini-3-flash-preview --language es
   ```
   ```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md README.md
git commit -m "docs: document Gemini API setup, drop Whisper instructions"
```

---

## Task 4: Manual end-to-end verification

No automated coverage exists for the live API path, so verify it by hand.

**Files:** none (runtime check)

- [ ] **Step 1: Confirm the binary runs and validates the key**

Run without a key to confirm the fail-fast message:

```bash
env -u GEMINI_API_KEY target/release/stt-typer 2>&1 | head -3
```
Expected: an error mentioning `GEMINI_API_KEY is not set`.

- [ ] **Step 2: Run with a real key and transcribe**

```bash
export GEMINI_API_KEY=your-key-here   # if not already exported
target/release/stt-typer
```
Then, with a text field focused: hold **right CTRL**, speak a sentence, release.

Expected:
- A beep on press.
- Log lines: `recorded N.Ns, transcribing...` then `typing: <your words>`.
- The spoken text is typed into the active window.

- [ ] **Step 3: Confirm graceful handling of a bad key**

```bash
GEMINI_API_KEY=invalid-key target/release/stt-typer
```
Hold right CTRL, speak, release.
Expected: the program logs `transcription failed: Gemini API rejected the request (HTTP 400/401/403) …` and keeps running (does not crash).

---

## Spec coverage check

- Replace Whisper entirely → Tasks 1–2 (whisper-rs removed, verified via `cargo tree`).
- `GEMINI_API_KEY` auth, fail-fast → Task 2 Step 5, verified Task 4 Step 1.
- Default model `gemini-3.5-flash`, `--model` override → Task 2 Step 4.
- WAV encode → inline base64 → POST → parse → Tasks 1–2.
- `ureq` timeout so a hung network can't freeze the loop → Task 2 Step 2 (`timeout_global`).
- Non-fatal per-utterance error handling → existing `main.rs` loop + Task 2 error mapping, verified Task 4 Step 3.
- Drop cmake/clang build requirement → Task 2 Step 1 + Step 6 verification; docs in Task 3.
- Dependency changes → Task 2 Step 1.
- Docs (CLAUDE.md, README) → Task 3.
- WAV encoder unit test → Task 1.
