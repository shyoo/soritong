//! Format conversion from engine PCM to one device's rate and channel count.
//!
//! The resampler is stateful, so consecutive chunks of one stream join without seams. It holds back
//! a partial input block plus the filter delay; [`Converter::flush`] drains that tail, after which
//! the next chunk starts a fresh stream.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

/// Input frames per resampler block: ~21 ms at 24 kHz, small enough to keep latency low.
const BLOCK: usize = 512;

/// Up/down-mixes interleaved samples. Mono goes to the first two outputs (left/right); a mono
/// output averages every input; otherwise channels map one to one and extras are dropped or silent.
pub fn remix(input: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || from == 0 {
        return input.to_vec();
    }
    let frames = input.len() / from;
    let mut out = vec![0.0; frames * to];
    for (src, dst) in input.chunks_exact(from).zip(out.chunks_exact_mut(to)) {
        if to == 1 {
            dst[0] = src.iter().sum::<f32>() / from as f32;
        } else if from == 1 {
            dst[0] = src[0];
            dst[1] = src[0];
        } else {
            let n = from.min(to);
            dst[..n].copy_from_slice(&src[..n]);
        }
    }
    out
}

pub struct Converter {
    pub in_rate: u32,
    pub in_channels: usize,
    out_channels: usize,
    /// Channels resampled: the smaller side, so a 2→1 or 1→8 path resamples a single channel.
    work_channels: usize,
    rs: Option<Box<Fft<f32>>>,
    pending: Vec<f32>,
    trim: usize,
    in_frames: u64,
    out_frames: u64,
    outbuf: Vec<f32>,
}

impl Converter {
    pub fn new(in_rate: u32, in_channels: usize, out_rate: u32, out_channels: usize) -> Self {
        let in_channels = in_channels.max(1);
        let work_channels = in_channels.min(out_channels).max(1);
        let rs = (in_rate != out_rate && in_rate > 0)
            .then(|| {
                Fft::<f32>::new(
                    in_rate as usize,
                    out_rate as usize,
                    BLOCK,
                    work_channels,
                    FixedSync::Input,
                )
                .map_err(|e| log::error!("resampler {in_rate}->{out_rate}: {e}"))
                .ok()
            })
            .flatten()
            .map(Box::new);
        let (trim, out_max) = rs
            .as_ref()
            .map_or((0, 0), |r| (r.output_delay(), r.output_frames_max()));
        Self {
            in_rate,
            in_channels,
            out_channels,
            work_channels,
            rs,
            pending: Vec::new(),
            trim,
            in_frames: 0,
            out_frames: 0,
            outbuf: vec![0.0; out_max * work_channels],
        }
    }

    /// True while audio is held inside (partial block or filter delay) and needs [`Self::flush`].
    pub fn holds_audio(&self) -> bool {
        self.in_frames > 0
    }

    /// Converts as much as can be emitted without breaking continuity.
    pub fn feed(&mut self, samples: &[f32]) -> Vec<f32> {
        let work = remix(samples, self.in_channels, self.work_channels);
        let Some(rs) = self.rs.as_mut() else {
            return remix(&work, self.work_channels, self.out_channels);
        };
        self.pending.extend_from_slice(&work);
        self.in_frames += (work.len() / self.work_channels) as u64;
        let mut out = Vec::new();
        let block = BLOCK * self.work_channels;
        let mut consumed = 0;
        while self.pending.len() - consumed >= block {
            let input = &self.pending[consumed..consumed + block];
            Self::run(rs, &mut self.outbuf, input, None, &mut out, &mut self.trim);
            consumed += block;
        }
        self.pending.drain(..consumed);
        self.out_frames += (out.len() / self.work_channels) as u64;
        remix(&out, self.work_channels, self.out_channels)
    }

