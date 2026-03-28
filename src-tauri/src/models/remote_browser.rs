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
pub struct RemoteBrowseSession {
    pub browser_id: String,
    pub root_path: String,
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

#[derive(Clone)]
pub struct SshEndpoint {
    pub user: String,
    pub host: String,
    pub port: u16,
}
