# Design: Replace Whisper with the Gemini API for transcription

**Date:** 2026-06-03
**Status:** Approved, pending implementation plan

## Summary

Replace the local whisper.cpp transcription backend with the Gemini cloud API.
The recording and keyboard layers are unchanged; the change is contained to
`src/transcribe.rs` (rewritten) and a small number of lines in `src/main.rs`,
plus dependency and documentation updates.

The motivation is **transcription accuracy** — Gemini's quality is a clear step
up from the local `whisper-base` model. A secondary benefit is dropping the C++
build toolchain (cmake + clang) and the ~150 MB model download.

## Decisions

- **Replace Whisper entirely.** No `--backend` flag, no offline fallback. The
  `whisper-rs` dependency and its toolchain are removed.
- **Auth via `GEMINI_API_KEY`** environment variable. Fail fast at startup with
  a clear message if it is unset.
- **Default model: `gemini-3.5-flash`** (the current stable Flash line),
  overridable with `--model`. Flash is chosen over Pro because push-to-talk UX
  is latency-sensitive and Flash accuracy already exceeds local whisper-base.
- **Trade-offs accepted:** every utterance requires network + a valid API key
  (no offline use), each utterance's audio is uploaded to Google, and per-call
  latency is network-bound rather than CPU-bound.

## Architecture

### Unchanged

- **`src/audio.rs`** — already produces 16 kHz mono `f32` samples, exactly what
  we need to encode and send. No changes.
- **`src/keyboard.rs`** — no changes.

### `src/transcribe.rs` (rewritten)

The module keeps the same two-function shape so `main.rs` call sites barely
change, but the implementation moves from "run a local model" to
"encode audio → POST → parse response".

- **`create_context(api_key, model) -> Result<GeminiClient>`**
  Builds a lightweight client struct instead of loading a model file:

  ```
  struct GeminiClient {
      api_key: String,
      model: String,
      agent: ureq::Agent,  // reused across utterances for connection pooling
  }
  ```

- **`transcribe_with_context(&client, samples: &[f32], language: &str) -> Result<String>`**
  Same signature as today. Steps:
  1. **Encode** the `f32` samples to an in-memory 16-bit PCM WAV (44-byte
     header + samples). No extra dependency. A 30 s clip is ~1 MB, well under
     Gemini's 20 MB inline-data limit, so inline base64 is used (no Files API).
  2. **Build request** to
     `https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent`
     with the API key. Body (typed via serde):
     - a text part: a fixed instruction —
       *"Transcribe this audio verbatim. Return only the transcription text,
       with no commentary, labels, or formatting."* — combined with the
       language hint (e.g. "The spoken language is en.").
     - an inline-data part: `{ mime_type: "audio/wav", data: <base64 WAV> }`.
  3. **POST** synchronously via `ureq` (fits the existing synchronous thread
     model — no async runtime needed).
  4. **Parse** `candidates[0].content.parts[0].text`, trim, and return.

  Errors (network failure, non-200 status, auth rejection, empty/blocked
  response) return an `anyhow::Error` with a clear message. They do **not**
  crash the program — see error handling below.

A small private helper encodes the WAV; it lives in `transcribe.rs`.

### `src/main.rs` (minimal changes)

- **Remove:** `DEFAULT_MODEL_PATH`, the `WHISPER_MODEL_PATH` env binding, the
  `--model <path>` argument, the model-path resolution, and the
  `create_context(&model_path)` model-loading preflight log.
- **Add:**
  - Read `GEMINI_API_KEY` near the top of `main`; if unset, `bail!` with an
    install/setup hint.
  - A `--model` argument (string) defaulting to `gemini-3.5-flash`.
  - Build the `GeminiClient` via `transcribe::create_context(api_key, model)`.
- **Unchanged:** the record → transcribe → type loop and its error handling.
  The transcribe call site keeps the same shape; a failed Gemini call falls
  through the existing `match … Err(e) => { eprintln!(...); continue; }` branch,
  so a dropped request skips that one utterance and the program keeps running.

### Data flow

```
right CTRL held
  → audio.rs records → 16 kHz mono f32 samples
  → transcribe.rs: encode WAV → base64 → POST to Gemini generateContent
  → parse transcript text
  → main.rs: type_text() via ydotool
```

## Error handling

| Condition                         | Behavior                                              |
|-----------------------------------|-------------------------------------------------------|
| `GEMINI_API_KEY` unset            | `bail!` at startup with a setup hint (fatal).         |
| Network error / timeout           | Log `[stt-typer] transcription failed: …`, `continue`. |
| HTTP 401/403 (bad key)            | Log a clear auth message, `continue` (non-fatal).      |
| HTTP 4xx/5xx other                | Log status + body snippet, `continue`.                 |
| Empty / safety-blocked response   | Log `(empty transcription)`, `continue`.               |

No automatic retries in this version (a held-key re-press is a natural retry).
A request timeout is set on the `ureq` agent so a hung network does not block
the loop indefinitely.

## Dependencies (`Cargo.toml`)

- **Remove:** `whisper-rs` — this also removes the **cmake + clang** build
  requirement. (`cpal` still needs `alsa-lib-devel` on Linux.)
- **Add:**
  - `ureq` — blocking HTTP client, no async runtime.
  - `serde` (derive) + `serde_json` — typed request/response.
  - `base64` — inline audio encoding.

## Documentation updates

- **`CLAUDE.md`** — update the project overview, Build & Run (remove the model
  download / `WHISPER_MODEL_PATH`, add `GEMINI_API_KEY`; note cmake/clang no
  longer required), the `transcribe.rs` description, and the Key Dependencies
  list.
- **`README`** — same substance: remove Whisper model setup, document
  `GEMINI_API_KEY` and `--model`, update build prerequisites.

## Testing

The project currently has no tests. In scope for verification:

- `cargo build --release` succeeds without cmake/clang present.
- A unit test for the WAV encoder (header fields + sample count) is cheap and
  worth adding since it is pure and deterministic.
- Manual end-to-end check: set `GEMINI_API_KEY`, run, hold right CTRL, speak,
  confirm transcribed text is typed into the active window.

## Out of scope

- Offline / fallback transcription.
- Streaming or partial transcription.
- LLM post-processing beyond plain verbatim transcription.
- Provider abstraction for non-Gemini backends.
