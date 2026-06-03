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
