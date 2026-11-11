use scp_core::{
    mcp_dto::{McpScpTransferGetArgs, McpScpTransferGetResult, McpScpTransferListResult},
    scp_transfer::ScpTransferState,
};
#[path = "contracts/mcp_transfer_args.rs"]
mod arguments;
#[path = "contracts/mcp_transfers.rs"]
mod contract;

#[test]
fn transfer_results_are_the_shared_models() {
    let _: fn(scp_core::scp_transfer::ScpTransferEntry) -> McpScpTransferGetResult =
        std::convert::identity;
    let _: fn(Vec<scp_core::scp_transfer::ScpTransferEntry>) -> McpScpTransferListResult =
        std::convert::identity;
}
