//! Shared bounded history store and progress tracker for SCP file transfers.
use crate::models::{ScpTransferEntry, ScpTransferState};
use std::collections::HashMap;
use std::sync::Mutex;

/// Maximum number of SCP transfer history records retained in memory.
pub const MAX_TRANSFER_HISTORY: usize = 32;
const MAX_SESSION_OUTPUT: usize = 8000;

/// In-memory transfer history store shared between headless and desktop runtimes.
#[derive(Default)]
pub struct ScpTransferStore {
    pub entries: Mutex<Vec<ScpTransferEntry>>,
    pub session_to_transfer: Mutex<HashMap<String, String>>,
    pub session_output: Mutex<HashMap<String, String>>,
    pub next_id: Mutex<u64>,
}

macro_rules! guard_state {
    ($mutex:expr) => {
        $mutex
            .lock()
            .map_err(|e| format!("state lock error: {e}"))?
    };
}

/// Inserts a new pending SCP transfer record and trims history to `MAX_TRANSFER_HISTORY`.
pub fn create_scp_transfer_entry(
    store: &ScpTransferStore,
    now_ms: fn() -> u64,
    title: impl Into<String>,
    detail: impl Into<String>,
    source: Option<String>,
    destination: Option<String>,
) -> Result<String, String> {
    let now = now_ms();
    let mut next_id = guard_state!(store.next_id);
    *next_id += 1;
    let transfer_id = format!("scp-{}-{}", now, *next_id);
    drop(next_id);

    let entry = ScpTransferEntry {
        id: transfer_id.clone(),
        title: title.into(),
        detail: detail.into(),
        state: ScpTransferState::Pending,
        created_at_ms: now,
        updated_at_ms: now,
        progress_percent: Some(0),
        source,
        destination,
        session_id: None,
    };
    let mut entries = guard_state!(store.entries);
    entries.insert(0, entry);
    entries.truncate(MAX_TRANSFER_HISTORY);
    Ok(transfer_id)
}

/// Associates a running child session ID with an existing transfer entry.
pub fn attach_scp_transfer_session(
    store: &ScpTransferStore,
    transfer_id: &str,
    session_id: &str,
) -> Result<(), String> {
    {
        let mut entries = guard_state!(store.entries);
        if let Some(entry) = entries.iter_mut().find(|entry| entry.id == transfer_id) {
            entry.session_id = Some(session_id.to_string());
        }
    }
    guard_state!(store.session_to_transfer).insert(session_id.to_string(), transfer_id.to_string());
    guard_state!(store.session_output).insert(session_id.to_string(), String::new());
    Ok(())
}

/// Updates the state, detail message, and optional progress percentage of a transfer entry.
pub fn update_scp_transfer_entry(
    store: &ScpTransferStore,
    transfer_id: &str,
    state: ScpTransferState,
    detail: impl Into<String>,
    progress_percent: Option<u8>,
    now_ms: fn() -> u64,
) -> Result<(), String> {
    let now = now_ms();
    let detail = detail.into();
    let mut entries = guard_state!(store.entries);
    if let Some(entry) = entries.iter_mut().find(|entry| entry.id == transfer_id) {
        entry.state = state;
        entry.detail = detail;
        entry.updated_at_ms = now;
        entry.progress_percent = progress_percent;
    }
    Ok(())
}

/// Finalizes a session-backed SCP transfer entry using captured output or exit reason.
pub fn complete_scp_transfer_session(
    store: &ScpTransferStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
    now_ms: fn() -> u64,
) -> Result<(), String> {
    let transfer_id = guard_state!(store.session_to_transfer).remove(session_id);
    let output = guard_state!(store.session_output)
        .remove(session_id)
        .unwrap_or_default();
    if let Some(transfer_id) = transfer_id {
        let summary = summarize_transfer_output(&output, reason);
        let (state, detail, progress_percent) = if exit_code == Some(0) {
            let msg = if summary.is_empty() {
                "Transfer finished.".to_string()
            } else {
                summary
            };
            (ScpTransferState::Success, msg, Some(100))
        } else {
            let msg = if summary.is_empty() {
                "Transfer failed.".to_string()
            } else {
                summary
            };
            (ScpTransferState::Error, msg, None)
        };
        update_scp_transfer_entry(store, &transfer_id, state, detail, progress_percent, now_ms)?;
    }
    Ok(())
}

/// Appends transfer output bytes and updates progress percentage if a percent token appears.
pub fn handle_scp_transfer_output(
    store: &ScpTransferStore,
    session_id: &str,
    data: &str,
    now_ms: fn() -> u64,
) -> Result<(), String> {
    let transfer_id = {
        let guard = guard_state!(store.session_to_transfer);
        guard.get(session_id).cloned()
    };
    let Some(transfer_id) = transfer_id else {
        return Ok(());
    };

    let percent = {
        let mut outputs = guard_state!(store.session_output);
        let output = outputs.entry(session_id.to_string()).or_default();
        output.push_str(data);
        trim_to_tail_bytes(output, MAX_SESSION_OUTPUT);
        parse_scp_progress_percent(output)
    };

    if let Some(percent) = percent {
        update_scp_transfer_entry(
            store,
            &transfer_id,
            ScpTransferState::Pending,
            format!("Transferring… {percent}%"),
            Some(percent),
            now_ms,
        )?;
    }
    Ok(())
}

/// Returns all tracked SCP transfer entries sorted newest first.
pub fn list_scp_transfer_entries(
    store: &ScpTransferStore,
) -> Result<Vec<ScpTransferEntry>, String> {
    Ok(guard_state!(store.entries).clone())
}

/// Looks up one SCP transfer entry by ID.
pub fn get_scp_transfer_entry(
    store: &ScpTransferStore,
    transfer_id: &str,
) -> Result<Option<ScpTransferEntry>, String> {
    Ok(guard_state!(store.entries)
        .iter()
        .find(|entry| entry.id == transfer_id)
        .cloned())
}

fn trim_to_tail_bytes(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut keep_from = text.len() - max_bytes;
    while !text.is_char_boundary(keep_from) {
        keep_from += 1;
    }
    *text = text[keep_from..].to_string();
}

fn strip_ansi(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            let _ = chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_scp_progress_percent(output: &str) -> Option<u8> {
    let clean = strip_ansi(output);
    let mut last = None;
    for token in clean.split_whitespace() {
        if let Some(number) = token.strip_suffix('%') {
            if let Ok(value) = number.parse::<u16>() {
                last = Some(value.min(100) as u8);
            }
        }
    }
    last
}

fn summarize_transfer_output(output: &str, reason: &str) -> String {
    let clean = strip_ansi(output);
    let trimmed = clean.trim();
    if !trimmed.is_empty() {
        return trimmed
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(trimmed)
            .trim()
            .to_string();
    }
    reason.trim().to_string()
}
