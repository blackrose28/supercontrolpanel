use std::{
    collections::VecDeque,
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use flate2::{write::GzEncoder, Compression};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::telemetry_queue::{
    telemetry_queue, ReadyTelemetryBatch, TelemetryQueue, TelemetryQueueRecord,
};
use super::telemetry_queue_disk::ensure_disk_queue_layout;
use crate::services::load_sync_settings_from_disk;

const SYNC_SETTINGS_FILE_NAME: &str = "sync-local.json";
static NEXT_TELEMETRY_SEQ: AtomicU64 = AtomicU64::new(1);
static TELEMETRY_WORKER_STARTED: AtomicBool = AtomicBool::new(false);
static TELEMETRY_DISK_LAYOUT_READY: AtomicBool = AtomicBool::new(false);
static RECENT_TELEMETRY: OnceLock<Mutex<VecDeque<Value>>> = OnceLock::new();
static TELEMETRY_THROTTLE: OnceLock<Mutex<TelemetryThrottleState>> = OnceLock::new();
static TELEMETRY_REDACTION_RULES: OnceLock<Vec<Regex>> = OnceLock::new();

const MAX_QUEUED_TELEMETRY_EVENTS: usize = 500;
const MAX_RECENT_TELEMETRY_EVENTS: usize = 100;
const TELEMETRY_IDLE_POLL_MS: u64 = 500;
const TELEMETRY_MAX_BACKOFF_SECS: u64 = 30;
const TELEMETRY_BATCH_WINDOW_MS: u64 = 15;
const TELEMETRY_BATCH_MAX_BYTES: usize = 16 * 1024;
const TELEMETRY_BATCH_MAX_EVENTS: usize = 64;
const TELEMETRY_GZIP_THRESHOLD_BYTES: usize = 8 * 1024;
const TELEMETRY_EVENT_SOFT_LIMIT_BYTES: usize = 16 * 1024;
const TELEMETRY_EVENT_CHUNK_TARGET_BYTES: usize = 8 * 1024;
const TELEMETRY_RATE_BYTES_PER_SEC: f64 = 256.0 * 1024.0;
const TELEMETRY_BURST_BYTES: f64 = TELEMETRY_RATE_BYTES_PER_SEC * 5.0;
const TELEMETRY_SERVER_BACKOFF_DEFAULT_MS: u64 = 1_000;
const TELEMETRY_SERVER_BACKOFF_MAX_MS: u64 = 5_000;

struct TelemetryThrottleState {
    tokens: f64,
    last_refill_at: Instant,
    server_backoff_until: Option<Instant>,
}

fn recent_telemetry() -> &'static Mutex<VecDeque<Value>> {
    RECENT_TELEMETRY.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn telemetry_throttle() -> &'static Mutex<TelemetryThrottleState> {
    TELEMETRY_THROTTLE.get_or_init(|| {
        Mutex::new(TelemetryThrottleState {
            tokens: TELEMETRY_BURST_BYTES,
            last_refill_at: Instant::now(),
            server_backoff_until: None,
        })
    })
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

fn ensure_telemetry_disk_layout(app: &AppHandle) {
    if TELEMETRY_DISK_LAYOUT_READY.load(Ordering::SeqCst) {
        return;
    }

    match ensure_disk_queue_layout(app) {
        Ok(layout) => {
            TELEMETRY_DISK_LAYOUT_READY.store(true, Ordering::SeqCst);
            if layout.recovery.truncated_bytes > 0 {
                eprintln!(
                    "Recovered CC telemetry queue by truncating {} partial byte(s) from {}",
                    layout.recovery.truncated_bytes,
                    layout.active_segment_path.display()
                );
            }
        }
        Err(err) => {
            eprintln!(
                "CC telemetry durable queue layout unavailable; continuing with in-memory queue: {err}"
            );
        }
    }
}

fn ensure_telemetry_worker_started(app: AppHandle) {
    if TELEMETRY_WORKER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let queue = telemetry_queue();

        loop {
            let Some(wait_for) = queue.next_wait(Duration::from_millis(TELEMETRY_IDLE_POLL_MS))
            else {
                queue.notifier().notified().await;
                continue;
            };

            if !wait_for.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(wait_for) => {}
                    _ = queue.notifier().notified() => {}
                }
                continue;
            }

            tokio::time::sleep(Duration::from_millis(TELEMETRY_BATCH_WINDOW_MS)).await;

            let Some(batch) =
                queue.prepare_ready_batch(TELEMETRY_BATCH_MAX_BYTES, TELEMETRY_BATCH_MAX_EVENTS)
            else {
                continue;
            };

            let throttle_delay = reserve_throttle_delay(batch.bytes);
            if !throttle_delay.is_zero() {
                tokio::time::sleep(throttle_delay).await;
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

            let request = client.post(format!(
                "{}/v1/logs",
                settings.cc_server_url.trim_end_matches('/')
            ));
            let Some(request) = build_ingest_request(request, &settings.cc_agent_token, &batch)
            else {
                schedule_retry(
                    queue,
                    batch.count,
                    "failed to build CC telemetry request".to_string(),
                );
                continue;
            };
            let send_result = request.send().await;

            match send_result {
                Ok(response) if response.status().is_success() => {
                    clear_server_backoff();
                    queue.drop_sent(batch.count);
                }
                Ok(response)
                    if response.status() == reqwest::StatusCode::UNAUTHORIZED
                        || response.status() == reqwest::StatusCode::FORBIDDEN =>
                {
                    clear_server_backoff();
                    queue.drop_sent(batch.count);
                    eprintln!(
                        "CC telemetry dropped {} event(s) after auth failure: server returned {}",
                        batch.count,
                        response.status(),
                    );
                }
                Ok(response) => {
                    let retry_after = parse_retry_after_ms(response.headers());
                    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error()
                    {
                        apply_server_backoff(retry_after);
                    }
                    schedule_retry(
                        queue,
                        batch.count,
                        format!("server returned {}", response.status()),
                    );
                }
                Err(err) => {
                    apply_server_backoff(None);
                    schedule_retry(queue, batch.count, err.to_string());
                }
            }
        }
    });
}

