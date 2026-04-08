use std::{
    fs::{self, File, OpenOptions},
    io::{self, ErrorKind, Read, Seek, SeekFrom},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const TELEMETRY_QUEUE_DIR_NAME: &str = "cc-telemetry-queue";
const TELEMETRY_QUEUE_SEGMENTS_DIR_NAME: &str = "segments";
const TELEMETRY_QUEUE_MANIFEST_FILE_NAME: &str = "manifest.json";
const TELEMETRY_QUEUE_FORMAT_VERSION: u32 = 1;
const TELEMETRY_QUEUE_INITIAL_SEGMENT_ID: u64 = 1;
pub(super) const TELEMETRY_SEGMENT_ROTATION_BYTES: u64 = 4 * 1024 * 1024;

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

pub(super) fn ensure_disk_queue_layout(app: &AppHandle) -> Result<DiskQueueLayout, String> {
    let root_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("failed to resolve app config dir: {e}"))?
        .join(TELEMETRY_QUEUE_DIR_NAME);
    let manifest_path = root_dir.join(TELEMETRY_QUEUE_MANIFEST_FILE_NAME);
    let segments_dir = root_dir.join(TELEMETRY_QUEUE_SEGMENTS_DIR_NAME);

    fs::create_dir_all(&segments_dir)
        .map_err(|e| format!("failed to create telemetry queue dir: {e}"))?;

    let mut manifest = load_or_create_manifest(&manifest_path)?;
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
        root_dir,
        manifest_path,
        segments_dir,
        active_segment_path,
        manifest,
        recovery,
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

fn save_manifest(path: &PathBuf, manifest: &DiskQueueManifest) -> Result<(), String> {
    let encoded = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("failed to encode telemetry manifest: {e}"))?;
    fs::write(path, encoded).map_err(|e| format!("failed to write telemetry manifest: {e}"))
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
