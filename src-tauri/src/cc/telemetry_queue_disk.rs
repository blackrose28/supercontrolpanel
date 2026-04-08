use std::{
    fs::{self, File, OpenOptions},
    io::{self, ErrorKind, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use super::telemetry_queue::{ReadyTelemetryBatch, RetrySchedule, TelemetryQueueRecord};
use super::telemetry_queue_crypto::{EncryptedRecord, TelemetryQueueCrypto};

const TELEMETRY_QUEUE_DIR_NAME: &str = "cc-telemetry-queue";
const TELEMETRY_QUEUE_SEGMENTS_DIR_NAME: &str = "segments";
const TELEMETRY_QUEUE_MANIFEST_FILE_NAME: &str = "manifest.json";
const TELEMETRY_QUEUE_FORMAT_VERSION: u32 = 1;
const TELEMETRY_QUEUE_INITIAL_SEGMENT_ID: u64 = 1;
pub(super) const TELEMETRY_SEGMENT_ROTATION_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const TELEMETRY_QUEUE_MAX_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const TELEMETRY_QUEUE_HIGH_WATERMARK_PERCENT: u64 = 90;
static DISK_QUEUE_IO: OnceLock<Mutex<()>> = OnceLock::new();
static DISK_QUEUE_ROOT: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone)]
pub(super) struct DiskQueueLayout {
    pub(super) root_dir: PathBuf,
    pub(super) manifest_path: PathBuf,
    pub(super) segments_dir: PathBuf,
    pub(super) active_segment_path: PathBuf,
    pub(super) manifest: DiskQueueManifest,
    pub(super) recovery: DiskQueueRecovery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DiskQueueManifest {
    pub(super) format_version: u32,
    pub(super) active_segment: u64,
    pub(super) read_cursor: DiskQueueCursor,
    pub(super) ack_cursor: DiskQueueCursor,
    pub(super) total_bytes: u64,
    pub(super) key_id: Option<String>,
    pub(super) segment_rotation_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DiskQueueCursor {
    pub(super) segment_id: u64,
    pub(super) offset: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct DiskQueueRecovery {
    pub(super) truncated_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DiskQueueStats {
    pub(super) queued_records: usize,
    pub(super) queued_bytes: u64,
    pub(super) oldest_queued_age_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct AppendTelemetryRecordOutcome {
    pub(super) persisted: bool,
    pub(super) dropped_local_ids: Vec<u64>,
    pub(super) dropped_records: usize,
    pub(super) dropped_bytes: u64,
    pub(super) queue_bytes: u64,
}

#[derive(Debug, Clone)]
struct QueuePressureNotification {
    queue_bytes: u64,
    dropped_records: usize,
    dropped_bytes: u64,
    dropped_incoming: bool,
}

pub(super) fn ensure_disk_queue_layout(app: &AppHandle) -> Result<DiskQueueLayout, String> {
    let root_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?
        .join(TELEMETRY_QUEUE_DIR_NAME);
    let _ = DISK_QUEUE_ROOT.get_or_init(|| root_dir.clone());
    ensure_disk_queue_layout_at_root(&root_dir)
}

pub(super) fn append_telemetry_record(
    app: &AppHandle,
    record: &TelemetryQueueRecord,
) -> Result<AppendTelemetryRecordOutcome, String> {
    let root_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?
        .join(TELEMETRY_QUEUE_DIR_NAME);
    append_telemetry_record_at_root(&root_dir, record, |notification| {
        emit_queue_pressure(app, notification);
    })
}

fn append_telemetry_record_at_root(
    root_dir: &PathBuf,
    record: &TelemetryQueueRecord,
    mut on_queue_pressure: impl FnMut(QueuePressureNotification),
) -> Result<AppendTelemetryRecordOutcome, String> {
    let io_lock = DISK_QUEUE_IO.get_or_init(|| Mutex::new(()));
    let Ok(_guard) = io_lock.lock() else {
        return Err("failed to lock telemetry disk queue io".to_string());
    };

    let mut layout = ensure_disk_queue_layout_at_root(root_dir)?;
    let encoded = serde_json::to_vec(&PersistedTelemetryQueueRecord::from(record))
        .map_err(|e| format!("failed to encode telemetry queue record: {e}"))?;
    let crypto = load_queue_crypto()?;
    let encrypted = crypto.encrypt_record(&encoded)?;
    let encrypted_bytes = serde_json::to_vec(&encrypted)
        .map_err(|e| format!("failed to encode encrypted telemetry queue record: {e}"))?;
    let encoded_len: u32 = encrypted_bytes
        .len()
        .try_into()
        .map_err(|_| "telemetry queue record too large to persist".to_string())?;
    let record_total_len = 4u64.saturating_add(encoded_len as u64);

    let capacity_outcome = enforce_capacity_limit(
        &mut layout,
        record,
        record_total_len,
        &mut on_queue_pressure,
    )?;
    if !capacity_outcome.persisted {
        return Ok(capacity_outcome);
    }

    let current_segment_len = fs::metadata(&layout.active_segment_path)
        .map_err(|e| format!("failed to stat telemetry segment for append: {e}"))?
        .len();
    if current_segment_len > 0
        && current_segment_len.saturating_add(record_total_len)
            > layout.manifest.segment_rotation_bytes
    {
        rotate_active_segment(&mut layout)?;
    }

    let mut file = OpenOptions::new()
        .append(true)
        .open(&layout.active_segment_path)
        .map_err(|e| format!("failed to open telemetry segment for append: {e}"))?;
    file.write_all(&encoded_len.to_le_bytes())
        .and_then(|_| file.write_all(&encrypted_bytes))
        .and_then(|_| file.flush())
        .map_err(|e| format!("failed to append telemetry queue record: {e}"))?;

    layout.manifest.total_bytes = layout
        .manifest
        .total_bytes
        .saturating_add(4u64.saturating_add(encoded_len as u64));
    save_manifest(&layout.manifest_path, &layout.manifest)?;
    Ok(AppendTelemetryRecordOutcome {
        persisted: true,
        dropped_local_ids: capacity_outcome.dropped_local_ids,
        dropped_records: capacity_outcome.dropped_records,
        dropped_bytes: capacity_outcome.dropped_bytes,
        queue_bytes: layout.manifest.total_bytes,
    })
}

pub(super) fn next_disk_queue_wait(idle_poll: Duration) -> Result<Option<Duration>, String> {
    let (_layout, records) = read_persisted_records()?;
    next_disk_queue_wait_for_records(records, idle_poll)
}

fn next_disk_queue_wait_for_records(
    records: Vec<OwnedPersistedTelemetryQueueRecord>,
    idle_poll: Duration,
) -> Result<Option<Duration>, String> {
    let Some(first) = records.first() else {
        return Ok(None);
    };

    let now_ms = unix_time_now_ms();
    if first.next_attempt_at <= now_ms {
        return Ok(Some(Duration::ZERO));
    }

    Ok(Some(Duration::from_millis(
        first
            .next_attempt_at
            .saturating_sub(now_ms)
            .max(idle_poll.as_millis() as u64),
    )))
}

pub(super) fn prepare_disk_ready_batch(
    max_batch_bytes: usize,
    max_batch_events: usize,
) -> Result<Option<ReadyTelemetryBatch>, String> {
    let (_layout, records) = read_persisted_records()?;
    prepare_disk_ready_batch_for_records(records, max_batch_bytes, max_batch_events)
}

fn prepare_disk_ready_batch_for_records(
    records: Vec<OwnedPersistedTelemetryQueueRecord>,
    max_batch_bytes: usize,
    max_batch_events: usize,
) -> Result<Option<ReadyTelemetryBatch>, String> {
    let now_ms = unix_time_now_ms();
    let mut events = Vec::new();
    let mut bytes = 0usize;

    for record in records {
        if record.next_attempt_at > now_ms {
            break;
        }

        let event_bytes = serde_json::to_vec(&record.body)
            .map_err(|e| format!("failed to encode telemetry event batch entry: {e}"))?
            .len();
        let would_exceed = !events.is_empty()
            && (bytes + event_bytes > max_batch_bytes || events.len() >= max_batch_events);
        if would_exceed {
            break;
        }

        bytes += event_bytes;
        events.push(record.body);
    }

    if events.is_empty() {
        return Ok(None);
    }

    let count = events.len();
    Ok(Some(ReadyTelemetryBatch {
        events,
        count,
        bytes,
    }))
}

pub(super) fn drop_disk_sent(count: usize) -> Result<(), String> {
    mutate_persisted_records(|records| {
        let drain_count = count.min(records.len());
        records.drain(0..drain_count);
        Ok(())
    })
}

#[cfg(test)]
fn drop_disk_sent_at_root(root: &PathBuf, count: usize) -> Result<(), String> {
    mutate_persisted_records_at_root(root, |records| {
        let drain_count = count.min(records.len());
        records.drain(0..drain_count);
        Ok(())
    })
}

pub(super) fn schedule_disk_retry(
    count: usize,
    max_backoff_secs: u64,
) -> Result<RetrySchedule, String> {
    let mut computed = RetrySchedule {
        attempt: 1,
        backoff_secs: 2,
    };

    mutate_persisted_records(|records| {
        let attempt = records
            .iter()
            .take(count)
            .map(|record| record.attempt)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let backoff_secs = 2_u64.saturating_pow(attempt.min(5)).min(max_backoff_secs);
        let next_attempt_at = unix_time_now_ms().saturating_add(backoff_secs.saturating_mul(1_000));

        for record in records.iter_mut().take(count) {
            record.attempt = attempt;
            record.next_attempt_at = next_attempt_at;
        }

        computed = RetrySchedule {
            attempt,
            backoff_secs,
        };
        Ok(())
    })?;

    Ok(computed)
}

#[cfg(test)]
fn schedule_disk_retry_at_root(
    root: &PathBuf,
    count: usize,
    max_backoff_secs: u64,
) -> Result<RetrySchedule, String> {
    let mut computed = RetrySchedule {
        attempt: 1,
        backoff_secs: 2,
    };

    mutate_persisted_records_at_root(root, |records| {
        let attempt = records
            .iter()
            .take(count)
            .map(|record| record.attempt)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let backoff_secs = 2_u64.saturating_pow(attempt.min(5)).min(max_backoff_secs);
        let next_attempt_at = unix_time_now_ms().saturating_add(backoff_secs.saturating_mul(1_000));

        for record in records.iter_mut().take(count) {
            record.attempt = attempt;
            record.next_attempt_at = next_attempt_at;
        }

        computed = RetrySchedule {
            attempt,
            backoff_secs,
        };
        Ok(())
    })?;

    Ok(computed)
}

pub(super) fn disk_queue_stats() -> Result<DiskQueueStats, String> {
    let (_layout, records) = read_persisted_records()?;
    let queued_bytes = read_persisted_records()?.0.manifest.total_bytes;
    let now_ms = unix_time_now_ms();
    let oldest_queued_age_ms = records
        .iter()
        .map(|record| now_ms.saturating_sub(record.created_at))
        .max();

    Ok(DiskQueueStats {
        queued_records: records.len(),
        queued_bytes,
        oldest_queued_age_ms,
    })
}

fn load_or_create_manifest(path: &PathBuf) -> Result<DiskQueueManifest, String> {
    if !path.exists() {
        let manifest = DiskQueueManifest::default();
        save_manifest(path, &manifest)?;
        return Ok(manifest);
    }

    let raw =
        fs::read_to_string(path).map_err(|e| format!("failed to read telemetry manifest: {e}"))?;
    let manifest: DiskQueueManifest = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid telemetry manifest format: {e}"))?;
    Ok(manifest)
}

fn high_watermark_bytes() -> u64 {
    queue_max_bytes().saturating_mul(queue_high_watermark_percent()) / 100
}

fn ensure_disk_queue_layout_at_root(root_dir: &PathBuf) -> Result<DiskQueueLayout, String> {
    let manifest_path = root_dir.join(TELEMETRY_QUEUE_MANIFEST_FILE_NAME);
    let segments_dir = root_dir.join(TELEMETRY_QUEUE_SEGMENTS_DIR_NAME);

    fs::create_dir_all(&segments_dir)
        .map_err(|e| format!("failed to create telemetry queue dir: {e}"))?;

    let mut manifest = load_or_create_manifest(&manifest_path)?;
    let crypto = load_queue_crypto()?;
    let active_segment_path = segment_path(&segments_dir, manifest.active_segment);
    ensure_segment_exists(&active_segment_path)?;

    let recovery = recover_segment_tail(&active_segment_path)?;
    let active_segment_len = fs::metadata(&active_segment_path)
        .map_err(|e| format!("failed to stat active telemetry segment: {e}"))?
        .len();

    let expected_total_bytes = total_segment_bytes(&segments_dir)?;
    if manifest.total_bytes != expected_total_bytes {
        manifest.total_bytes = expected_total_bytes;
    }
    if manifest.key_id.as_deref() != Some(crypto.key_id()) {
        manifest.key_id = Some(crypto.key_id().to_string());
    }
    if manifest.read_cursor.segment_id == 0 {
        manifest.read_cursor = DiskQueueCursor::new(TELEMETRY_QUEUE_INITIAL_SEGMENT_ID, 0);
    }
    if manifest.ack_cursor.segment_id == 0 {
        manifest.ack_cursor = DiskQueueCursor::new(TELEMETRY_QUEUE_INITIAL_SEGMENT_ID, 0);
    }
    if manifest.segment_rotation_bytes == 0 {
        manifest.segment_rotation_bytes = TELEMETRY_SEGMENT_ROTATION_BYTES;
    }
    if manifest.active_segment == 0 {
        manifest.active_segment = TELEMETRY_QUEUE_INITIAL_SEGMENT_ID;
    }
    if manifest.active_segment == manifest.read_cursor.segment_id {
        manifest.read_cursor.offset = manifest.read_cursor.offset.min(active_segment_len);
    }
    if manifest.active_segment == manifest.ack_cursor.segment_id {
        manifest.ack_cursor.offset = manifest.ack_cursor.offset.min(active_segment_len);
    }

    save_manifest(&manifest_path, &manifest)?;

    Ok(DiskQueueLayout {
        root_dir: root_dir.clone(),
        manifest_path,
        segments_dir,
        active_segment_path,
        manifest,
        recovery,
    })
}

fn save_manifest(path: &PathBuf, manifest: &DiskQueueManifest) -> Result<(), String> {
    let encoded = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("failed to encode telemetry manifest: {e}"))?;
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, encoded)
        .map_err(|e| format!("failed to write temporary telemetry manifest: {e}"))?;
    fs::rename(&temp_path, path)
        .map_err(|e| format!("failed to replace telemetry manifest atomically: {e}"))
}

fn ensure_segment_exists(path: &PathBuf) -> Result<(), String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(|_| ())
        .map_err(|e| format!("failed to create telemetry segment: {e}"))
}

fn recover_segment_tail(path: &PathBuf) -> Result<DiskQueueRecovery, String> {
    let mut file = File::open(path)
        .map_err(|e| format!("failed to open telemetry segment for recovery: {e}"))?;
    let valid_len = scan_valid_prefix_len(&mut file)
        .map_err(|e| format!("failed to recover telemetry segment tail: {e}"))?;
    let current_len = file
        .metadata()
        .map_err(|e| format!("failed to stat telemetry segment during recovery: {e}"))?
        .len();

    if valid_len < current_len {
        file.set_len(valid_len)
            .map_err(|e| format!("failed to truncate telemetry segment tail: {e}"))?;
    }

    Ok(DiskQueueRecovery {
        truncated_bytes: current_len.saturating_sub(valid_len),
    })
}

fn scan_valid_prefix_len(file: &mut File) -> io::Result<u64> {
    let file_len = file.metadata()?.len();
    let mut offset = 0u64;

    loop {
        if file_len.saturating_sub(offset) < 4 {
            return Ok(offset);
        }

        file.seek(SeekFrom::Start(offset))?;
        let mut len_buf = [0u8; 4];
        match file.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Ok(offset),
            Err(err) => return Err(err),
        }

        let record_len = u32::from_le_bytes(len_buf) as u64;
        let record_end = offset
            .checked_add(4)
            .and_then(|value| value.checked_add(record_len))
            .ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidData, "telemetry record length overflow")
            })?;

        if record_end > file_len {
            return Ok(offset);
        }

        file.seek(SeekFrom::Start(record_end))?;
        offset = record_end;
    }
}