fn build_ingest_request(
    request: reqwest::RequestBuilder,
    agent_token: &str,
    batch: &ReadyTelemetryBatch,
) -> Option<reqwest::RequestBuilder> {
    let payload = serde_json::to_vec(&json!({ "events": batch.events })).ok()?;
    let request = request
        .bearer_auth(agent_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json");

    if payload.len() <= TELEMETRY_GZIP_THRESHOLD_BYTES {
        return Some(request.body(payload));
    }

    let compressed = gzip_bytes(&payload).ok()?;
    Some(
        request
            .header(reqwest::header::CONTENT_ENCODING, "gzip")
            .body(compressed),
    )
}

fn gzip_bytes(input: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(input)?;
    encoder.finish()
}

fn reserve_throttle_delay(bytes: usize) -> Duration {
    let Ok(mut state) = telemetry_throttle().lock() else {
        return Duration::ZERO;
    };

    let now = Instant::now();
    refill_throttle_tokens(&mut state, now);

    let server_backoff = state
        .server_backoff_until
        .map(|deadline| deadline.saturating_duration_since(now))
        .unwrap_or(Duration::ZERO);

    let required = bytes as f64;
    if state.tokens >= required {
        state.tokens -= required;
        return server_backoff;
    }

    let deficit = required - state.tokens;
    state.tokens = 0.0;
    let local_delay = Duration::from_secs_f64(deficit / TELEMETRY_RATE_BYTES_PER_SEC);
    server_backoff.max(local_delay)
}

fn refill_throttle_tokens(state: &mut TelemetryThrottleState, now: Instant) {
    let elapsed = now.saturating_duration_since(state.last_refill_at);
    state.last_refill_at = now;
    state.tokens = (state.tokens + elapsed.as_secs_f64() * TELEMETRY_RATE_BYTES_PER_SEC)
        .min(TELEMETRY_BURST_BYTES);
}

fn apply_server_backoff(retry_after_ms: Option<u64>) {
    let Ok(mut state) = telemetry_throttle().lock() else {
        return;
    };

    let now = Instant::now();
    refill_throttle_tokens(&mut state, now);
    let delay_ms = retry_after_ms
        .unwrap_or(TELEMETRY_SERVER_BACKOFF_DEFAULT_MS)
        .min(TELEMETRY_SERVER_BACKOFF_MAX_MS);
    let backoff_until = now + Duration::from_millis(delay_ms);

    state.server_backoff_until = Some(
        state
            .server_backoff_until
            .map(|existing| existing.max(backoff_until))
            .unwrap_or(backoff_until),
    );
}

fn clear_server_backoff() {
    let Ok(mut state) = telemetry_throttle().lock() else {
        return;
    };

    refill_throttle_tokens(&mut state, Instant::now());
    state.server_backoff_until = None;
}

fn parse_retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let text = value.to_str().ok()?.trim();
    let seconds = text.parse::<u64>().ok()?;
    Some(seconds.saturating_mul(1_000))
}

