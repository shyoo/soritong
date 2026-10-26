//! S16LE to F32 PCM decoding and chunking for ElevenLabs audio streams.

use soritong_core::{Cancel, PcmChunk, Result, TtsError};
use std::io::Read;

/// Normalization constant for converting 16-bit signed PCM to f32 in [-1.0, 1.0].
const I16_NORM: f32 = 32768.0;

/// Default sample rate for ElevenLabs PCM streaming.
pub const ELEVENLABS_SAMPLE_RATE: u32 = 24_000;

/// Default channels for ElevenLabs PCM streaming (mono).
pub const ELEVENLABS_CHANNELS: u16 = 1;

/// Target samples per emitted chunk (~100 ms at 24 kHz = 2400 samples).
pub const DEFAULT_CHUNK_SAMPLES: usize = 2400;

/// Stream decoder that converts raw little-endian 16-bit PCM (s16le) bytes into
/// 32-bit floating-point samples (f32) in chunks of approximately 100 ms.
#[derive(Debug)]
pub struct PcmDecoder {
    sample_rate: u32,
    channels: u16,
    target_chunk_samples: usize,
    leftover_byte: Option<u8>,
    buffer: Vec<f32>,
}

impl PcmDecoder {
    pub fn new(sample_rate: u32, channels: u16, target_chunk_samples: usize) -> Self {
        Self {
            sample_rate,
            channels,
            target_chunk_samples,
            leftover_byte: None,
            buffer: Vec::with_capacity(target_chunk_samples),
        }
    }

    /// Feed raw bytes into the decoder, converting pairs of bytes into f32 samples.
    /// Emits `PcmChunk`s to `on_chunk` as soon as `target_chunk_samples` are accumulated.
    pub fn feed(
        &mut self,
        mut bytes: &[u8],
        cancel: &Cancel,
        on_chunk: &mut dyn FnMut(PcmChunk),
    ) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }

        // If an odd byte was left over from a previous read, pair it with the first byte.
        if let Some(prev) = self.leftover_byte.take() {
            let sample_i16 = i16::from_le_bytes([prev, bytes[0]]);
            self.buffer.push(sample_i16 as f32 / I16_NORM);
            bytes = &bytes[1..];
        }

        // If the remaining bytes have an odd length, save the last byte for next read.
        if bytes.len() % 2 != 0 {
            self.leftover_byte = Some(bytes[bytes.len() - 1]);
            bytes = &bytes[..bytes.len() - 1];
        }

        // Convert pairs of bytes to f32 samples
        for pair in bytes.chunks_exact(2) {
            let sample_i16 = i16::from_le_bytes([pair[0], pair[1]]);
            self.buffer.push(sample_i16 as f32 / I16_NORM);
        }

        // Emit chunks whenever buffer has accumulated enough samples
        while self.buffer.len() >= self.target_chunk_samples {
            if cancel.is_cancelled() {
                return Err(TtsError::Cancelled);
            }
            let chunk_samples = self.buffer.drain(..self.target_chunk_samples).collect();
            on_chunk(PcmChunk {
                sample_rate: self.sample_rate,
                channels: self.channels,
                samples: chunk_samples,
            });
        }

        Ok(())
    }

    /// Flush any remaining samples in the buffer (e.g. at EOF).
    /// If an odd byte is still dangling at EOF, it is discarded.
    pub fn finish(&mut self, cancel: &Cancel, on_chunk: &mut dyn FnMut(PcmChunk)) -> Result<()> {
        self.leftover_byte = None;
        if !self.buffer.is_empty() {
            if cancel.is_cancelled() {
                return Err(TtsError::Cancelled);
            }
            let chunk_samples = std::mem::take(&mut self.buffer);
            on_chunk(PcmChunk {
                sample_rate: self.sample_rate,
                channels: self.channels,
                samples: chunk_samples,
            });
        }
        Ok(())
    }
}

/// Reads from a blocking stream, decodes s16le bytes to f32, checks `cancel` between reads,
/// and invokes `on_chunk` roughly every 100 ms of audio (2400 samples at 24 kHz).
pub fn decode_pcm_stream<R: Read>(
    mut reader: R,
    cancel: &Cancel,
    on_chunk: &mut dyn FnMut(PcmChunk),
) -> Result<()> {
    let mut decoder = PcmDecoder::new(
        ELEVENLABS_SAMPLE_RATE,
        ELEVENLABS_CHANNELS,
        DEFAULT_CHUNK_SAMPLES,
    );
    let mut read_buf = [0u8; 4096];

    loop {
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let n = match reader.read(&mut read_buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(TtsError::Network(format!(
                    "Failed to read audio stream: {e}"
                )));
            }
        };

        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        decoder.feed(&read_buf[..n], cancel, on_chunk)?;
    }

    if cancel.is_cancelled() {
        return Err(TtsError::Cancelled);
    }

    decoder.finish(cancel, on_chunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm_conversion_and_odd_bytes() {
        let cancel = Cancel::new();
        let mut chunks = Vec::new();
        let mut decoder = PcmDecoder::new(24_000, 1, 4); // small target for testing: 4 samples per chunk

        // 3 samples = 6 bytes. We feed 5 bytes first, leaving 1 odd byte.
        // Sample values: 0, 16384, -16384
        let s0 = 0i16.to_le_bytes();
        let s1 = 16384i16.to_le_bytes();
        let s2 = (-16384i16).to_le_bytes();

        let mut data = Vec::new();
        data.extend_from_slice(&s0);
        data.extend_from_slice(&s1);
        data.extend_from_slice(&s2);
        assert_eq!(data.len(), 6);

        // Feed first 5 bytes (2 full samples + 1 odd byte)
        decoder
            .feed(&data[..5], &cancel, &mut |c| chunks.push(c))
            .unwrap();
        assert_eq!(chunks.len(), 0); // target is 4 samples, only 2 parsed so far

        // Feed remaining 1 byte of data + 2 more samples (4 bytes) + 1 trailing odd byte (1 byte) = 6 bytes
        let s3 = 32767i16.to_le_bytes();
        let s4 = (-32768i16).to_le_bytes();
        let mut data2 = Vec::new();
        data2.push(data[5]); // completes s2
        data2.extend_from_slice(&s3);
        data2.extend_from_slice(&s4);
        data2.push(0x7F); // trailing odd byte

        decoder
            .feed(&data2, &cancel, &mut |c| chunks.push(c))
            .unwrap();

        // Should have emitted 1 chunk of 4 samples (s0, s1, s2, s3)
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples.len(), 4);
        assert_eq!(chunks[0].samples[0], 0.0);
        assert_eq!(chunks[0].samples[1], 0.5);
        assert_eq!(chunks[0].samples[2], -0.5);
        assert!((chunks[0].samples[3] - (32767.0 / 32768.0)).abs() < 1e-6);

        // Finish should emit the remaining sample (s4) and discard trailing odd byte
        decoder.finish(&cancel, &mut |c| chunks.push(c)).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].samples.len(), 1);
        assert_eq!(chunks[1].samples[0], -1.0);
    }
}
