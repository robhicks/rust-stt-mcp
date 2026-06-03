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
