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
    McpAppOverviewResult, McpBoundaryGetStatusResult, McpFunctionStatusGetArgs,
    McpFunctionStatusGetResult, McpFunctionStatusListResult, McpOpenVpnGetStatusResult,
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
