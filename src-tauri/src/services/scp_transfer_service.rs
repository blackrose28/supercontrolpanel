use crate::{
    models::{ScpTransferEntry, ScpTransferState},
    services::lock_err,
    stores::ScpTransferStore,
};

const MAX_TRANSFER_HISTORY: usize = 32;
const MAX_SESSION_OUTPUT: usize = 8000;

pub fn create_scp_transfer_entry(
    store: &ScpTransferStore,
    now_ms: fn() -> u64,
    title: impl Into<String>,
    detail: impl Into<String>,
    source: Option<String>,
    destination: Option<String>,
) -> Result<String, String> {
    let now = now_ms();
    let mut next_id = lock_err(&store.next_id)?;
    let transfer_id = format!("scp-{}-{}", now, *next_id);
    *next_id += 1;
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
    let mut entries = lock_err(&store.entries)?;
    entries.insert(0, entry);
    entries.truncate(MAX_TRANSFER_HISTORY);
    Ok(transfer_id)
}

pub fn attach_scp_transfer_session(
    store: &ScpTransferStore,
    transfer_id: &str,
    session_id: &str,
) -> Result<(), String> {
    {
        let mut entries = lock_err(&store.entries)?;
        if let Some(entry) = entries.iter_mut().find(|entry| entry.id == transfer_id) {
            entry.session_id = Some(session_id.to_string());
        }
    }
    lock_err(&store.session_to_transfer)?
        .insert(session_id.to_string(), transfer_id.to_string());
    lock_err(&store.session_output)?
        .insert(session_id.to_string(), String::new());
    Ok(())
}

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
    let mut entries = lock_err(&store.entries)?;
    if let Some(entry) = entries.iter_mut().find(|entry| entry.id == transfer_id) {
        entry.state = state;
        entry.detail = detail;
        entry.updated_at_ms = now;
        entry.progress_percent = progress_percent;
    }
    Ok(())
}

pub fn complete_scp_transfer_session(
    store: &ScpTransferStore,
    session_id: &str,
    exit_code: Option<i32>,
    reason: &str,
    now_ms: fn() -> u64,
) -> Result<(), String> {
    let transfer_id = lock_err(&store.session_to_transfer)?.remove(session_id);
    let output = lock_err(&store.session_output)?
        .remove(session_id)
        .unwrap_or_default();
    if let Some(transfer_id) = transfer_id {
        let summary = summarize_transfer_output(&output, reason);
        let (state, detail, progress_percent) = if exit_code == Some(0) {
            (
                ScpTransferState::Success,
                if summary.is_empty() {
                    "Transfer finished.".to_string()
                } else {
                    summary
                },
                Some(100),
            )
        } else {
            (
                ScpTransferState::Error,
                if summary.is_empty() {
                    "Transfer failed.".to_string()
                } else {
                    summary
                },
                None,
            )
        };
        update_scp_transfer_entry(store, &transfer_id, state, detail, progress_percent, now_ms)?;
    }
    Ok(())
}

pub fn handle_scp_transfer_output(
    store: &ScpTransferStore,
    session_id: &str,
    data: &str,
    now_ms: fn() -> u64,
) -> Result<(), String> {
    let transfer_id = {
        let guard = lock_err(&store.session_to_transfer)?;
        guard.get(session_id).cloned()
    };
    let Some(transfer_id) = transfer_id else {
        return Ok(());
    };

    let percent = {
        let mut outputs = lock_err(&store.session_output)?;
        let output = outputs.entry(session_id.to_string()).or_default();
        output.push_str(data);
        if output.len() > MAX_SESSION_OUTPUT {
            let keep_from = output.len() - MAX_SESSION_OUTPUT;
            *output = output[keep_from..].to_string();
        }
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

pub fn list_scp_transfer_entries(store: &ScpTransferStore) -> Result<Vec<ScpTransferEntry>, String> {
    Ok(lock_err(&store.entries)?.clone())
}

pub fn get_scp_transfer_entry(
    store: &ScpTransferStore,
    transfer_id: &str,
) -> Result<Option<ScpTransferEntry>, String> {
    Ok(lock_err(&store.entries)?
        .iter()
        .find(|entry| entry.id == transfer_id)
        .cloned())
}

fn strip_ansi(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                let _ = chars.next();
                while let Some(next) = chars.next() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
                continue;
            }
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
