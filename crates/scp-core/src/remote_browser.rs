//! Remote browser wire values; hosts retain sessions, SFTP execution and path handling.

use serde::Serialize;

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
