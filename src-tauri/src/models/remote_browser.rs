pub use scp_core::remote_browser::{
    RemoteBrowseEntry, RemoteBrowseNodeInfo, RemoteBrowseSession, RemotePathKind,
};

#[derive(Clone)]
pub struct SshEndpoint {
    pub user: String,
    pub host: String,
    pub port: u16,
}

#[cfg(test)]
mod session_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_browser_session.rs"
    ));
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
