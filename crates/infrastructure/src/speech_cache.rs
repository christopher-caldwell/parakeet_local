use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub const IDLE_TTL: Duration = Duration::from_secs(60);
pub struct Cached<T> {
    pub key: String,
    pub model: T,
    pub last_used: Instant,
}
#[derive(Default)]
pub struct SpeechCache {
    pub whisper: Mutex<Option<Cached<whisper_rs::WhisperContext>>>,
    pub embedding: Mutex<Option<Cached<speakrs::inference::EmbeddingModel>>>,
    cleanup_started: AtomicBool,
}
impl SpeechCache {
    pub fn start_cleanup(this: &Arc<Self>) {
        if this.cleanup_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(this);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let Some(cache) = weak.upgrade() else { break };
                let _ = tokio::task::spawn_blocking(move || {
                    // Never wait for inference, and drop SDK resources off the UI
                    // thread and outside the cache locks.
                    let whisper = cache
                        .whisper
                        .try_lock()
                        .ok()
                        .and_then(|mut slot| take_expired(&mut slot));
                    let embedding = cache
                        .embedding
                        .try_lock()
                        .ok()
                        .and_then(|mut slot| take_expired(&mut slot));
                    drop((whisper, embedding));
                })
                .await;
            }
        });
    }
}
fn take_expired<T>(slot: &mut Option<Cached<T>>) -> Option<Cached<T>> {
    if slot
        .as_ref()
        .is_some_and(|c| c.last_used.elapsed() >= IDLE_TTL)
    {
        slot.take()
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_eviction_does_not_remove_recent_models() {
        let mut slot = Some(Cached {
            key: "fixture".into(),
            model: (),
            last_used: Instant::now(),
        });
        assert!(take_expired(&mut slot).is_none());
        assert!(slot.is_some());
        slot.as_mut().unwrap().last_used = Instant::now() - IDLE_TTL;
        assert!(take_expired(&mut slot).is_some());
        assert!(slot.is_none());
    }
}
