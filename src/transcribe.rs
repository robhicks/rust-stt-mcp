use anyhow::{Context, Result};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Create a WhisperContext from a model file, reusable across multiple transcriptions.
pub fn create_context(model_path: &Path) -> Result<WhisperContext> {
    WhisperContext::new_with_params(
        model_path.to_str().unwrap_or_default(),
        WhisperContextParameters::default(),
    )
    .context("failed to load whisper model")
}

/// Transcribe audio using an existing WhisperContext.
pub fn transcribe_with_context(ctx: &WhisperContext, audio: &[f32], language: &str) -> Result<String> {
    let mut state = ctx.create_state().context("failed to create whisper state")?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(language));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, audio)
        .context("whisper transcription failed")?;

    let n_segments = state.full_n_segments();

    let mut text = String::new();
    for i in 0..n_segments {
        let segment = state
            .get_segment(i)
            .context("failed to get segment")?;
        let segment_text = segment
            .to_str()
            .map_err(|e| anyhow::anyhow!("failed to get segment text: {e}"))?;
        text.push_str(segment_text);
    }

    Ok(text.trim().to_string())
}

/// Convenience wrapper: loads model and transcribes in one call.
pub fn transcribe(model_path: &Path, audio: &[f32], language: &str) -> Result<String> {
    let ctx = create_context(model_path)?;
    transcribe_with_context(&ctx, audio, language)
}
