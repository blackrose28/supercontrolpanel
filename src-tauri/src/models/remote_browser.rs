use serde::Serialize;

pub use scp_core::remote_browser::{RemoteBrowseEntry, RemoteBrowseNodeInfo, RemotePathKind};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowseSession {
    pub browser_id: String,
    pub root_path: String,
}

#[derive(Clone)]
pub struct SshEndpoint {
    pub user: String,
    pub host: String,
    pub port: u16,
}

#[cfg(test)]
mod read_contract {
    use super::{RemoteBrowseEntry, RemotePathKind};
    use crate::mcp::{McpRemoteBrowserListChildrenResult, McpRemoteBrowserStatPathResult};
    mod wire {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/scp-core/tests/contracts/mcp_browser_reads.rs"
        ));
    }
}
