//! Remote browser wire values; hosts retain sessions, SFTP execution and path handling.

use serde::Serialize;

/// Returned browser handle and root only; this value owns no live SSH/SFTP session.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowseSession {
    pub browser_id: String,
    pub root_path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum RemotePathKind {
    File,
    Directory,
    Symlink,
    Other,
    Missing,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowseEntry {
    pub name: String,
    pub path: String,
    pub kind: RemotePathKind,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowseNodeInfo {
    pub path: String,
    pub kind: RemotePathKind,
    pub exists: bool,
}
