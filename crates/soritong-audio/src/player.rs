//! Playback to the mic output and an optional monitor output.
//!
//! Three kinds of thread are involved:
//!
//! - the caller: [`Player::push`] only hands the chunk to a channel, so it never blocks on audio;
//! - one converter worker: resamples and remixes each chunk for each output and appends it to that
//!   output's queue (one queue per device, in the device's own format);
//! - one device thread per configuration: owns the cpal streams (which are not `Send` on every
//!   platform), so [`Player`] is `Send + Sync`. The streams' callbacks drain the queues.
//!
//! [`Player::stop`] bumps a generation counter and then truncates each queue to a 5 ms fade-out.
//! Every append checks the generation under the queue lock, so audio converted before a stop can
//! never be queued after it.

use crate::convert::Converter;
use crate::{AudioError, PlayerConfig, detect::assign_ids, lock};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use soritong_core::PcmChunk;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const FADE_SECS: f32 = 0.005;
/// A held-back resampler tail is flushed once a queue holds less than this, i.e. just before the
/// device would run dry. While chunks keep arriving the stream stays continuous.
const FLUSH_BELOW_SECS: f32 = 0.05;

struct Output {
    label: String,
    rate: u32,
    channels: usize,
    queue: Mutex<VecDeque<f32>>,
    volume: AtomicU32,
    dead: AtomicBool,
    /// Nanoseconds since `Shared::epoch` until which the device is still playing what it was given.
    busy_until: AtomicU64,
}

impl Output {
    fn buffered_secs(&self) -> f32 {
        lock(&self.queue).len() as f32 / (self.rate as f32 * self.channels as f32)
    }

    fn append(&self, generation: &AtomicU64, expected: u64, samples: &[f32]) {
        if samples.is_empty() || self.dead.load(Ordering::SeqCst) {
            return;
        }
        let mut q = lock(&self.queue);
        if generation.load(Ordering::SeqCst) == expected {
            q.extend(samples);
        }
    }

    fn fade_out(&self) {
        let mut q = lock(&self.queue);
        let keep = ((self.rate as f32 * FADE_SECS) as usize * self.channels).min(q.len());
        q.truncate(keep - keep % self.channels);
        let frames = q.len() / self.channels;
        for (i, s) in q.iter_mut().enumerate() {
            *s *= 1.0 - (i / self.channels) as f32 / frames.max(1) as f32;
        }
    }

    fn set_volume(&self, v: f32) {
        self.volume.store(v.max(0.0).to_bits(), Ordering::Relaxed);
    }
}

struct Shared {
    outputs: RwLock<Vec<Arc<Output>>>,
    generation: AtomicU64,
    /// Chunks pushed but not yet queued.
    in_flight: AtomicUsize,
    /// The worker holds resampler tails that are not queued yet.
    holding: AtomicBool,
    last_error: Mutex<Option<String>>,
    epoch: Instant,
}

impl Shared {
    fn now_nanos(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    fn report(&self, msg: String) {
        log::error!("{msg}");
        *lock(&self.last_error) = Some(msg);
    }
}

struct Job {
    generation: u64,
    chunk: PcmChunk,
}

struct DeviceThread {
    shutdown: mpsc::Sender<()>,
    handle: JoinHandle<()>,
}

impl DeviceThread {
    fn close(self) {
        drop(self.shutdown);
        let _ = self.handle.join();
    }
}

/// Plays PCM to the mic output and, optionally, the monitor output, in sync.
pub struct Player {
    shared: Arc<Shared>,
    tx: Option<mpsc::Sender<Job>>,
    devices: Mutex<Option<DeviceThread>>,
    config: Mutex<PlayerConfig>,
    /// `Some` for [`Player::null`]: when the discarded audio would have finished playing.
    null_until: Option<Mutex<Instant>>,
}

impl Player {
    /// Opens the mic output (error if missing) and the monitor output (if missing it is skipped,
    /// and [`Player::last_error`] says so, so unplugged headphones never block speech).
    pub fn new(config: PlayerConfig) -> Result<Self, AudioError> {
        let shared = Arc::new(Shared {
            outputs: RwLock::new(Vec::new()),
            generation: AtomicU64::new(0),
            in_flight: AtomicUsize::new(0),
            holding: AtomicBool::new(false),
            last_error: Mutex::new(None),
            epoch: Instant::now(),
        });
        let (devices, outputs) = open(&config, &shared)?;
        *shared.outputs.write().unwrap_or_else(|e| e.into_inner()) = outputs;
        let (tx, rx) = mpsc::channel();
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("soritong-convert".into())
            .spawn(move || convert_worker(&sh, &rx))
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        Ok(Self {
            shared,
            tx: Some(tx),
            devices: Mutex::new(Some(devices)),
            config: Mutex::new(config),
            null_until: None,
        })
    }

