//! The speak queue: one utterance at a time, sentence by sentence, through the cache and player.
//!
//! Each sentence's chunks are pushed as they arrive and [`Player::push`] never blocks, so sentence
//! N+1 is synthesised while sentence N plays. `Finished` waits for the player to go idle, i.e. for
//! the last sample to have been played.

use crate::{Player, lock};
use soritong_core::cache::{SynthCache, cache_key, concat};
use soritong_core::text::{detect_lang, split_sentences};
use soritong_core::{Cancel, Lang, PcmChunk, SynthRequest, TtsEngine, TtsError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

pub type UtteranceId = u64;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpeakEvent {
    /// Synthesis began for this utterance.
    Started {
        id: UtteranceId,
    },
    /// The last sample of this utterance has been played.
    Finished {
        id: UtteranceId,
    },
    /// Stopped by [`Speaker::stop`] before finishing.
    Cancelled {
        id: UtteranceId,
    },
    Failed {
        id: UtteranceId,
        message: String,
    },
}

/// The queue the UI talks to: utterances are synthesised sentence by sentence and played in
/// order, one at a time; [`Speaker::stop`] cancels the current one and clears the queue.
pub struct Speaker {
    tx: mpsc::Sender<Job>,
    shared: Arc<Shared>,
    player: Arc<Player>,
    next_id: AtomicU64,
}

struct Shared {
    /// Bumped by `stop`; jobs queued under an older generation are dropped.
    generation: AtomicU64,
    /// Held while pushing audio and while stopping, so no chunk can slip in after a stop.
    current: Mutex<Cancel>,
}

struct Job {
    id: UtteranceId,
    generation: u64,
    engine: Arc<dyn TtsEngine>,
    req: SynthRequest,
}

impl Speaker {
    /// `on_event` is called from the worker thread.
    pub fn new(player: Arc<Player>, on_event: impl Fn(SpeakEvent) + Send + 'static) -> Self {
        Self::with_cache(player, None, on_event)
    }

    /// Like [`Speaker::new`], replaying sentences from `cache` when it has them and storing
    /// every sentence that was synthesised to the end.
    pub fn with_cache(
        player: Arc<Player>,
        cache: Option<SynthCache>,
        on_event: impl Fn(SpeakEvent) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            current: Mutex::new(Cancel::new()),
        });
        let (sh, pl) = (shared.clone(), player.clone());
        std::thread::Builder::new()
            .name("soritong-speaker".into())
            .spawn(move || {
                for job in rx {
                    let cancel = Cancel::new();
                    {
                        // Checked under the lock so a concurrent `stop` either sees this token or
                        // has already bumped the generation.
                        let mut cur = lock(&sh.current);
                        if job.generation != sh.generation.load(Ordering::SeqCst) {
                            on_event(SpeakEvent::Cancelled { id: job.id });
                            continue;
                        }
                        *cur = cancel.clone();
                    }
                    on_event(SpeakEvent::Started { id: job.id });
                    let res = speak_job(&job, &cancel, &pl, &sh, cache.as_ref()).and_then(|()| {
                        while !pl.is_idle() {
                            if cancel.is_cancelled() {
                                return Err(TtsError::Cancelled);
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Ok(())
                    });
                    on_event(match res {
                        Ok(()) => SpeakEvent::Finished { id: job.id },
                        Err(TtsError::Cancelled) => SpeakEvent::Cancelled { id: job.id },
                        Err(e) => SpeakEvent::Failed {
                            id: job.id,
                            message: e.to_string(),
                        },
                    });
                }
            })
            .expect("spawn speaker thread");
        Self {
            tx,
            shared,
            player,
            next_id: AtomicU64::new(1),
        }
    }

    /// Queues an utterance and returns its id immediately.
    pub fn speak(&self, engine: Arc<dyn TtsEngine>, req: SynthRequest) -> UtteranceId {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let generation = self.shared.generation.load(Ordering::SeqCst);
        let _ = self.tx.send(Job {
            id,
            generation,
            engine,
            req,
        });
        id
    }

    /// Cancels the current utterance and everything queued before this call, and silences the
    /// player.
    pub fn stop(&self) {
        let cur = lock(&self.shared.current);
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        cur.cancel();
        self.player.stop();
    }
}

fn speak_job(
    job: &Job,
    cancel: &Cancel,
    player: &Player,
    shared: &Shared,
    cache: Option<&SynthCache>,
) -> Result<(), TtsError> {
    let push = |c: &PcmChunk| {
        let _guard = lock(&shared.current);
        if !cancel.is_cancelled() {
            player.push(c);
        }
    };
    for text in split_sentences(&job.req.text) {
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }
        let lang = match job.req.lang {
            Lang::Auto => detect_lang(&text),
            l => l,
        };
        let req = SynthRequest {
            text,
            lang,
            ..job.req.clone()
        };
        let key = cache.map(|_| cache_key(&*job.engine, &req));
        if let Some(hit) = cache.zip(key.as_deref()).and_then(|(c, k)| c.get(k)) {
            push(&hit);
            continue;
        }
        let mut chunks = Vec::new();
        job.engine.synthesize(&req, cancel, &mut |c| {
            push(&c);
            if cache.is_some() {
                chunks.push(c);
            }
        })?;
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }
        if let (Some(c), Some(k), Some(pcm)) = (cache, key, concat(&chunks)) {
            if let Err(e) = c.put(&k, &pcm) {
                log::warn!("cache write failed: {e}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soritong_core::mock::MockEngine;
    use soritong_core::{Voice, VoiceKind};
    use std::time::Instant;

    /// Wraps the mock to record what the engine was asked for.
    #[derive(Default)]
    struct Counting {
        inner: MockEngine,
        calls: Mutex<Vec<(String, Lang)>>,
        /// Per chunk, to make synthesis slower than instant so a stop lands mid-sentence.
        delay: Duration,
    }

    impl TtsEngine for Counting {
        fn id(&self) -> &'static str {
            "mock"
        }
        fn display_name(&self) -> &str {
            "counting mock"
        }
        fn is_offline(&self) -> bool {
            true
        }
        fn voices(&self) -> soritong_core::Result<Vec<Voice>> {
            Ok(vec![Voice {
                id: "beep".into(),
                name: "Beep".into(),
                langs: vec![],
                kind: VoiceKind::Stock,
            }])
        }
        fn synthesize(
            &self,
            req: &SynthRequest,
            cancel: &Cancel,
            on_chunk: &mut dyn FnMut(PcmChunk),
        ) -> soritong_core::Result<()> {
            lock(&self.calls).push((req.text.clone(), req.lang));
            self.inner.synthesize(req, cancel, &mut |c| {
                std::thread::sleep(self.delay);
                on_chunk(c)
            })
        }
    }

    fn req(t: &str, lang: Lang) -> SynthRequest {
        SynthRequest {
            text: t.into(),
            voice_id: "beep".into(),
            lang,
            speed: 1.0,
        }
    }

    fn speaker(cache: Option<SynthCache>) -> (Speaker, impl Fn() -> SpeakEvent) {
        let (tx, rx) = mpsc::channel();
        let s = Speaker::with_cache(Arc::new(Player::null()), cache, move |e| {
            tx.send(e).unwrap();
        });
        (s, move || rx.recv_timeout(Duration::from_secs(5)).unwrap())
    }

    fn temp_cache(name: &str) -> SynthCache {
        let dir =
            std::env::temp_dir().join(format!("soritong-speaker-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        SynthCache::new(dir)
    }

    #[test]
    fn speaks_in_order_and_stop_drops_the_queue() {
        let (speaker, recv) = speaker(None);
        let engine: Arc<dyn TtsEngine> = Arc::new(MockEngine::default());

        let a = speaker.speak(engine.clone(), req("하나", Lang::Ko));
        assert_eq!(recv(), SpeakEvent::Started { id: a });
        assert_eq!(recv(), SpeakEvent::Finished { id: a });

        speaker.stop();
        let b = speaker.speak(engine, req("둘", Lang::Ko));
        assert_eq!(recv(), SpeakEvent::Started { id: b });
        assert_eq!(recv(), SpeakEvent::Finished { id: b });
    }

    #[test]
    fn finished_waits_for_playback_and_utterances_do_not_overlap() {
        let (speaker, recv) = speaker(None);
        let engine: Arc<dyn TtsEngine> = Arc::new(MockEngine::default());
        let start = Instant::now();
        let a = speaker.speak(engine.clone(), req("하나. 둘!", Lang::Ko)); // 3 tones, 360 ms
        let b = speaker.speak(engine, req("셋", Lang::Ko));
        assert_eq!(recv(), SpeakEvent::Started { id: a });
        assert_eq!(recv(), SpeakEvent::Finished { id: a });
        assert!(start.elapsed() >= Duration::from_millis(340));
        assert_eq!(recv(), SpeakEvent::Started { id: b });
        assert_eq!(recv(), SpeakEvent::Finished { id: b });
        assert!(start.elapsed() >= Duration::from_millis(460));
    }

    #[test]
    fn auto_language_is_picked_per_sentence() {
        let (speaker, recv) = speaker(None);
        let engine = Arc::new(Counting::default());
        let id = speaker.speak(
            engine.clone(),
            req("안녕하세요. Nice to meet you!", Lang::Auto),
        );
        assert_eq!(recv(), SpeakEvent::Started { id });
        assert_eq!(recv(), SpeakEvent::Finished { id });
        assert_eq!(
            *lock(&engine.calls),
            vec![
                ("안녕하세요.".to_string(), Lang::Ko),
                ("Nice to meet you!".to_string(), Lang::En)
            ]
        );
    }

    #[test]
    fn stop_mid_utterance_cancels_it_and_the_queue_and_silences_the_player() {
        let cache = temp_cache("stop");
        let (tx, rx) = mpsc::channel();
        let player = Arc::new(Player::null());
        let speaker = Speaker::with_cache(player.clone(), Some(cache.clone()), move |e| {
            tx.send(e).unwrap();
        });
        let recv = || rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let slow = Arc::new(Counting {
            delay: Duration::from_millis(20),
            ..Counting::default()
        });
        let engine: Arc<dyn TtsEngine> = slow.clone();
        let long = "가나다라마바사아자차카타파하 가나다라마바사아자차카타파하.";
        let a = speaker.speak(engine.clone(), req(long, Lang::Ko));
        let b = speaker.speak(engine.clone(), req("둘", Lang::Ko));
        assert_eq!(recv(), SpeakEvent::Started { id: a });
        std::thread::sleep(Duration::from_millis(100));
        assert!(!player.is_idle());

        let t = Instant::now();
        speaker.stop();
        assert!(player.is_idle());
        assert_eq!(recv(), SpeakEvent::Cancelled { id: a });
        assert_eq!(recv(), SpeakEvent::Cancelled { id: b });
        assert!(t.elapsed() < Duration::from_secs(1));
        assert!(
            cache
                .get(&cache_key(&*slow, &req(long, Lang::Ko)))
                .is_none(),
            "a cancelled sentence is not cached"
        );

        // Speaking works again after a stop.
        let c = speaker.speak(engine, req("셋", Lang::Ko));
        assert_eq!(recv(), SpeakEvent::Started { id: c });
        assert_eq!(recv(), SpeakEvent::Finished { id: c });
        std::fs::remove_dir_all(cache.dir()).ok();
    }

    #[test]
    fn cache_hit_skips_the_engine() {
        let cache = temp_cache("hit");
        let (speaker, recv) = speaker(Some(cache.clone()));
        let engine = Arc::new(Counting::default());
        let say = |text: &str| {
            let id = speaker.speak(engine.clone(), req(text, Lang::Ko));
            assert_eq!(recv(), SpeakEvent::Started { id });
            assert_eq!(recv(), SpeakEvent::Finished { id });
        };
        say("네 맞아요. 감사합니다.");
        assert_eq!(lock(&engine.calls).len(), 2);
        say("네 맞아요. 감사합니다.");
        assert_eq!(
            lock(&engine.calls).len(),
            2,
            "second time comes from the cache"
        );
        say("감사합니다. 또 봐요.");
        assert_eq!(
            lock(&engine.calls).len(),
            3,
            "only the new sentence is synthesised"
        );
        std::fs::remove_dir_all(cache.dir()).ok();
    }

    #[test]
    fn engine_errors_are_reported() {
        let (speaker, recv) = speaker(None);
        let engine: Arc<dyn TtsEngine> = Arc::new(MockEngine::default());
        let mut r = req("하나", Lang::Ko);
        r.voice_id = "nope".into();
        let id = speaker.speak(engine, r);
        assert_eq!(recv(), SpeakEvent::Started { id });
        assert!(matches!(recv(), SpeakEvent::Failed { id: i, .. } if i == id));
    }
}