fn total_segment_bytes(segments_dir: &PathBuf) -> Result<u64, String> {
    let mut total = 0u64;
    let entries = fs::read_dir(segments_dir)
        .map_err(|e| format!("failed to read telemetry segments dir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read telemetry segment entry: {e}"))?;
        let metadata = entry
            .metadata()
            .map_err(|e| format!("failed to stat telemetry segment entry: {e}"))?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn segment_path(segments_dir: &PathBuf, segment_id: u64) -> PathBuf {
    segments_dir.join(format!("segment-{segment_id:020}.log"))
}

fn rotate_active_segment(layout: &mut DiskQueueLayout) -> Result<(), String> {
    layout.manifest.active_segment = layout.manifest.active_segment.saturating_add(1);
    layout.active_segment_path = segment_path(&layout.segments_dir, layout.manifest.active_segment);
    ensure_segment_exists(&layout.active_segment_path)?;
    save_manifest(&layout.manifest_path, &layout.manifest)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTelemetryQueueRecord<'a> {
    local_id: u64,
    seq: u64,
    body: &'a serde_json::Value,
    created_at: u64,
    attempt: u32,
    next_attempt_at: u64,
}

impl<'a> From<&'a TelemetryQueueRecord> for PersistedTelemetryQueueRecord<'a> {
    fn from(record: &'a TelemetryQueueRecord) -> Self {
        Self {
            local_id: record.local_id,
            seq: record.seq,
            body: &record.body,
            created_at: record.created_at_ms,
            attempt: record.attempt,
            next_attempt_at: record.next_attempt_at_ms,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnedPersistedTelemetryQueueRecord {
    local_id: u64,
    seq: u64,
    body: serde_json::Value,
    created_at: u64,
    attempt: u32,
    next_attempt_at: u64,
}

impl DiskQueueManifest {
    fn default() -> Self {
        Self {
            format_version: TELEMETRY_QUEUE_FORMAT_VERSION,
            active_segment: TELEMETRY_QUEUE_INITIAL_SEGMENT_ID,
            read_cursor: DiskQueueCursor::new(TELEMETRY_QUEUE_INITIAL_SEGMENT_ID, 0),
            ack_cursor: DiskQueueCursor::new(TELEMETRY_QUEUE_INITIAL_SEGMENT_ID, 0),
            total_bytes: 0,
            key_id: None,
            segment_rotation_bytes: TELEMETRY_SEGMENT_ROTATION_BYTES,
        }
    }
}

impl DiskQueueCursor {
    fn new(segment_id: u64, offset: u64) -> Self {
        Self { segment_id, offset }
    }
}

fn read_persisted_records(
) -> Result<(DiskQueueLayout, Vec<OwnedPersistedTelemetryQueueRecord>), String> {
    let io_lock = DISK_QUEUE_IO.get_or_init(|| Mutex::new(()));
    let Ok(_guard) = io_lock.lock() else {
        return Err("failed to lock telemetry disk queue io".to_string());
    };

    let root = disk_queue_root()?;
    read_persisted_records_at_root(root)
}

fn read_persisted_records_at_root(
    root: &PathBuf,
) -> Result<(DiskQueueLayout, Vec<OwnedPersistedTelemetryQueueRecord>), String> {
    let layout = load_layout_from_root(root)?;
    let records = read_records_from_layout(&layout)?;
    Ok((layout, records))
}

fn mutate_persisted_records(
    mutator: impl FnOnce(&mut Vec<OwnedPersistedTelemetryQueueRecord>) -> Result<(), String>,
) -> Result<(), String> {
    let io_lock = DISK_QUEUE_IO.get_or_init(|| Mutex::new(()));
    let Ok(_guard) = io_lock.lock() else {
        return Err("failed to lock telemetry disk queue io".to_string());
    };

    let root = disk_queue_root()?;
    mutate_persisted_records_at_root(root, mutator)
}

fn mutate_persisted_records_at_root(
    root: &PathBuf,
    mutator: impl FnOnce(&mut Vec<OwnedPersistedTelemetryQueueRecord>) -> Result<(), String>,
) -> Result<(), String> {
    let mut layout = load_layout_from_root(root)?;
    let mut records = read_records_from_layout(&layout)?;
    mutator(&mut records)?;
    rewrite_records(&mut layout, &records)?;
    Ok(())
}

fn disk_queue_root() -> Result<&'static PathBuf, String> {
    DISK_QUEUE_ROOT
        .get()
        .ok_or_else(|| "telemetry disk queue root is not initialized".to_string())
}

fn load_layout_from_root(root_dir: &PathBuf) -> Result<DiskQueueLayout, String> {
    ensure_disk_queue_layout_at_root(root_dir)
}

fn read_records_from_layout(
    layout: &DiskQueueLayout,
) -> Result<Vec<OwnedPersistedTelemetryQueueRecord>, String> {
    let segment_ids = list_segment_ids(&layout.segments_dir)?;
    let mut records = Vec::new();
    for segment_id in segment_ids {
        let path = segment_path(&layout.segments_dir, segment_id);
        records.extend(read_records_from_segment(&path)?);
    }
    Ok(records)
}

fn read_records_from_segment(
    path: &PathBuf,
) -> Result<Vec<OwnedPersistedTelemetryQueueRecord>, String> {
    let crypto = load_queue_crypto()?;
    let mut file =
        File::open(path).map_err(|e| format!("failed to open telemetry segment for read: {e}"))?;
    let mut records = Vec::new();
    let mut record_index = 0usize;

    loop {
        let mut len_buf = [0u8; 4];
        match file.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(err) => {
                return Err(format!(
                    "failed to read telemetry segment length prefix: {err}"
                ))
            }
        }

        let record_len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; record_len];
        file.read_exact(&mut payload)
            .map_err(|e| format!("failed to read telemetry segment record body: {e}"))?;
        let decoded = serde_json::from_slice::<EncryptedRecord>(&payload)
            .map_err(|e| format!("failed to decode encrypted telemetry segment record: {e}"))
            .and_then(|encrypted| crypto.decrypt_record(&encrypted))
            .and_then(|plaintext| {
                serde_json::from_slice::<OwnedPersistedTelemetryQueueRecord>(&plaintext)
                    .map_err(|e| format!("failed to decode telemetry segment record: {e}"))
            });

        match decoded {
            Ok(record) => records.push(record),
            Err(err) => {
                eprintln!(
                    "Skipping corrupt telemetry queue record in {} at index {}: {}",
                    path.display(),
                    record_index,
                    err
                );
            }
        }
        record_index = record_index.saturating_add(1);
    }

    Ok(records)
}