    /// Emits everything held back and resets, so the next chunk starts a new stream.
    pub fn flush(&mut self) -> Vec<f32> {
        let Some(rs) = self.rs.as_mut() else {
            return Vec::new();
        };
        if self.in_frames == 0 {
            return Vec::new();
        }
        let expected = (self.in_frames as f64 * rs.resample_ratio()).round() as u64;
        let wc = self.work_channels;
        let mut out = Vec::new();
        let mut input = std::mem::take(&mut self.pending);
        let partial = input.len() / wc;
        input.resize(BLOCK * wc, 0.0);
        let mut partial = Some(partial);
        // Bounded: each pass yields about a block; the guard only matters if rubato yields nothing.
        let mut passes = 0;
        while self.out_frames + ((out.len() / wc) as u64) < expected && passes < 64 {
            passes += 1;
            Self::run(
                rs,
                &mut self.outbuf,
                &input,
                partial,
                &mut out,
                &mut self.trim,
            );
            partial = Some(0);
        }
        out.truncate((expected.saturating_sub(self.out_frames)) as usize * wc);
        rs.reset();
        self.trim = rs.output_delay();
        self.in_frames = 0;
        self.out_frames = 0;
        remix(&out, wc, self.out_channels)
    }

    fn run(
        rs: &mut Fft<f32>,
        outbuf: &mut [f32],
        input: &[f32],
        partial: Option<usize>,
        out: &mut Vec<f32>,
        trim: &mut usize,
    ) {
        let wc = rs.nbr_channels();
        let Ok(adapter) = InterleavedSlice::new(input, wc, input.len() / wc) else {
            return;
        };
        let frames_out = outbuf.len() / wc;
        let Ok(mut out_adapter) = InterleavedSlice::new_mut(outbuf, wc, frames_out) else {
            return;
        };
        let indexing = partial.map(|p| Indexing::new().partial_len(p));
        match rs.process_into_buffer(&adapter, &mut out_adapter, indexing.as_ref()) {
            Ok((_, produced)) => {
                let data = &*outbuf;
                let skip = (*trim).min(produced);
                *trim -= skip;
                out.extend_from_slice(&data[skip * wc..produced * wc]);
            }
            Err(e) => log::error!("resample: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|n| (std::f32::consts::TAU * 440.0 * n as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn remixes_channels() {
        assert_eq!(remix(&[1.0, 2.0], 1, 2), vec![1.0, 1.0, 2.0, 2.0]);
        assert_eq!(remix(&[1.0, 3.0], 2, 1), vec![2.0]);
        assert_eq!(remix(&[1.0], 1, 4), vec![1.0, 1.0, 0.0, 0.0]);
        assert_eq!(remix(&[1.0, 2.0, 3.0, 4.0], 4, 2), vec![1.0, 2.0]);
        assert_eq!(remix(&[1.0, 2.0], 2, 2), vec![1.0, 2.0]);
    }

    #[test]
    fn same_rate_passes_through_without_holding_audio() {
        let mut c = Converter::new(48_000, 1, 48_000, 2);
        assert_eq!(c.feed(&[0.5, -0.5]), vec![0.5, 0.5, -0.5, -0.5]);
        assert!(!c.holds_audio());
        assert!(c.flush().is_empty());
    }

    #[test]
    fn resamples_to_the_expected_length_and_joins_chunks_seamlessly() {
        let input = tone(24_000, 0.3);
        let mut whole = Converter::new(24_000, 1, 48_000, 2);
        let mut a = whole.feed(&input);
        a.extend(whole.flush());
        assert_eq!(a.len(), input.len() * 2 * 2);

        // Odd-sized pieces, as a streaming engine sends them, give the same signal.
        let mut split = Converter::new(24_000, 1, 48_000, 2);
        let mut b = Vec::new();
        for piece in input.chunks(777) {
            b.extend(split.feed(piece));
            assert!(split.holds_audio());
        }
        b.extend(split.flush());
        assert!(!split.holds_audio());
        assert_eq!(a.len(), b.len());
        let max_diff = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max);
        assert!(max_diff < 1e-4, "max diff {max_diff}");

        // The tone survives: same peak level, and no leading silence from the filter delay.
        let peak = a.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.05, "peak {peak}");
        assert!(a[2..40].iter().any(|s| s.abs() > 0.05));
    }

    #[test]
    fn handles_44k1_to_48k() {
        let input = tone(44_100, 0.25);
        let mut c = Converter::new(44_100, 1, 48_000, 1);
        let mut out = c.feed(&input);
        out.extend(c.flush());
        assert_eq!(
            out.len(),
            (input.len() as f64 * 48_000.0 / 44_100.0).round() as usize
        );
    }
}
