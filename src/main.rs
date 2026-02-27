mod audio;
mod transcribe;

use anyhow::Result;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEFAULT_MODEL_PATH: &str = ".local/share/stt-mcp/ggml-base.bin";

#[derive(Debug, Deserialize, JsonSchema)]
struct RecordRequest {
    /// How many seconds to record (default: 5)
    duration_secs: Option<u32>,
    /// Language hint for Whisper, e.g. "en", "es", "fr" (default: "en")
    language: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListenRequest {
    /// How many seconds to record after the trigger phrase is detected (default: 5)
    duration_secs: Option<u32>,
    /// Maximum seconds to listen for the trigger phrase before timing out (default: 60)
    timeout_secs: Option<u32>,
    /// Language hint for Whisper, e.g. "en", "es", "fr" (default: "en")
    language: Option<String>,
}

#[derive(Debug, Clone)]
struct SttServer {
    tool_router: ToolRouter<Self>,
    model_path: PathBuf,
}

impl SttServer {
    fn new() -> Self {
        let model_path = std::env::var("WHISPER_MODEL_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs_path().join(DEFAULT_MODEL_PATH));

        Self {
            tool_router: Self::tool_router(),
            model_path,
        }
    }
}

#[tool_router]
impl SttServer {
    #[tool(description = "Record audio from the microphone and transcribe it to text using Whisper. Returns the transcribed text.")]
    async fn record_and_transcribe(
        &self,
        Parameters(req): Parameters<RecordRequest>,
    ) -> String {
        let duration = Duration::from_secs(req.duration_secs.unwrap_or(5) as u64);
        let lang = req.language.unwrap_or_else(|| "en".to_string());
        let model_path = self.model_path.clone();

        let result = tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            let samples =
                audio::record(duration).map_err(|e| format!("recording failed: {e}"))?;

            if samples.is_empty() {
                return Err("no audio samples captured".to_string());
            }

            transcribe::transcribe(&model_path, &samples, &lang)
                .map_err(|e| format!("transcription failed: {e}"))
        })
        .await;

        match result {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => format!("Error: {e}"),
            Err(e) => format!("Error: task failed: {e}"),
        }
    }

    #[tool(description = "Listen for the wake phrase \"Hey Claude Code\", then record and transcribe the following speech. Returns the transcribed text spoken after the trigger.")]
    async fn listen_for_trigger(
        &self,
        Parameters(req): Parameters<ListenRequest>,
    ) -> String {
        let record_duration = Duration::from_secs(req.duration_secs.unwrap_or(5) as u64);
        let timeout = Duration::from_secs(req.timeout_secs.unwrap_or(60) as u64);
        let lang = req.language.unwrap_or_else(|| "en".to_string());
        let model_path = self.model_path.clone();

        let result = tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            let ctx = transcribe::create_context(&model_path)
                .map_err(|e| format!("failed to load model: {e}"))?;

            let start = Instant::now();
            let chunk_duration = Duration::from_secs(2);

            // Detection loop: record short chunks and check for trigger phrase
            loop {
                if start.elapsed() > timeout {
                    return Err("Timed out waiting for \"Hey Claude Code\" trigger phrase.".to_string());
                }

                let samples = audio::record(chunk_duration)
                    .map_err(|e| format!("recording failed: {e}"))?;

                if samples.is_empty() {
                    continue;
                }

                let text = transcribe::transcribe_with_context(&ctx, &samples, &lang)
                    .map_err(|e| format!("transcription failed: {e}"))?;

                let normalized: String = text
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                    .collect();

                if normalized.contains("hey claude code") {
                    break;
                }
            }

            // Trigger detected — record the actual message
            let samples = audio::record(record_duration)
                .map_err(|e| format!("recording failed: {e}"))?;

            if samples.is_empty() {
                return Err("no audio samples captured after trigger".to_string());
            }

            transcribe::transcribe_with_context(&ctx, &samples, &lang)
                .map_err(|e| format!("transcription failed: {e}"))
        })
        .await;

        match result {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => format!("Error: {e}"),
            Err(e) => format!("Error: task failed: {e}"),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SttServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Speech-to-text server. Use record_and_transcribe to capture audio from the microphone and get transcribed text. Use listen_for_trigger to wait for the wake phrase \"Hey Claude Code\" and then record and transcribe the following speech."
                    .to_string(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

fn dirs_path() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    SttServer::new()
        .serve(stdio())
        .await?
        .waiting()
        .await?;

    Ok(())
}