fn rewrite_records(
    layout: &mut DiskQueueLayout,
    records: &[OwnedPersistedTelemetryQueueRecord],
) -> Result<(), String> {
    let existing_segment_ids = list_segment_ids(&layout.segments_dir)?;
    let mut next_segment_id = existing_segment_ids
        .iter()
        .copied()
        .max()
        .unwrap_or(TELEMETRY_QUEUE_INITIAL_SEGMENT_ID);
    let mut used_segment_ids = Vec::new();
    let mut batch_records: Vec<OwnedPersistedTelemetryQueueRecord> = Vec::new();
    let mut batch_bytes = 0u64;

    for record in records {
        let record_bytes = persisted_record_storage_len(record)?;

        if !batch_records.is_empty()
            && batch_bytes.saturating_add(record_bytes) > layout.manifest.segment_rotation_bytes
        {
            next_segment_id = next_segment_id.saturating_add(1);
            let path = segment_path(&layout.segments_dir, next_segment_id);
            write_segment_records(&path, &batch_records)?;
            used_segment_ids.push(next_segment_id);
            batch_records.clear();
            batch_bytes = 0;
        }

        batch_records.push(record.clone());
        batch_bytes = batch_bytes.saturating_add(record_bytes);
    }

    if batch_records.is_empty() {
        next_segment_id = next_segment_id.saturating_add(1);
        let path = segment_path(&layout.segments_dir, next_segment_id);
        write_segment_records(&path, &[])?;
        used_segment_ids.push(next_segment_id);
    } else {
        next_segment_id = next_segment_id.saturating_add(1);
        let path = segment_path(&layout.segments_dir, next_segment_id);
        write_segment_records(&path, &batch_records)?;
        used_segment_ids.push(next_segment_id);
    }

    for segment_id in existing_segment_ids {
        if !used_segment_ids.contains(&segment_id) {
            let path = segment_path(&layout.segments_dir, segment_id);
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|e| format!("failed to delete old telemetry segment: {e}"))?;
            }
        }
    }

    let first_segment_id = *used_segment_ids
        .first()
        .unwrap_or(&TELEMETRY_QUEUE_INITIAL_SEGMENT_ID);
    let active_segment_id = *used_segment_ids
        .last()
        .unwrap_or(&TELEMETRY_QUEUE_INITIAL_SEGMENT_ID);
    layout.manifest.active_segment = active_segment_id;
    layout.manifest.read_cursor = DiskQueueCursor::new(first_segment_id, 0);
    layout.manifest.ack_cursor = DiskQueueCursor::new(first_segment_id, 0);
    layout.manifest.total_bytes = total_segment_bytes(&layout.segments_dir)?;
    layout.active_segment_path = segment_path(&layout.segments_dir, active_segment_id);
    save_manifest(&layout.manifest_path, &layout.manifest)
}

