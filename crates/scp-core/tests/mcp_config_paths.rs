use scp_core::mcp_dto::McpConfigGetPathsResult;
#[path = "contracts/mcp_config_paths.rs"]
mod contract;

#[test]
fn result_is_the_shared_configuration_paths_model() {
    let _: fn(scp_core::config::ConfigPaths) -> McpConfigGetPathsResult = std::convert::identity;
}
