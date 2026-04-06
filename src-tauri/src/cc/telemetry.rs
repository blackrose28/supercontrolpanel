use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::services::load_sync_settings_from_disk;

const SYNC_SETTINGS_FILE_NAME: &str = "sync-local.json";
static NEXT_TELEMETRY_SEQ: AtomicU64 = AtomicU64::new(1);
static TELEMETRY_WORKER_STARTED: AtomicBool = AtomicBool::new(false);
static TELEMETRY_QUEUE: OnceLock<Mutex<VecDeque<QueuedTelemetryEvent>>> = OnceLock::new();
static RECENT_TELEMETRY: OnceLock<Mutex<VecDeque<Value>>> = OnceLock::new();

const MAX_QUEUED_TELEMETRY_EVENTS: usize = 500;
const MAX_RECENT_TELEMETRY_EVENTS: usize = 100;
const TELEMETRY_IDLE_POLL_MS: u64 = 500;
const TELEMETRY_MAX_BACKOFF_SECS: u64 = 30;

#[derive(Clone)]
struct QueuedTelemetryEvent {
    body: Value,
    attempt: u32,
    next_attempt_at: Instant,
}

fn telemetry_queue() -> &'static Mutex<VecDeque<QueuedTelemetryEvent>> {
    TELEMETRY_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn recent_telemetry() -> &'static Mutex<VecDeque<Value>> {
    RECENT_TELEMETRY.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn enqueue_telemetry_event(event: QueuedTelemetryEvent) {
    let Ok(mut queue) = telemetry_queue().lock() else {
        return;
    };

    if queue.len() >= MAX_QUEUED_TELEMETRY_EVENTS {
        queue.pop_front();
        eprintln!(
            "CC telemetry queue full; dropped oldest event to keep at most {} buffered items",
            MAX_QUEUED_TELEMETRY_EVENTS
        );
    }

    queue.push_back(event);
}

fn record_recent_telemetry(body: &Value) {
    let Ok(mut recent) = recent_telemetry().lock() else {
        return;
    };

    if recent.len() >= MAX_RECENT_TELEMETRY_EVENTS {
        recent.pop_front();
    }

    recent.push_back(body.clone());
}

fn ensure_telemetry_worker_started(app: AppHandle) {
    if TELEMETRY_WORKER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();

        loop {
            let next_event = {
                let Ok(queue) = telemetry_queue().lock() else {
                    tokio::time::sleep(Duration::from_millis(TELEMETRY_IDLE_POLL_MS)).await;
                    continue;
                };

                queue.front().cloned()
            };

            let Some(event) = next_event else {
                tokio::time::sleep(Duration::from_millis(TELEMETRY_IDLE_POLL_MS)).await;
                continue;
            };

            let now = Instant::now();
            if event.next_attempt_at > now {
                tokio::time::sleep(
                    event
                        .next_attempt_at
                        .saturating_duration_since(now)
                        .min(Duration::from_secs(1)),
                )
                .await;
                continue;
            }

            let Ok(settings) = load_sync_settings_from_disk(&app, SYNC_SETTINGS_FILE_NAME) else {
                tokio::time::sleep(Duration::from_millis(TELEMETRY_IDLE_POLL_MS)).await;
                continue;
            };

            if !settings.cc_enabled
                || settings.cc_server_url.trim().is_empty()
                || settings.cc_agent_token.trim().is_empty()
            {
                tokio::time::sleep(Duration::from_millis(TELEMETRY_IDLE_POLL_MS)).await;
                continue;
            }

            let send_result = client
                .post(format!("{}/v1/logs", settings.cc_server_url.trim_end_matches('/')))
                .bearer_auth(&settings.cc_agent_token)
                .json(&event.body)
                .send()
                .await;

            match send_result {
                Ok(response) if response.status().is_success() => {
                    if let Ok(mut queue) = telemetry_queue().lock() {
                        let _ = queue.pop_front();
                    }
                }
                Ok(response)
                    if response.status() == reqwest::StatusCode::UNAUTHORIZED
                        || response.status() == reqwest::StatusCode::FORBIDDEN =>
                {
                    if let Ok(mut queue) = telemetry_queue().lock() {
                        let _ = queue.pop_front();
                    }
                    eprintln!(
                        "CC telemetry dropped event after auth failure: server returned {}",
                        response.status()
                    );
                }
                Ok(response) => {
                    schedule_retry(
                        &event,
                        format!("server returned {}", response.status()),
                    );
                }
                Err(err) => {
                    schedule_retry(&event, err.to_string());
                }
            }
        }
    });
}

fn schedule_retry(event: &QueuedTelemetryEvent, reason: String) {
    let attempt = event.attempt.saturating_add(1);
    let backoff_secs =
        2_u64.saturating_pow(attempt.min(5)).min(TELEMETRY_MAX_BACKOFF_SECS);

    if let Ok(mut queue) = telemetry_queue().lock() {
        if let Some(front) = queue.front_mut() {
            front.attempt = attempt;
            front.next_attempt_at = Instant::now() + Duration::from_secs(backoff_secs);
        }
    }

    eprintln!(
        "CC telemetry send failed: {reason}; retrying in {}s (attempt #{})",
        backoff_secs, attempt
    );
}

pub fn forward_log_event<T: Serialize>(
    app: &AppHandle,
    event_type: &str,
    session_id: Option<&str>,
    function_id: Option<&str>,
    payload: &T,
) {
    let Ok(settings) = load_sync_settings_from_disk(app, SYNC_SETTINGS_FILE_NAME) else {
        return;
    };

    if !settings.cc_enabled
        || settings.cc_server_url.trim().is_empty()
        || settings.cc_agent_token.trim().is_empty()
    {
        return;
    }

    let Ok(payload_value) = serde_json::to_value(payload) else {
        return;
    };

    let body = json!({
        "type": event_type,
        "sessionId": session_id,
        "functionId": function_id,
        "seq": NEXT_TELEMETRY_SEQ.fetch_add(1, Ordering::Relaxed),
        "ts": chrono::Utc::now().to_rfc3339(),
        "payload": payload_value,
    });

    record_recent_telemetry(&body);

    enqueue_telemetry_event(QueuedTelemetryEvent {
        body,
        attempt: 0,
        next_attempt_at: Instant::now(),
    });
    ensure_telemetry_worker_started(app.clone());
}

pub fn recent_log_events(limit: usize) -> Vec<Value> {
    let Ok(recent) = recent_telemetry().lock() else {
        return Vec::new();
    };

    let start = recent.len().saturating_sub(limit);
    recent.iter().skip(start).cloned().collect()
}
