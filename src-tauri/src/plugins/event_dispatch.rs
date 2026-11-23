use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::mpsc;

use super::runtime::PluginRuntime;
use crate::events::bus::AppEvent;

const EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Debug)]
struct PluginEvent {
    name: &'static str,
    key: String,
    payload: serde_json::Value,
}

struct EventQueue {
    sender: mpsc::Sender<Arc<PluginEvent>>,
    sink: &'static str,
    dropping_events: bool,
}

impl EventQueue {
    fn channel(sink: &'static str, capacity: usize) -> (Self, mpsc::Receiver<Arc<PluginEvent>>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Self {
                sender,
                sink,
                dropping_events: false,
            },
            receiver,
        )
    }

    fn enqueue(&mut self, event: Arc<PluginEvent>) {
        // Keep the frontend event bridge moving even if either sink is slow.
        // A single warning covers each burst of dropped events.
        match self.sender.try_send(event) {
            Ok(()) => self.dropping_events = false,
            Err(error) if !self.dropping_events => {
                tracing::warn!(target: "events", sink = self.sink, %error, "dropping event deliveries");
                self.dropping_events = true;
            }
            Err(_) => {}
        }
    }
}

pub(crate) struct EventDispatch {
    plugins: EventQueue,
    webhooks: EventQueue,
}

impl EventDispatch {
    pub(crate) fn spawn(runtime: Arc<PluginRuntime>, pool: SqlitePool) -> Self {
        let (plugins, mut plugin_events) = EventQueue::channel("plugins", EVENT_QUEUE_CAPACITY);
        let (webhooks, mut webhook_events) = EventQueue::channel("webhooks", EVENT_QUEUE_CAPACITY);

        tauri::async_runtime::spawn(async move {
            while let Some(event) = plugin_events.recv().await {
                let runtime = runtime.clone();
                // Await each job before scheduling another blocking task.
                match tauri::async_runtime::spawn_blocking(move || {
                    runtime.dispatch_event(event.name, &event.key, event.payload.clone())
                })
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(target: "events", %error, "plugin event dispatch failed")
                    }
                    Err(error) => {
                        tracing::warn!(target: "events", %error, "plugin event worker failed")
                    }
                }
            }
        });
        tauri::async_runtime::spawn(async move {
            let repository = crate::db::webhooks::WebhookRepository::new(&pool);
            while let Some(event) = webhook_events.recv().await {
                if let Err(error) = repository
                    .enqueue_deliveries(event.name, &event.key, &event.payload)
                    .await
                {
                    tracing::warn!(target: "events", %error, "webhook delivery enqueue failed");
                }
            }
        });
        Self { plugins, webhooks }
    }

    pub(crate) fn enqueue(&mut self, event: &AppEvent) {
        if let Some(event) = plugin_event_payload(event) {
            let event = Arc::new(event);
            self.webhooks.enqueue(event.clone());
            self.plugins.enqueue(event);
        }
    }
}

fn plugin_event_payload(event: &AppEvent) -> Option<PluginEvent> {
    match event {
        AppEvent::ThreadsUpdated(thread_ids) => Some(PluginEvent {
            name: "thread.updated",
            key: format!("thread.updated:{}", thread_ids.join(",")),
            payload: serde_json::json!({ "thread_ids": thread_ids, "thread_id": if thread_ids.len() == 1 { thread_ids.first() } else { None } }),
        }),
        AppEvent::NewMail {
            event_id,
            account_id,
            from_name,
            from_address,
            subject,
            preview,
            thread_id,
        } => Some(PluginEvent {
            name: "message.received",
            key: format!("message.received:{account_id}:{event_id}"),
            payload: serde_json::json!({
                "event_id": event_id,
                "account_id": account_id,
                "thread_id": thread_id,
                "from_name": from_name,
                "from_address": from_address,
                "subject": subject,
                "preview": preview,
            }),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_change_payload_carries_the_whole_batch() {
        let event = AppEvent::ThreadsUpdated(vec!["a".into(), "b".into()]);
        let PluginEvent { name, payload, .. } = plugin_event_payload(&event).unwrap();
        assert_eq!(name, "thread.updated");
        assert_eq!(payload["thread_ids"], serde_json::json!(["a", "b"]));
        let event = AppEvent::ThreadsUpdated(vec!["a".into()]);
        let PluginEvent { payload, .. } = plugin_event_payload(&event).unwrap();
        assert_eq!(payload["thread_id"], "a");
    }

    #[test]
    fn a_saturated_sink_drops_the_new_event_and_resumes_after_draining() {
        let (plugins, mut plugin_events) = EventQueue::channel("plugins", 1);
        let (webhooks, mut webhook_events) = EventQueue::channel("webhooks", 2);
        let mut dispatch = EventDispatch { plugins, webhooks };

        dispatch.enqueue(&AppEvent::ThreadsUpdated(vec!["first".into()]));
        dispatch.enqueue(&AppEvent::ThreadsUpdated(vec!["second".into()]));

        assert!(dispatch.plugins.dropping_events);
        assert_eq!(
            plugin_events.try_recv().unwrap().key,
            "thread.updated:first"
        );
        assert!(plugin_events.try_recv().is_err());
        assert_eq!(
            webhook_events.try_recv().unwrap().key,
            "thread.updated:first"
        );
        assert_eq!(
            webhook_events.try_recv().unwrap().key,
            "thread.updated:second"
        );

        dispatch.enqueue(&AppEvent::ThreadsUpdated(vec!["third".into()]));
        assert_eq!(
            plugin_events.try_recv().unwrap().key,
            "thread.updated:third"
        );
        assert!(!dispatch.plugins.dropping_events);
    }
}
