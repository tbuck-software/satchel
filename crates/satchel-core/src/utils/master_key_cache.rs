use std::sync::{Condvar, Mutex};

type KeyResult = Result<[u8; 32], String>;

#[derive(Default)]
enum State {
    #[default]
    Empty,
    Loading,
    Ready(KeyResult),
}

#[derive(Default)]
pub(super) struct MasterKeyCache {
    state: Mutex<State>,
    ready: Condvar,
}

impl MasterKeyCache {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(State::Empty),
            ready: Condvar::new(),
        }
    }

    pub(super) fn start(&'static self, load: impl FnOnce() -> KeyResult + Send + 'static) {
        let mut state = self.state.lock().expect("master key cache poisoned");
        if !matches!(*state, State::Empty) {
            return;
        }
        *state = State::Loading;
        drop(state);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(load))
                .unwrap_or_else(|_| Err("master key loader panicked".to_string()));
            if let Err(err) = &result {
                tracing::error!(%err, "credential key unavailable; stored passwords cannot be unlocked");
            }
            let _ = self.finish(result);
        });
    }

    pub(super) fn load(&self, load: impl FnOnce() -> KeyResult) -> KeyResult {
        let mut state = self.state.lock().expect("master key cache poisoned");
        loop {
            match &*state {
                State::Ready(result) => return result.clone(),
                State::Loading => {
                    state = self.ready.wait(state).expect("master key cache poisoned")
                }
                State::Empty => {
                    *state = State::Loading;
                    drop(state);
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(load))
                        .unwrap_or_else(|_| Err("master key loader panicked".to_string()));
                    return self.finish(result);
                }
            }
        }
    }

    pub(super) async fn load_async(
        &'static self,
        load: impl FnOnce() -> KeyResult + Send + 'static,
    ) -> KeyResult {
        if let State::Ready(result) = &*self.state.lock().expect("master key cache poisoned") {
            return result.clone();
        }
        tokio::task::spawn_blocking(move || self.load(load))
            .await
            .map_err(|err| format!("credential key worker failed: {err}"))?
    }

    fn finish(&self, result: KeyResult) -> KeyResult {
        *self.state.lock().expect("master key cache poisoned") = State::Ready(result.clone());
        self.ready.notify_all();
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
    fn failure_is_shared_without_retrying_the_keychain() {
        let cache = MasterKeyCache::new();
        assert_eq!(cache.load(|| Err("locked".into())), Err("locked".into()));
        assert_eq!(
            cache.load(|| panic!("no competing load")),
            Err("locked".into())
        );
    }
}
