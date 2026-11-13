//! Synthetic subprocess only; never reads real configuration or credentials.
//! Stdout belongs exclusively to the SDK. No production scpctl entry point.
use rmcp::ServiceExt;
use scp_core::mcp_dto::McpAppOverviewResult;
use scp_mcp_sdk_spike::overview::{OverviewAdapter, OverviewHost, OverviewPolicy};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct SyntheticHost {
    fail: bool,
}

impl OverviewHost for SyntheticHost {
    async fn read_overview(&self, _: CancellationToken) -> Result<McpAppOverviewResult, String> {
        eprintln!("fixture host read");
        if self.fail {
            return Err("password=TOP_SECRET native /private/path".into());
        }
        Ok(McpAppOverviewResult {
            app_version: "stdio-fake".into(),
            active_functions: 2,
            known_function_count: 5,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let allowed_categories = match mode.as_str() {
        "denied" => vec![],
        "bad-category" => vec!["read-only".into(), "SECRET_CATEGORY".into()],
        _ => vec!["read-only".into()],
    };
    let sensitive_action_policy = if mode == "bad-policy" {
        "SECRET_POLICY"
    } else {
        "require-ui-approval"
    };
    let adapter = OverviewAdapter::new(
        SyntheticHost {
            fail: mode == "error",
        },
        OverviewPolicy {
            allowed_categories,
            sensitive_action_policy: sensitive_action_policy.into(),
        },
    );
    eprintln!("fixture started");
    let service = adapter.serve(rmcp::transport::stdio_pipes()).await?;
    service.waiting().await?;
    eprintln!("fixture stopped");
    Ok(())
}