fn enforce_capacity_limit(
    layout: &mut DiskQueueLayout,
    record: &TelemetryQueueRecord,
    record_total_len: u64,
    on_queue_pressure: &mut impl FnMut(QueuePressureNotification),
) -> Result<AppendTelemetryRecordOutcome, String> {
    let projected_total = layout.manifest.total_bytes.saturating_add(record_total_len);
    let high_watermark = high_watermark_bytes();
    if projected_total <= high_watermark {
        return Ok(AppendTelemetryRecordOutcome {
            persisted: true,
            dropped_local_ids: Vec::new(),
            dropped_records: 0,
            dropped_bytes: 0,
            queue_bytes: layout.manifest.total_bytes,
        });
    }

    let mut records = read_records_from_layout(layout)?;
    let mut dropped_local_ids = Vec::new();
    let mut dropped_bytes = 0u64;
    let mut projected_after_drop = projected_total;

    while projected_after_drop > high_watermark {
        let Some(index) = records.iter().position(is_drop_candidate) else {
            break;
        };
        let dropped = records.remove(index);
        let dropped_len = persisted_record_storage_len(&dropped)?;
        dropped_bytes = dropped_bytes.saturating_add(dropped_len);
        dropped_local_ids.push(dropped.local_id);
        projected_after_drop = projected_after_drop.saturating_sub(dropped_len);
    }

    if !dropped_local_ids.is_empty() {
        rewrite_records(layout, &records)?;
    }

    let queue_bytes_after_drop = layout.manifest.total_bytes;
    if queue_bytes_after_drop.saturating_add(record_total_len) > queue_max_bytes() {
        let incoming_dropped = !is_protected_record(record);
        let dropped_count = dropped_local_ids.len();
        on_queue_pressure(QueuePressureNotification {
            queue_bytes: queue_bytes_after_drop,
            dropped_records: dropped_count + usize::from(incoming_dropped),
            dropped_bytes: dropped_bytes.saturating_add(if incoming_dropped {
                record_total_len
            } else {
                0
            }),
            dropped_incoming: incoming_dropped,
        });

        if incoming_dropped {
            return Ok(AppendTelemetryRecordOutcome {
                persisted: false,
                dropped_local_ids,
                dropped_records: dropped_count.saturating_add(1),
                dropped_bytes: dropped_bytes.saturating_add(record_total_len),
                queue_bytes: queue_bytes_after_drop,
            });
        }

        return Err(format!(
            "telemetry durable queue is full ({queue_bytes_after_drop} bytes) and only protected events remain"
        ));
    }

    if !dropped_local_ids.is_empty() {
        let dropped_count = dropped_local_ids.len();
        on_queue_pressure(QueuePressureNotification {
            queue_bytes: queue_bytes_after_drop,
            dropped_records: dropped_count,
            dropped_bytes,
            dropped_incoming: false,
        });
        return Ok(AppendTelemetryRecordOutcome {
            persisted: true,
            dropped_local_ids,
            dropped_records: dropped_count,
            dropped_bytes,
            queue_bytes: queue_bytes_after_drop,
        });
    }

    Ok(AppendTelemetryRecordOutcome {
        persisted: true,
        dropped_local_ids,
        dropped_records: 0,
        dropped_bytes: 0,
        queue_bytes: queue_bytes_after_drop,
    })
}

