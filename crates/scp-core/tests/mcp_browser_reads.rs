use scp_core::{
    mcp_dto::{
        McpRemoteBrowserListChildrenResult, McpRemoteBrowserReadArgs,
        McpRemoteBrowserStatPathResult,
    },
    remote_browser::{RemoteBrowseEntry, RemotePathKind},
};
#[path = "contracts/mcp_browser_read_args.rs"]
mod arguments;
#[path = "contracts/mcp_browser_reads.rs"]
mod results;
