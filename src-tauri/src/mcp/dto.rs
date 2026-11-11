use serde::Serialize;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub enabled: bool,
    pub active: bool,
    pub transport: String,
    pub bind_address: String,
    pub port: u16,
    pub implementation_stage: String,
    pub last_error: Option<String>,
    pub last_started_at_ms: Option<u64>,
}

pub use scp_core::mcp_catalog::McpToolDescriptor;

pub use scp_core::mcp_resources::McpResourceDescriptor;

pub use scp_core::mcp_dto::{
    McpAppOverviewResult, McpBoundaryGetStatusResult, McpConfigGetPathsResult,
    McpFunctionStatusGetArgs, McpFunctionStatusGetResult, McpFunctionStatusListResult,
    McpOpenVpnGetStatusResult, McpScpTransferGetArgs, McpScpTransferGetResult,
    McpScpTransferListResult, McpServerGetArgs, McpServerGetResult, McpServersListResult,
    McpToolCallEnvelope,
};

#[cfg(test)]
mod envelope_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_envelope.rs"
    ));
}

#[cfg(test)]
mod shared_envelope_identity {
    #[test]
    fn desktop_public_paths_accept_the_core_envelope_without_conversion() {
        let shared = scp_core::mcp_dto::McpToolCallEnvelope {
            ok: true,
            message: String::new(),
            data: serde_json::Value::Null,
            warnings: Vec::new(),
            timestamp: 0,
        };
        let dto: super::McpToolCallEnvelope = shared;
        let public: crate::mcp::McpToolCallEnvelope = dto;
        let _: scp_core::mcp_dto::McpToolCallEnvelope = public;
    }
}

#[cfg(test)]
mod function_status_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_function_status.rs"
    ));
}

#[cfg(test)]
mod shared_function_status_identity {
    #[test]
    fn desktop_paths_accept_shared_arguments_and_runtime_results_without_conversion() {
        let input = serde_json::json!({"functionId":"boundary-auth"});
        let args = scp_core::mcp_dto::McpFunctionStatusGetArgs::from_args(&input).unwrap();
        let dto: super::McpFunctionStatusGetArgs<'_> = args;
        let _: crate::mcp::McpFunctionStatusGetArgs<'_> = dto;
        let runtime = crate::models::FunctionStatus {
            function_id: String::new(),
            state: String::new(),
            message: String::new(),
            boundary_token_source: None,
            session_id: None,
            updated_at_ms: 0,
        };
        let dto: super::McpFunctionStatusGetResult = runtime;
        let public: crate::mcp::McpFunctionStatusGetResult = dto;
        let shared: scp_core::mcp_dto::McpFunctionStatusGetResult = public;
        let dto: super::McpFunctionStatusListResult = vec![shared];
        let public: crate::mcp::McpFunctionStatusListResult = dto;
        let _: scp_core::mcp_dto::McpFunctionStatusListResult = public;
    }
}

#[cfg(test)]
mod app_overview_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_app_overview.rs"
    ));
}

#[cfg(test)]
mod shared_app_overview_identity {
    #[test]
    fn desktop_public_paths_accept_the_core_overview_without_conversion() {
        let _: fn(scp_core::mcp_dto::McpAppOverviewResult) -> crate::mcp::McpAppOverviewResult =
            std::convert::identity::<super::McpAppOverviewResult>;
    }
}

#[cfg(test)]
mod service_status_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_service_status.rs"
    ));
}

#[cfg(test)]
mod shared_service_status_identity {
    #[test]
    fn desktop_public_paths_accept_core_service_status_results_without_conversion() {
        let _: fn(
            scp_core::mcp_dto::McpOpenVpnGetStatusResult,
        ) -> crate::mcp::McpOpenVpnGetStatusResult =
            std::convert::identity::<super::McpOpenVpnGetStatusResult>;
        let _: fn(
            scp_core::mcp_dto::McpBoundaryGetStatusResult,
        ) -> crate::mcp::McpBoundaryGetStatusResult =
            std::convert::identity::<super::McpBoundaryGetStatusResult>;
    }
}

#[cfg(test)]
mod servers_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_servers.rs"
    ));
}

#[cfg(test)]
mod shared_servers_identity {
    #[test]
    fn desktop_paths_accept_core_server_contracts_without_conversion() {
        let input = serde_json::json!({"serverId":"s"});
        let shared = scp_core::mcp_dto::McpServerGetArgs::from_args(&input).unwrap();
        let dto: super::McpServerGetArgs<'_> = shared;
        let _: crate::mcp::McpServerGetArgs<'_> = dto;
        let _: fn(crate::server_mgmt::Server) -> crate::mcp::McpServerGetResult =
            std::convert::identity::<super::McpServerGetResult>;
        let _: fn(Vec<crate::server_mgmt::Server>) -> crate::mcp::McpServersListResult =
            std::convert::identity::<super::McpServersListResult>;
    }
}

#[cfg(test)]
mod config_paths_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_config_paths.rs"
    ));
}

#[cfg(test)]
mod shared_config_paths_identity {
    #[test]
    fn desktop_public_paths_accept_the_shared_configuration_paths_without_conversion() {
        let expose: fn(crate::services::ConfigPaths) -> super::McpConfigGetPathsResult =
            |paths: scp_core::config::ConfigPaths| -> crate::mcp::McpConfigGetPathsResult { paths };
        let result = expose(scp_core::config::ConfigPaths {
            app_config_path: "config.json".to_owned(),
        });
        assert_eq!(result.app_config_path, "config.json");
    }
}

#[cfg(test)]
mod transfers_contract {
    use super::{McpScpTransferGetResult, McpScpTransferListResult};
    use crate::models::ScpTransferState;
    mod wire {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/scp-core/tests/contracts/mcp_transfers.rs"
        ));
    }
}
#[cfg(test)]
mod transfer_arguments_contract {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/scp-core/tests/contracts/mcp_transfer_args.rs"
    ));
}
#[cfg(test)]
mod shared_transfer_identity {
    #[test]
    fn desktop_model_and_mcp_paths_accept_core_types() {
        let _: fn(scp_core::scp_transfer::ScpTransferEntry) -> crate::models::ScpTransferEntry =
            std::convert::identity;
        let _: fn(scp_core::scp_transfer::ScpTransferState) -> crate::models::ScpTransferState =
            std::convert::identity;
        let _: fn(crate::models::ScpTransferEntry) -> crate::mcp::McpScpTransferGetResult =
            std::convert::identity::<super::McpScpTransferGetResult>;
        let _: fn(Vec<crate::models::ScpTransferEntry>) -> crate::mcp::McpScpTransferListResult =
            std::convert::identity::<super::McpScpTransferListResult>;
        let input = serde_json::json!({"transferId":"id"});
        let shared = scp_core::mcp_dto::McpScpTransferGetArgs::from_args(&input).unwrap();
        let dto: super::McpScpTransferGetArgs<'_> = shared;
        let public: crate::mcp::McpScpTransferGetArgs<'_> = dto;
        assert_eq!(public.transfer_id, "id");
    }
}