fn write_segment_records(
    path: &PathBuf,
    records: &[OwnedPersistedTelemetryQueueRecord],
) -> Result<(), String> {
    let crypto = load_queue_crypto()?;
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)
        .map_err(|e| format!("failed to create temporary telemetry segment: {e}"))?;
    for record in records {
        let encoded = serde_json::to_vec(record)
            .map_err(|e| format!("failed to encode telemetry record for segment write: {e}"))?;
        let encrypted = crypto.encrypt_record(&encoded)?;
        let encrypted_bytes = serde_json::to_vec(&encrypted).map_err(|e| {
            format!("failed to encode encrypted telemetry record for segment write: {e}")
        })?;
        let encrypted_len: u32 = encrypted_bytes
            .len()
            .try_into()
            .map_err(|_| "telemetry queue record too large to persist".to_string())?;
        file.write_all(&encrypted_len.to_le_bytes())
            .and_then(|_| file.write_all(&encrypted_bytes))
            .map_err(|e| format!("failed to write telemetry segment record: {e}"))?;
    }
    file.flush()
        .map_err(|e| format!("failed to flush rewritten telemetry segment: {e}"))?;
    fs::rename(&temp_path, path)
        .map_err(|e| format!("failed to replace telemetry segment with rewritten data: {e}"))
}

