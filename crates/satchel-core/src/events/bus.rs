use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum AppEvent {
    SyncStarted,
    SyncCompleted,
    ThreadsUpdated(Vec<String>),
    Notification {
        title: NotificationTitle,
        body: String,
    },
    NewMail {
        event_id: String,
        account_id: String,
        from_name: Option<String>,
        from_address: String,
        subject: String,
        preview: String,
        thread_id: String,
    },
    ConnectionState {
        online: bool,
        syncing: bool,
        last_sync_at: Option<String>,
        pending_outbox: u64,
        pending_sends: u64,
        failed_outbox: u64,
        failed_sends: u64,
        failed_remote_mutations: u64,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum NotificationTitle {
    SendFailed,
    SyncFailed,
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, event: AppEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn large_thread_change_set_uses_one_bus_slot() {
        let bus = EventBus::new(2);
        let mut receiver = bus.subscribe();
        bus.publish(AppEvent::ThreadsUpdated(
            (0..2000).map(|i| format!("thread-{i}")).collect(),
        ));
        bus.publish(AppEvent::SyncCompleted);
        assert!(
            matches!(receiver.recv().await.unwrap(), AppEvent::ThreadsUpdated(ids) if ids.len() == 2000)
        );
        assert!(matches!(
            receiver.recv().await.unwrap(),
            AppEvent::SyncCompleted
        ));
    }
}
