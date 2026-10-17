use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct SnoozeJob {
    pub thread_id: String,
    pub wake_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct WorkflowScheduler {
    jobs: Vec<SnoozeJob>,
}

impl WorkflowScheduler {
    pub fn schedule_snooze(&mut self, thread_id: String, wake_at: DateTime<Utc>) {
        self.jobs.push(SnoozeJob { thread_id, wake_at });
    }

    pub fn due(&self, now: DateTime<Utc>) -> Vec<SnoozeJob> {
        self.jobs
            .iter()
            .filter(|job| job.wake_at <= now)
            .cloned()
            .collect()
    }
}
