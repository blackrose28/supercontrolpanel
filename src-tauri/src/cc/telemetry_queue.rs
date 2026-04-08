use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use tokio::sync::Notify;

use super::telemetry_queue_disk::{
    drop_disk_sent, next_disk_queue_wait, prepare_disk_ready_batch, schedule_disk_retry,
};

static NEXT_TELEMETRY_LOCAL_ID: AtomicU64 = AtomicU64::new(1);
static TELEMETRY_QUEUE: OnceLock<Box<dyn TelemetryQueue>> = OnceLock::new();

pub(super) struct TelemetryQueueRecord {
    pub(super) local_id: u64,
    pub(super) seq: u64,
    pub(super) body: Value,
    pub(super) created_at_ms: u64,
    pub(super) created_at: Instant,
    pub(super) attempt: u32,
    pub(super) next_attempt_at_ms: u64,
    pub(super) next_attempt_at: Instant,
}

impl TelemetryQueueRecord {
    pub(super) fn new(seq: u64, body: Value) -> Self {
        let now = Instant::now();
        let now_ms = unix_time_now_ms();
        Self {
            local_id: NEXT_TELEMETRY_LOCAL_ID.fetch_add(1, Ordering::Relaxed),
            seq,
            body,
            created_at_ms: now_ms,
            created_at: now,
            attempt: 0,
            next_attempt_at_ms: now_ms,
            next_attempt_at: now,
        }
    }
}

pub(super) struct ReadyTelemetryBatch {
    pub(super) events: Vec<Value>,
    pub(super) count: usize,
    pub(super) bytes: usize,
}

pub(super) struct RetrySchedule {
    pub(super) attempt: u32,
    pub(super) backoff_secs: u64,
}

pub(super) trait TelemetryQueue: Send + Sync {
    fn enqueue(&self, record: TelemetryQueueRecord, max_queued_events: usize);
    fn forget_local_ids(&self, local_ids: &[u64]);
    fn notifier(&self) -> &Notify;
    fn next_wait(&self, idle_poll: Duration) -> Option<Duration>;
    fn prepare_ready_batch(
        &self,
        max_batch_bytes: usize,
        max_batch_events: usize,
    ) -> Option<ReadyTelemetryBatch>;
    fn drop_sent(&self, count: usize);
    fn schedule_retry(&self, count: usize, max_backoff_secs: u64) -> RetrySchedule;
}

pub(super) fn telemetry_queue() -> &'static dyn TelemetryQueue {
    TELEMETRY_QUEUE
        .get_or_init(|| Box::new(InMemoryTelemetryQueue::default()))
        .as_ref()
}

#[derive(Default)]
struct InMemoryTelemetryQueue {
    queue: Mutex<VecDeque<TelemetryQueueRecord>>,
    notify: Notify,
}

impl TelemetryQueue for InMemoryTelemetryQueue {
    fn enqueue(&self, record: TelemetryQueueRecord, max_queued_events: usize) {
        let Ok(mut queue) = self.queue.lock() else {
            return;
        };

        if queue.len() >= max_queued_events {
            queue.pop_front();
            eprintln!(
                "CC telemetry queue full; dropped oldest event to keep at most {} buffered items",
                max_queued_events
            );
        }

        queue.push_back(record);
        self.notify.notify_one();
    }

    fn forget_local_ids(&self, local_ids: &[u64]) {
        if local_ids.is_empty() {
            return;
        }

        let Ok(mut queue) = self.queue.lock() else {
            return;
        };
        queue.retain(|record| !local_ids.contains(&record.local_id));
    }

    fn notifier(&self) -> &Notify {
        &self.notify
    }

    fn next_wait(&self, idle_poll: Duration) -> Option<Duration> {
        if let Ok(wait_for) = next_disk_queue_wait(idle_poll) {
            return wait_for;
        }

        let Ok(queue) = self.queue.lock() else {
            return Some(idle_poll);
        };

        let event = queue.front()?;
        Some(
            event
                .next_attempt_at
                .saturating_duration_since(Instant::now()),
        )
    }

    fn prepare_ready_batch(
        &self,
        max_batch_bytes: usize,
        max_batch_events: usize,
    ) -> Option<ReadyTelemetryBatch> {
        if let Ok(batch) = prepare_disk_ready_batch(max_batch_bytes, max_batch_events) {
            return batch;
        }

        let Ok(queue) = self.queue.lock() else {
            return None;
        };

        let now = Instant::now();
        let mut events = Vec::new();
        let mut bytes = 0usize;

        for event in queue.iter() {
            let _ = (
                event.local_id,
                event.seq,
                event.created_at_ms,
                event.created_at,
                event.next_attempt_at_ms,
            );

            if event.next_attempt_at > now {
                break;
            }

            let event_bytes = serde_json::to_vec(&event.body).ok()?.len();
            let would_exceed = !events.is_empty()
                && (bytes + event_bytes > max_batch_bytes || events.len() >= max_batch_events);
            if would_exceed {
                break;
            }

            bytes += event_bytes;
            events.push(event.body.clone());
        }

        if events.is_empty() {
            return None;
        }

        let count = events.len();
        Some(ReadyTelemetryBatch {
            events,
            count,
            bytes,
        })
    }

    fn drop_sent(&self, count: usize) {
        let _ = drop_disk_sent(count);

        if let Ok(mut queue) = self.queue.lock() {
            for _ in 0..count {
                let _ = queue.pop_front();
            }
        }
    }

    fn schedule_retry(&self, count: usize, max_backoff_secs: u64) -> RetrySchedule {
        if let Ok(schedule) = schedule_disk_retry(count, max_backoff_secs) {
            if let Ok(mut queue) = self.queue.lock() {
                let next_attempt_at = Instant::now() + Duration::from_secs(schedule.backoff_secs);
                let next_attempt_at_ms =
                    unix_time_now_ms().saturating_add(schedule.backoff_secs.saturating_mul(1_000));
                for event in queue.iter_mut().take(count) {
                    event.attempt = schedule.attempt;
                    event.next_attempt_at_ms = next_attempt_at_ms;
                    event.next_attempt_at = next_attempt_at;
                }
            }
            return schedule;
        }

        let mut attempt = 1u32;

        if let Ok(mut queue) = self.queue.lock() {
            attempt = queue
                .iter()
                .take(count)
                .map(|event| event.attempt)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let backoff_secs = 2_u64.saturating_pow(attempt.min(5)).min(max_backoff_secs);
            let next_attempt_at = Instant::now() + Duration::from_secs(backoff_secs);
            let next_attempt_at_ms =
                unix_time_now_ms().saturating_add(backoff_secs.saturating_mul(1_000));

            for event in queue.iter_mut().take(count) {
                event.attempt = attempt;
                event.next_attempt_at_ms = next_attempt_at_ms;
                event.next_attempt_at = next_attempt_at;
            }
        }

        RetrySchedule {
            attempt,
            backoff_secs: 2_u64.saturating_pow(attempt.min(5)).min(max_backoff_secs),
        }
    }
}

fn unix_time_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
