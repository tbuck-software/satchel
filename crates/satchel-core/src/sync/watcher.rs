use tokio::sync::broadcast;

use crate::providers::traits::MailEvent;

pub struct Watcher {
    tx: broadcast::Sender<MailEvent>,
}

impl Watcher {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<MailEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: MailEvent) {
        let _ = self.tx.send(event);
    }
}
