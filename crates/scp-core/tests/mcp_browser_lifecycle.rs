use scp_core::{
    mcp_dto::{
        McpRemoteBrowserCloseArgs, McpRemoteBrowserCloseResult, McpRemoteBrowserOpenArgs,
        McpRemoteBrowserOpenResult,
    },
    remote_browser::RemoteBrowseSession,
};
#[path = "contracts/mcp_browser_lifecycle.rs"]
mod arguments_and_close;
#[path = "contracts/mcp_browser_session.rs"]
mod session;

#[test]
fn open_result_is_the_shared_session_wire_value() {
    let result: McpRemoteBrowserOpenResult = RemoteBrowseSession {
        browser_id: "host-allocated".into(),
        root_path: "/home".into(),
    };
    assert_eq!(result.browser_id, "host-allocated");
}