fn schedule_retry(queue: &dyn TelemetryQueue, count: usize, reason: String) {
    let schedule = queue.schedule_retry(count, TELEMETRY_MAX_BACKOFF_SECS);
    eprintln!(
        "CC telemetry send failed for {count} event(s): {reason}; retrying in {}s (attempt #{})",
        schedule.backoff_secs, schedule.attempt,
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
    let payload_value = redact_payload_value(payload_value);

    ensure_telemetry_disk_layout(app);

    let bodies = build_telemetry_bodies(event_type, session_id, function_id, payload_value);
    let queue = telemetry_queue();
    for body in bodies {
        record_recent_telemetry(&body);
        let seq = body.get("seq").and_then(Value::as_u64).unwrap_or(0);
        queue.enqueue(
            TelemetryQueueRecord::new(seq, body),
            MAX_QUEUED_TELEMETRY_EVENTS,
        );
    }
    ensure_telemetry_worker_started(app.clone());
}

pub fn recent_log_events(limit: usize) -> Vec<Value> {
    let Ok(recent) = recent_telemetry().lock() else {
        return Vec::new();
    };

    let start = recent.len().saturating_sub(limit);
    recent.iter().skip(start).cloned().collect()
}

fn build_telemetry_bodies(
    event_type: &str,
    session_id: Option<&str>,
    function_id: Option<&str>,
    payload: Value,
) -> Vec<Value> {
    let body = telemetry_body(event_type, session_id, function_id, payload.clone());
    let body_len = serde_json::to_vec(&body)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if body_len <= TELEMETRY_EVENT_SOFT_LIMIT_BYTES {
        return vec![body];
    }

    if let Some(chunks) = chunk_payload_value(payload) {
        return chunks
            .into_iter()
            .map(|chunk_payload| telemetry_body(event_type, session_id, function_id, chunk_payload))
            .collect();
    }

    vec![body]
}

fn telemetry_body(
    event_type: &str,
    session_id: Option<&str>,
    function_id: Option<&str>,
    payload: Value,
) -> Value {
    json!({
        "type": event_type,
        "sessionId": session_id,
        "functionId": function_id,
        "seq": NEXT_TELEMETRY_SEQ.fetch_add(1, Ordering::Relaxed),
        "ts": chrono::Utc::now().to_rfc3339(),
        "payload": payload,
    })
}

fn chunk_payload_value(payload: Value) -> Option<Vec<Value>> {
    if let Some(object) = payload.as_object() {
        if let Some(data) = object.get("data").and_then(Value::as_str) {
            let data_chunks = split_utf8_by_bytes(data, TELEMETRY_EVENT_CHUNK_TARGET_BYTES);
            if data_chunks.len() <= 1 {
                return None;
            }

            let total_chunks = data_chunks.len();
            let mut payloads = Vec::with_capacity(total_chunks);
            for (index, chunk) in data_chunks.into_iter().enumerate() {
                let mut chunk_object = object.clone();
                chunk_object.insert("data".to_string(), Value::String(chunk));
                chunk_object.insert("chunked".to_string(), Value::Bool(true));
                chunk_object.insert("chunkIndex".to_string(), json!(index));
                chunk_object.insert("totalChunks".to_string(), json!(total_chunks));
                payloads.push(Value::Object(chunk_object));
            }
            return Some(payloads);
        }
    }

    let payload_json = serde_json::to_string(&payload).ok()?;
    let json_chunks = split_utf8_by_bytes(&payload_json, TELEMETRY_EVENT_CHUNK_TARGET_BYTES);
    if json_chunks.len() <= 1 {
        return None;
    }

    let total_chunks = json_chunks.len();
    Some(
        json_chunks
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| {
                json!({
                    "chunked": true,
                    "chunkEncoding": "json-string",
                    "chunkIndex": index,
                    "totalChunks": total_chunks,
                    "chunk": chunk,
                })
            })
            .collect(),
    )
}

fn split_utf8_by_bytes(input: &str, max_bytes: usize) -> Vec<String> {
    if input.is_empty() || max_bytes == 0 {
        return vec![input.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_bytes = 0usize;

    for ch in input.chars() {
        let ch_len = ch.len_utf8();
        if !current.is_empty() && current_bytes + ch_len > max_bytes {
            chunks.push(current);
            current = String::new();
            current_bytes = 0;
        }
        current.push(ch);
        current_bytes += ch_len;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    chunks
}

fn redact_payload_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let redacted = map
                .into_iter()
                .map(|(key, entry)| {
                    if is_sensitive_key(&key) {
                        (key, Value::String("[redacted]".to_string()))
                    } else {
                        (key, redact_payload_value(entry))
                    }
                })
                .collect();
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_payload_value).collect()),
        Value::String(text) => Value::String(redact_sensitive_string(&text)),
        other => other,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key,
        "password"
            | "otpSecret"
            | "oidcPassword"
            | "oidcOtpSecret"
            | "mcpAuthToken"
            | "ccAgentToken"
            | "token"
            | "secret"
            | "authorization"
    )
}

fn redact_sensitive_string(input: &str) -> String {
    let mut output = input.to_string();
    for rule in telemetry_redaction_rules() {
        output = rule.replace_all(&output, "$prefix[redacted]").into_owned();
    }
    output
}

fn telemetry_redaction_rules() -> &'static [Regex] {
    TELEMETRY_REDACTION_RULES.get_or_init(|| {
        vec![
            Regex::new(r#"(?i)(?P<prefix>bearer\s+)[^\s"']+"#).expect("valid bearer redaction regex"),
            Regex::new(r#"(?im)(?P<prefix>\b(?:password|passwd|otp|totp|token|secret|authorization)\b\s*(?:=|:)\s*)[^\r\n]+"#)
                .expect("valid key-value redaction regex"),
            Regex::new(r#"(?im)(?P<prefix>\b(?:password|passwd|otp|totp|token|secret)\b\s+is\s+)[^\r\n]+"#)
                .expect("valid is-redaction regex"),
        ]
    })
}