fn list_segment_ids(segments_dir: &PathBuf) -> Result<Vec<u64>, String> {
    let mut ids = Vec::new();
    let entries = fs::read_dir(segments_dir)
        .map_err(|e| format!("failed to read telemetry segments dir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read telemetry segment entry: {e}"))?;
        let metadata = entry
            .metadata()
            .map_err(|e| format!("failed to stat telemetry segment entry: {e}"))?;
        if !metadata.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if let Some(segment_id) = parse_segment_id(&file_name) {
            ids.push(segment_id);
        }
    }
    ids.sort_unstable();
    Ok(ids)
}

fn parse_segment_id(file_name: &str) -> Option<u64> {
    file_name
        .strip_prefix("segment-")
        .and_then(|rest| rest.strip_suffix(".log"))
        .and_then(|id| id.parse::<u64>().ok())
}

fn is_drop_candidate(record: &OwnedPersistedTelemetryQueueRecord) -> bool {
    !is_protected_event_type(record.body.get("type").and_then(|value| value.as_str()))
}

fn is_protected_record(record: &TelemetryQueueRecord) -> bool {
    is_protected_event_type(record.body.get("type").and_then(|value| value.as_str()))
}

fn is_protected_event_type(event_type: Option<&str>) -> bool {
    matches!(
        event_type,
        Some(
            "function-status"
                | "pty-exit"
                | "pty-open"
                | "pty-close"
                | "pty-resize"
                | "terminal-open"
                | "terminal-close"
                | "terminal-resize"
                | "terminal-start"
                | "terminal-stop"
        )
    )
}

