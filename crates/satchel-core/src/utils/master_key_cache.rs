use std::sync::{Arc, Mutex, OnceLock, PoisonError};

type KeyResult = Result<[u8; 32], String>;
#[derive(Default)]
struct Attempt {
    result: OnceLock<KeyResult>,
    ready: tokio::sync::Notify,
}

#[derive(Default)]
enum State {
    #[default]
    Empty,
    Loading(Arc<Attempt>),
    Ready([u8; 32]),
}

enum Claim {
    Cached([u8; 32]),
    Wait(Arc<Attempt>),
    Load(Arc<Attempt>),
}

#[derive(Default)]
pub(super) struct MasterKeyCache {
    state: Mutex<State>,
}

impl MasterKeyCache {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(State::Empty),
        }
    }

    fn claim(&self) -> Claim {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match &*state {
            State::Ready(key) => Claim::Cached(*key),
            State::Loading(attempt) => Claim::Wait(attempt.clone()),
            State::Empty => {
                let attempt = Arc::new(Attempt::default());
                *state = State::Loading(attempt.clone());
                Claim::Load(attempt)
            }
        }
    }

    pub(super) fn start(&'static self, load: impl FnOnce() -> KeyResult + Send + 'static) {
        let Claim::Load(attempt) = self.claim() else {
            return;
        };
        std::thread::spawn(move || {
            if let Err(err) = self.run_load(attempt, load) {
                tracing::error!(%err, "credential key unavailable; stored passwords cannot be unlocked");
            }
        });
    }

    pub(super) fn load(&self, load: impl FnOnce() -> KeyResult) -> KeyResult {
        self.resolve(self.claim(), load)
    }

    pub(super) async fn load_async(
        &'static self,
        load: impl FnOnce() -> KeyResult + Send + 'static,
    ) -> KeyResult {
        match self.claim() {
            Claim::Cached(key) => Ok(key),
            Claim::Wait(attempt) => {
                let ready = attempt.ready.notified();
                if attempt.result.get().is_none() {
                    ready.await;
                }
                attempt.result.wait().clone()
            }
            Claim::Load(attempt) => {
                tokio::task::spawn_blocking(move || self.run_load(attempt, load))
                    .await
                    .map_err(|err| format!("credential key worker failed: {err}"))?
            }
        }
    }

    fn resolve(&self, claim: Claim, load: impl FnOnce() -> KeyResult) -> KeyResult {
        match claim {
            Claim::Cached(key) => Ok(key),
            Claim::Wait(attempt) => attempt.result.wait().clone(),
            Claim::Load(attempt) => self.run_load(attempt, load),
        }
    }

    fn run_load(&self, attempt: Arc<Attempt>, load: impl FnOnce() -> KeyResult) -> KeyResult {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(load))
            .unwrap_or_else(|_| Err("master key loader panicked".to_string()));
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        *state = match &result {
            Ok(key) => State::Ready(*key),
            Err(_) => State::Empty,
        };
        // Waiters retain their attempt even if a new load starts immediately.
        let _ = attempt.result.set(result.clone());
        attempt.ready.notify_waiters();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn credential_users_wait_for_the_background_load_and_share_its_key() {
        let cache: &'static MasterKeyCache = Box::leak(Box::new(MasterKeyCache::new()));
        let (release, wait) = mpsc::channel();
        cache.start(move || {
            wait.recv().unwrap();
            Ok([9; 32])
        });
        let (result, received) = mpsc::channel();
        std::thread::spawn(move || {
            result
                .send(cache.load(|| panic!("a second key must never be loaded")))
                .unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(50)).is_err());
        release.send(()).unwrap();
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap(),
            [9; 32]
        );
        assert_eq!(cache.load(|| panic!("already cached")).unwrap(), [9; 32]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn awaiting_credentials_keeps_the_async_executor_available() {
        let cache: &'static MasterKeyCache = Box::leak(Box::new(MasterKeyCache::new()));
        let (release, wait) = mpsc::channel();
        cache.start(move || {
            wait.recv().unwrap();
            Ok([9; 32])
        });
        let reader = tokio::spawn(cache.load_async(|| panic!("no competing load")));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!reader.is_finished());
        release.send(()).unwrap();
        assert_eq!(reader.await.unwrap().unwrap(), [9; 32]);
    }

    #[test]
    fn waiters_get_the_failed_attempt_even_after_a_retry_succeeds() {
        let cache = MasterKeyCache::new();
        let Claim::Load(attempt) = cache.claim() else {
            panic!("expected a fresh attempt");
        };
        let first_waiter = cache.claim();
        let second_waiter = cache.claim();

        assert_eq!(
            cache.run_load(attempt, || Err("locked".into())),
            Err("locked".into())
        );
        assert_eq!(cache.load(|| Ok([7; 32])), Ok([7; 32]));
        for waiter in [first_waiter, second_waiter] {
            assert_eq!(
                cache.resolve(waiter, || panic!("already joined the first load")),
                Err("locked".into())
            );
        }
        assert_eq!(cache.load(|| panic!("already cached")), Ok([7; 32]));
    }

    #[test]
    fn a_poisoned_state_mutex_can_still_cache_the_key() {
        let cache = MasterKeyCache::new();
        let _ = std::panic::catch_unwind(|| {
            let _guard = cache.state.lock().unwrap();
            panic!("poison the cache mutex");
        });
        assert_eq!(cache.load(|| Ok([6; 32])), Ok([6; 32]));
        assert_eq!(cache.load(|| panic!("already cached")), Ok([6; 32]));
    }
}