    /// A player with no device: audio is discarded, and [`Player::is_idle`] turns true once the
    /// pushed audio would have finished playing in real time. For tests and headless use.
    pub fn null() -> Self {
        Self {
            shared: Arc::new(Shared {
                outputs: RwLock::new(Vec::new()),
                generation: AtomicU64::new(0),
                in_flight: AtomicUsize::new(0),
                holding: AtomicBool::new(false),
                last_error: Mutex::new(None),
                epoch: Instant::now(),
            }),
            tx: None,
            devices: Mutex::new(None),
            config: Mutex::new(PlayerConfig::default()),
            null_until: Some(Mutex::new(Instant::now())),
        }
    }

    pub fn config(&self) -> PlayerConfig {
        lock(&self.config).clone()
    }

    /// Swap devices or volumes without restarting the app. Queued audio may be dropped.
    ///
    /// A volume-only change is applied in place, without touching the streams. Otherwise the new
    /// streams are opened first and the old ones closed only if that succeeded.
    pub fn reconfigure(&self, config: PlayerConfig) -> Result<(), AudioError> {
        let mut cur = lock(&self.config);
        if self.null_until.is_some() {
            *cur = config;
            return Ok(());
        }
        if cur.mic == config.mic && cur.monitor == config.monitor {
            let outputs = self.outputs();
            for o in &outputs {
                o.set_volume(if o.label.starts_with("mic") {
                    config.mic_volume
                } else {
                    config.monitor_volume
                });
            }
            *cur = config;
            return Ok(());
        }
        *lock(&self.shared.last_error) = None;
        let (devices, outputs) = open(&config, &self.shared)?;
        self.stop();
        *self
            .shared
            .outputs
            .write()
            .unwrap_or_else(|e| e.into_inner()) = outputs;
        if let Some(old) = lock(&self.devices).replace(devices) {
            old.close();
        }
        *cur = config;
        Ok(())
    }