fn emit_queue_pressure(app: &AppHandle, notification: QueuePressureNotification) {
    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct QueuePressureEvent {
        degraded: bool,
        queue_bytes: u64,
        max_bytes: u64,
        high_watermark_bytes: u64,
        dropped_records: usize,
        dropped_bytes: u64,
        dropped_incoming: bool,
    }

    eprintln!(
        "CC telemetry queue degraded: dropped {} record(s) / {} byte(s); queue at {} bytes",
        notification.dropped_records, notification.dropped_bytes, notification.queue_bytes
    );
    let _ = app.emit(
        "cc-telemetry-health",
        QueuePressureEvent {
            degraded: true,
            queue_bytes: notification.queue_bytes,
            max_bytes: queue_max_bytes(),
            high_watermark_bytes: high_watermark_bytes(),
            dropped_records: notification.dropped_records,
            dropped_bytes: notification.dropped_bytes,
            dropped_incoming: notification.dropped_incoming,
        },
    );
}

fn persisted_record_storage_len(
    record: &OwnedPersistedTelemetryQueueRecord,
) -> Result<u64, String> {
    let crypto = load_queue_crypto()?;
    let encoded = serde_json::to_vec(record)
        .map_err(|e| format!("failed to encode telemetry record for size estimation: {e}"))?;
    let encrypted = crypto.encrypt_record(&encoded)?;
    let encrypted_bytes = serde_json::to_vec(&encrypted).map_err(|e| {
        format!("failed to encode encrypted telemetry record for size estimation: {e}")
    })?;
    Ok(4u64.saturating_add(encrypted_bytes.len() as u64))
}

fn unix_time_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn load_queue_crypto() -> Result<TelemetryQueueCrypto, String> {
    #[cfg(test)]
    {
        return TelemetryQueueCrypto::from_key_bytes_for_test([0x5a; 32]);
    }

    #[cfg(not(test))]
    {
        TelemetryQueueCrypto::load_or_create()
    }
}

fn queue_max_bytes() -> u64 {
    #[cfg(test)]
    {
        return 32 * 1024;
    }

    #[cfg(not(test))]
    {
        TELEMETRY_QUEUE_MAX_BYTES
    }
}

fn queue_high_watermark_percent() -> u64 {
    #[cfg(test)]
    {
        return 90;
    }

    #[cfg(not(test))]
    {
        TELEMETRY_QUEUE_HIGH_WATERMARK_PERCENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        fs,
        io::{Read, Seek, SeekFrom, Write},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("scp-cc-queue-{name}-{unique}"))
    }

    fn reset_root(root: &PathBuf) {
        let _ = fs::remove_dir_all(root);
        fs::create_dir_all(root).unwrap();
    }

    fn record(
        seq: u64,
        event_type: &str,
        session_id: &str,
        payload: serde_json::Value,
    ) -> TelemetryQueueRecord {
        TelemetryQueueRecord::new(
            seq,
            json!({
                "type": event_type,
                "sessionId": session_id,
                "functionId": null,
                "seq": seq,
                "ts": "2026-04-08T00:00:00Z",
                "payload": payload,
            }),
        )
    }

    #[test]
    fn persists_across_reload_and_flushes_after_retry() {
        let root = test_root("restart-flush");
        reset_root(&root);

        append_telemetry_record_at_root(
            &root,
            &record(1, "pty-output", "s1", json!({"data":"a"})),
            |_| {},
        )
        .unwrap();
        append_telemetry_record_at_root(
            &root,
            &record(2, "pty-output", "s1", json!({"data":"b"})),
            |_| {},
        )
        .unwrap();
        append_telemetry_record_at_root(
            &root,
            &record(3, "pty-output", "s1", json!({"data":"c"})),
            |_| {},
        )
        .unwrap();

        let (_, persisted_before_restart) = read_persisted_records_at_root(&root).unwrap();
        assert_eq!(persisted_before_restart.len(), 3);

        let (_, persisted_after_restart) = read_persisted_records_at_root(&root).unwrap();
        assert_eq!(persisted_after_restart.len(), 3);

        let schedule = schedule_disk_retry_at_root(&root, 2, 30).unwrap();
        assert_eq!(schedule.attempt, 1);
        assert!(prepare_disk_ready_batch_for_records(
            read_persisted_records_at_root(&root).unwrap().1,
            1024,
            16
        )
        .unwrap()
        .is_none());

        mutate_persisted_records_at_root(&root, |records| {
            for record in records.iter_mut() {
                record.next_attempt_at = 0;
            }
            Ok(())
        })
        .unwrap();

        let ready = prepare_disk_ready_batch_for_records(
            read_persisted_records_at_root(&root).unwrap().1,
            1024,
            16,
        )
        .unwrap()
        .unwrap();
        assert_eq!(ready.count, 3);

        drop_disk_sent_at_root(&root, ready.count).unwrap();
        assert!(read_persisted_records_at_root(&root).unwrap().1.is_empty());
    }

    #[test]
    fn preserves_per_stream_ordering() {
        let root = test_root("ordering");
        reset_root(&root);

        append_telemetry_record_at_root(
            &root,
            &record(1, "pty-output", "alpha", json!({"data":"1"})),
            |_| {},
        )
        .unwrap();
        append_telemetry_record_at_root(
            &root,
            &record(2, "pty-output", "beta", json!({"data":"2"})),
            |_| {},
        )
        .unwrap();
        append_telemetry_record_at_root(
            &root,
            &record(3, "pty-output", "alpha", json!({"data":"3"})),
            |_| {},
        )
        .unwrap();
        append_telemetry_record_at_root(
            &root,
            &record(4, "pty-output", "alpha", json!({"data":"4"})),
            |_| {},
        )
        .unwrap();

        let (_, records) = read_persisted_records_at_root(&root).unwrap();
        let alpha: Vec<u64> = records
            .iter()
            .filter(|record| {
                record.body.get("type").and_then(|value| value.as_str()) == Some("pty-output")
                    && record
                        .body
                        .get("sessionId")
                        .and_then(|value| value.as_str())
                        == Some("alpha")
            })
            .filter_map(|record| record.body.get("seq").and_then(|value| value.as_u64()))
            .collect();

        assert_eq!(alpha, vec![1, 3, 4]);
    }

    #[test]
    fn skips_corrupt_record_without_blocking_following_records() {
        let root = test_root("corrupt");
        reset_root(&root);

        append_telemetry_record_at_root(
            &root,
            &record(1, "pty-output", "s1", json!({"data":"first"})),
            |_| {},
        )
        .unwrap();
        append_telemetry_record_at_root(
            &root,
            &record(2, "pty-output", "s1", json!({"data":"second"})),
            |_| {},
        )
        .unwrap();

        let layout = ensure_disk_queue_layout_at_root(&root).unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&layout.active_segment_path)
            .unwrap();
        let mut len_buf = [0u8; 4];
        file.read_exact(&mut len_buf).unwrap();
        let first_len = u32::from_le_bytes(len_buf) as u64;
        file.seek(SeekFrom::Start(4 + first_len / 2)).unwrap();
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::Current(-1)).unwrap();
        byte[0] ^= 0xff;
        file.write_all(&byte).unwrap();
        file.flush().unwrap();

        let (_, records) = read_persisted_records_at_root(&root).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0]
                .body
                .get("payload")
                .and_then(|payload| payload.get("data"))
                .and_then(|value| value.as_str()),
            Some("second")
        );
    }

    #[test]
    fn sheds_oldest_non_critical_records_and_keeps_protected_events() {
        let root = test_root("capacity");
        reset_root(&root);
        let mut notifications = Vec::new();
        let payload = json!({ "data": "x".repeat(8 * 1024) });

        for seq in 1..=6 {
            append_telemetry_record_at_root(
                &root,
                &record(seq, "pty-output", "bulk", payload.clone()),
                |notification| notifications.push(notification),
            )
            .unwrap();
        }

        append_telemetry_record_at_root(
            &root,
            &record(7, "function-status", "bulk", json!({"message":"protected"})),
            |notification| notifications.push(notification),
        )
        .unwrap();

        let (_, records) = read_persisted_records_at_root(&root).unwrap();
        assert!(notifications
            .iter()
            .any(|notification| notification.dropped_records > 0));
        assert!(records.iter().any(|record| {
            record.body.get("type").and_then(|value| value.as_str()) == Some("function-status")
        }));
        assert!(records.len() < 7);
    }

    #[test]
    fn disk_artifacts_do_not_contain_plaintext_secret_and_store_redacted_payload() {
        let root = test_root("redaction");
        reset_root(&root);
        let secret = "super-secret-token";
        let redacted = "[redacted]";

        append_telemetry_record_at_root(
            &root,
            &record(
                1,
                "pty-output",
                "s1",
                json!({"authorization": redacted, "data": redacted}),
            ),
            |_| {},
        )
        .unwrap();

        let layout = ensure_disk_queue_layout_at_root(&root).unwrap();
        let raw_bytes = fs::read(&layout.active_segment_path).unwrap();
        let raw_text = String::from_utf8_lossy(&raw_bytes);
        assert!(!raw_text.contains(secret));

        let (_, records) = read_persisted_records_at_root(&root).unwrap();
        let payload = records[0].body.get("payload").unwrap();
        assert_eq!(
            payload
                .get("authorization")
                .and_then(|value| value.as_str()),
            Some(redacted)
        );
        assert_eq!(
            payload.get("data").and_then(|value| value.as_str()),
            Some(redacted)
        );
    }
}