    /// Appends to the playback queue (any rate/channels; resampled internally). Non-blocking.
    pub fn push(&self, chunk: &PcmChunk) {
        if chunk.samples.is_empty() || chunk.sample_rate == 0 {
            return;
        }
        if let Some(until) = &self.null_until {
            let mut u = lock(until);
            let start = (*u).max(Instant::now());
            *u = start + Duration::from_secs_f32(chunk.duration_secs());
            return;
        }
        let Some(tx) = &self.tx else { return };
        self.shared.in_flight.fetch_add(1, Ordering::SeqCst);
        let job = Job {
            generation: self.shared.generation.load(Ordering::SeqCst),
            chunk: chunk.clone(),
        };
        if tx.send(job).is_err() {
            self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Drops everything queued and silences output immediately (after a 5 ms fade, to avoid a click).
    pub fn stop(&self) {
        if let Some(until) = &self.null_until {
            *lock(until) = Instant::now();
            return;
        }
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        for o in self.outputs() {
            o.fade_out();
        }
    }

    /// True when nothing is queued or playing.
    pub fn is_idle(&self) -> bool {
        if let Some(until) = &self.null_until {
            return Instant::now() >= *lock(until);
        }
        if self.shared.in_flight.load(Ordering::SeqCst) > 0
            || self.shared.holding.load(Ordering::SeqCst)
        {
            return false;
        }
        let now = self.shared.now_nanos();
        self.outputs().iter().all(|o| {
            o.dead.load(Ordering::SeqCst)
                || (lock(&o.queue).is_empty() && o.busy_until.load(Ordering::SeqCst) <= now)
        })
    }

    /// The most recent device problem (failed open, device unplugged…), if any. Cleared by a
    /// successful [`Player::reconfigure`] to new devices.
    pub fn last_error(&self) -> Option<String> {
        lock(&self.shared.last_error).clone()
    }

    fn outputs(&self) -> Vec<Arc<Output>> {
        self.shared
            .outputs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(d) = lock(&self.devices).take() {
            d.close();
        }
    }
}

fn convert_worker(shared: &Shared, rx: &mpsc::Receiver<Job>) {
    let mut convs: Vec<(Arc<Output>, Option<Converter>)> = Vec::new();
    let mut generation = shared.generation.load(Ordering::SeqCst);
    loop {
        let holding = convs
            .iter()
            .any(|(_, c)| c.as_ref().is_some_and(Converter::holds_audio));
        shared.holding.store(holding, Ordering::SeqCst);
        let msg = if holding {
            rx.recv_timeout(Duration::from_millis(5))
        } else {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };

        let current_gen = shared.generation.load(Ordering::SeqCst);
        let outputs = shared
            .outputs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let same_outputs = convs.len() == outputs.len()
            && convs
                .iter()
                .zip(&outputs)
                .all(|((a, _), b)| Arc::ptr_eq(a, b));
        if current_gen != generation || !same_outputs {
            generation = current_gen;
            convs = outputs.into_iter().map(|o| (o, None)).collect();
        }

        match msg {
            Ok(job) => {
                if job.generation == generation {
                    let c = &job.chunk;
                    for (out, conv) in &mut convs {
                        if out.dead.load(Ordering::SeqCst) {
                            continue;
                        }
                        let fits = conv.as_ref().is_some_and(|cv| {
                            cv.in_rate == c.sample_rate && cv.in_channels == c.channels as usize
                        });
                        if !fits {
                            if let Some(old) = conv.as_mut() {
                                out.append(&shared.generation, generation, &old.flush());
                            }
                            *conv = Some(Converter::new(
                                c.sample_rate,
                                c.channels as usize,
                                out.rate,
                                out.channels,
                            ));
                        }
                        let samples = conv.as_mut().map(|cv| cv.feed(&c.samples));
                        out.append(&shared.generation, generation, &samples.unwrap_or_default());
                    }
                }
                // Recompute `holding` before releasing the chunk, so `is_idle` never sees a gap.
                let holding = convs
                    .iter()
                    .any(|(_, c)| c.as_ref().is_some_and(Converter::holds_audio));
                if holding {
                    shared.holding.store(true, Ordering::SeqCst);
                }
                shared.in_flight.fetch_sub(1, Ordering::SeqCst);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                for (out, conv) in &mut convs {
                    if let Some(cv) = conv.as_mut().filter(|c| c.holds_audio()) {
                        if out.dead.load(Ordering::SeqCst) {
                            cv.flush();
                        } else if out.buffered_secs() < FLUSH_BELOW_SECS {
                            out.append(&shared.generation, generation, &cv.flush());
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Resolves a device id from [`crate::list_outputs`] (`None` = system default).
fn find_device(host: &cpal::Host, id: Option<&str>) -> Result<cpal::Device, AudioError> {
    let Some(id) = id else {
        return host
            .default_output_device()
            .ok_or_else(|| AudioError::DeviceNotFound("system default output".into()));
    };
    let devices: Vec<cpal::Device> = host
        .output_devices()
        .map_err(|e| AudioError::Backend(e.to_string()))?
        .collect();
    let names: Vec<String> = devices.iter().map(crate::device_name).collect();
    assign_ids(&names)
        .iter()
        .position(|i| i == id)
        .map(|i| devices[i].clone())
        .ok_or_else(|| AudioError::DeviceNotFound(id.into()))
}

fn open(
    config: &PlayerConfig,
    shared: &Arc<Shared>,
) -> Result<(DeviceThread, Vec<Arc<Output>>), AudioError> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (shutdown, shutdown_rx) = mpsc::channel::<()>();
    let config = config.clone();
    let sh = shared.clone();
    let handle = std::thread::Builder::new()
        .name("soritong-devices".into())
        .spawn(move || {
            let host = cpal::default_host();
            let mut streams = Vec::new();
            let mut outputs = Vec::new();
            let mic = find_device(&host, config.mic.as_deref()).and_then(|d| {
                let (s, o) = build(&d, "mic", config.mic_volume, &sh)?;
                streams.push(s);
                outputs.push(o);
                Ok(d)
            });
            let mic = match mic {
                Ok(d) => d,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            if let Some(id) = config.monitor.as_deref() {
                let monitor = find_device(&host, Some(id)).and_then(|d| {
                    if d.id().ok().is_some_and(|id| mic.id().ok() == Some(id)) {
                        return Ok(()); // the same device twice would just play everything louder
                    }
                    let (s, o) = build(&d, "monitor", config.monitor_volume, &sh)?;
                    streams.push(s);
                    outputs.push(o);
                    Ok(())
                });
                if let Err(e) = monitor {
                    sh.report(format!("monitor output skipped: {e}"));
                }
            }
            let _ = ready_tx.send(Ok(outputs));
            // Keep the streams alive until the Player closes this thread.
            let _ = shutdown_rx.recv();
            drop(streams);
        })
        .map_err(|e| AudioError::Backend(e.to_string()))?;
    match ready_rx.recv() {
        Ok(Ok(outputs)) => Ok((DeviceThread { shutdown, handle }, outputs)),
        Ok(Err(e)) => {
            let _ = handle.join();
            shared.report(e.to_string());
            Err(e)
        }
        Err(_) => Err(AudioError::Backend("device thread exited".into())),
    }
}

fn build(
    device: &cpal::Device,
    role: &str,
    volume: f32,
    shared: &Arc<Shared>,
) -> Result<(cpal::Stream, Arc<Output>), AudioError> {
    let name = crate::device_name(device);
    let backend = |e: cpal::Error| AudioError::Backend(format!("{role} {name:?}: {e}"));
    let supported = device.default_output_config().map_err(backend)?;
    let out = Arc::new(Output {
        label: format!("{role} {name:?}"),
        rate: supported.sample_rate(),
        channels: supported.channels().max(1) as usize,
        queue: Mutex::new(VecDeque::new()),
        volume: AtomicU32::new(volume.max(0.0).to_bits()),
        dead: AtomicBool::new(false),
        busy_until: AtomicU64::new(0),
    });
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => stream::<f32>(device, config, &out, shared),
        SampleFormat::I16 => stream::<i16>(device, config, &out, shared),
        SampleFormat::I32 => stream::<i32>(device, config, &out, shared),
        SampleFormat::U16 => stream::<u16>(device, config, &out, shared),
        SampleFormat::F64 => stream::<f64>(device, config, &out, shared),
        other => {
            return Err(AudioError::Backend(format!(
                "{role} {name:?}: unsupported sample format {other:?}"
            )));
        }
    }
    .map_err(backend)?;
    stream.play().map_err(backend)?;
    log::info!(
        "{role} output {name:?}: {} Hz, {} ch",
        out.rate,
        out.channels
    );
    Ok((stream, out))
}

fn stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    out: &Arc<Output>,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, cpal::Error> {
    let (o, sh) = (out.clone(), shared.clone());
    let (o_err, sh_err) = (out.clone(), shared.clone());
    let frame_nanos = 1e9 / o.rate as f64;
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            let vol = f32::from_bits(o.volume.load(Ordering::Relaxed));
            let mut q = lock(&o.queue);
            let n = q.len().min(data.len());
            for (d, s) in data.iter_mut().zip(q.drain(..n)) {
                *d = T::from_sample(s * vol);
            }
            drop(q);
            for d in &mut data[n..] {
                *d = T::EQUILIBRIUM;
            }
            if n > 0 {
                // This buffer has just been handed to the device; it is audible for its length.
                let frames = data.len() / o.channels;
                let until = sh.now_nanos() + (frames as f64 * frame_nanos) as u64;
                o.busy_until.store(until, Ordering::SeqCst);
            }
        },
        move |e: cpal::Error| {
            if e.kind() == cpal::ErrorKind::DeviceNotAvailable {
                o_err.dead.store(true, Ordering::SeqCst);
                lock(&o_err.queue).clear();
                sh_err.report(format!("{} disappeared: {e}", o_err.label));
            } else {
                log::warn!("{}: {e}", o_err.label);
            }
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(secs: f32) -> PcmChunk {
        PcmChunk {
            sample_rate: 24_000,
            channels: 1,
            samples: vec![0.1; (24_000.0 * secs) as usize],
        }
    }

    #[test]
    fn null_player_is_busy_for_the_pushed_duration() {
        let p = Player::null();
        assert!(p.is_idle());
        p.push(&chunk(0.1));
        p.push(&chunk(0.1));
        assert!(!p.is_idle());
        std::thread::sleep(Duration::from_millis(120));
        assert!(!p.is_idle());
        std::thread::sleep(Duration::from_millis(120));
        assert!(p.is_idle());

        p.push(&chunk(5.0));
        p.stop();
        assert!(p.is_idle());
    }

    #[test]
    fn player_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Player>();
    }

    #[test]
    fn fade_out_keeps_five_ms_ramping_down() {
        let o = Output {
            label: "mic".into(),
            rate: 48_000,
            channels: 2,
            queue: Mutex::new(vec![1.0; 48_000].into()),
            volume: AtomicU32::new(1.0f32.to_bits()),
            dead: AtomicBool::new(false),
            busy_until: AtomicU64::new(0),
        };
        o.fade_out();
        let q = lock(&o.queue);
        assert_eq!(q.len(), 240 * 2);
        assert_eq!((q[0], q[1]), (1.0, 1.0));
        assert!(q[q.len() - 1] < 0.01);
    }

    /// Plays a 0.5 s tone on the default output. Needs a sound device:
    /// `cargo test -p soritong-audio -- --ignored plays_on_default_output --nocapture`
    #[test]
    #[ignore = "needs an audio output device"]
    fn plays_on_default_output() {
        let p = Player::new(PlayerConfig::default()).unwrap();
        let samples = (0..24_000)
            .map(|n| 0.2 * (std::f32::consts::TAU * 440.0 * n as f32 / 48_000.0).sin())
            .collect();
        p.push(&PcmChunk {
            sample_rate: 48_000,
            channels: 1,
            samples,
        });
        assert!(!p.is_idle());
        let start = Instant::now();
        while !p.is_idle() {
            assert!(start.elapsed() < Duration::from_secs(5), "never went idle");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(start.elapsed() >= Duration::from_millis(400));
        assert_eq!(p.last_error(), None);
    }
}
