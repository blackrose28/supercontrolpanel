//! Real pipes/processes with synthetic work only. Watchdogs are fixture limits.
use std::{future::Future, process::Stdio, time::Duration};

use rmcp::{
    model::{CallToolRequestParams, ClientConfig, ProtocolVersion},
    ClientLifecycleMode, ClientServiceExt,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};

const MODERN: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";

#[path = "stdio/cancellation.rs"]
mod cancellation;
#[path = "stdio/eof.rs"]
mod eof;
#[path = "stdio/startup_eof.rs"]
mod startup_eof;

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("stdio fixture watchdog")
}

/// Test-only 64 KiB bound; inspect EVERY stdout line, including shutdown.
async fn read_response_line(
    output: &mut (impl tokio::io::AsyncBufRead + Unpin),
) -> Option<(Vec<u8>, Value)> {
    let mut bytes = Vec::new();
    let read = output
        .take(65537)
        .read_until(b'\n', &mut bytes)
        .await
        .unwrap();
    if read == 0 {
        return None;
    }
    assert!(
        read <= 65536 && bytes.ends_with(b"\n"),
        "invalid protocol line"
    );
    let value: Value = serde_json::from_slice(&bytes).expect("stdout must be protocol JSON");
    assert_eq!(value["jsonrpc"], "2.0");
    assert!(value.get("id").is_some());
    assert!(value.get("result").is_some() ^ value.get("error").is_some());
    Some((bytes, value))
}

fn spawn(mode: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_stdio-fixture"))
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

async fn finish(mut child: Child, expected_reads: usize) {
    assert!(bounded(child.wait()).await.unwrap().success());
    let mut diagnostics = String::new();
    bounded(
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut diagnostics),
    )
    .await
    .unwrap();
    assert_eq!(
        diagnostics.matches("fixture host read\n").count(),
        expected_reads
    );
    assert!(diagnostics.contains("fixture stopped\n"));
    assert!(!diagnostics.contains("SECRET"));
}

struct Wire {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    version: &'static str,
}

impl Wire {
    async fn start(version: &'static str, mode: &str) -> Self {
        Self::connect(version, spawn(mode)).await
    }

    async fn connect(version: &'static str, mut child: Child) -> Self {
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut wire = Self {
            child,
            input: Some(input),
            output,
            version,
        };
        let info = if version == LEGACY {
            let response = wire
                .request(
                    "initialize",
                    json!({"protocolVersion":version,
                "capabilities":{},"clientInfo":{"name":"stdio-probe","version":"0"}}),
                    1,
                )
                .await;
            assert_eq!(response["result"]["protocolVersion"], version);
            wire.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                .await;
            response
        } else {
            wire.request("server/discover", json!({}), 1).await
        };
        assert_eq!(info["result"]["capabilities"], json!({"tools":{}}));
        wire
    }

    async fn send(&mut self, value: Value) {
        let line = format!("{value}\n");
        bounded(self.input.as_mut().unwrap().write_all(line.as_bytes()))
            .await
            .unwrap();
        bounded(self.input.as_mut().unwrap().flush()).await.unwrap();
    }

    async fn line(&mut self) -> Option<Value> {
        bounded(read_response_line(&mut self.output))
            .await
            .map(|(_, value)| value)
    }

    async fn request(&mut self, method: &str, mut params: Value, id: u64) -> Value {
        if self.version == MODERN {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":MODERN,
                "io.modelcontextprotocol/clientInfo":{"name":"stdio-probe","version":"0"},
                "io.modelcontextprotocol/clientCapabilities":{}});
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await;
        let response = self.line().await.expect("protocol response before EOF");
        assert_eq!(response["id"], id);
        response
    }

    async fn finish(mut self, expected_reads: usize) {
        drop(self.input.take());
        assert!(self.line().await.is_none());
        finish(self.child, expected_reads).await;
    }
}

fn check_catalog(value: &Value) {
    let tools = value["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    let tool = &tools[0];
    assert_eq!(tool["name"], "app_get_overview");
    assert_eq!(
        tool["inputSchema"],
        json!({"type":"object","properties":{},"additionalProperties":false})
    );
    assert_eq!(
        tool["outputSchema"],
        json!({"type":"object","additionalProperties":false,
        "properties":{"appVersion":{"type":"string"},"activeFunctions":{"type":"integer","minimum":0},
        "knownFunctionCount":{"type":"integer","minimum":0}},
        "required":["appVersion","activeFunctions","knownFunctionCount"]})
    );
}

fn check_result(result: &Value, version: &str, failed: bool) {
    assert_eq!(result["isError"], failed);
    assert_eq!(result.get("resultType").is_some(), version == MODERN);
    if failed {
        assert_eq!(
            result["content"],
            json!([{"type":"text","text":"App overview unavailable"}])
        );
        assert!(result.get("structuredContent").is_none());
    } else {
        let expected =
            json!({"appVersion":"stdio-fake","activeFunctions":2,"knownFunctionCount":5});
        assert_eq!(result["structuredContent"], expected);
        assert_eq!(
            serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap(),
            expected
        );
    }
    assert!(!result.to_string().contains("SECRET"));
}

#[tokio::test]
async fn newline_protocol_catalog_dto_errors_and_idle_eof() {
    for version in [LEGACY, MODERN] {
        for mode in ["success", "error"] {
            let mut wire = Wire::start(version, mode).await;
            let listed = wire.request("tools/list", json!({}), 2).await;
            check_catalog(&listed["result"]);
            for (id, params) in [
                (3, json!({"name":"app_get_overview"})),
                (4, json!({"name":"app_get_overview","arguments":{}})),
            ] {
                let response = wire.request("tools/call", params, id).await;
                check_result(&response["result"], version, mode == "error");
            }
            wire.finish(2).await;
        }
    }
}

#[tokio::test]
async fn stdio_policy_and_argument_rejections_do_not_enter_host() {
    for version in [LEGACY, MODERN] {
        for mode in ["denied", "bad-policy", "bad-category", "success"] {
            let mut wire = Wire::start(version, mode).await;
            let listed = wire.request("tools/list", json!({}), 2).await;
            if mode == "success" {
                check_catalog(&listed["result"]);
            } else {
                assert_eq!(listed["result"]["tools"], json!([]));
            }
            let mut params = vec![
                json!({"name":"SECRET_UNKNOWN"}),
                json!({"name":"function_status_list"}),
                json!({"name":"app_get_overview","arguments":{"SECRET_ARG":true}}),
            ];
            if mode != "success" {
                params.push(json!({"name":"app_get_overview"}));
            }
            for (index, params) in params.into_iter().enumerate() {
                let response = wire.request("tools/call", params, index as u64 + 3).await;
                assert_eq!(response["error"]["code"], -32602);
                assert!(response["error"].get("data").is_none());
                assert!(!response.to_string().contains("SECRET"));
            }
            wire.finish(0).await;
        }
    }
}

#[tokio::test]
async fn official_stdio_client_uses_both_supported_lifecycles() {
    for version in [ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28] {
        for mode in ["success", "error"] {
            let mut child = spawn(mode);
            let input = child.stdin.take().unwrap();
            let output = child.stdout.take().unwrap();
            let lifecycle = if version.as_str() == LEGACY {
                ClientLifecycleMode::Initialize
            } else {
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![version.clone()],
                }
            };
            let client = bounded(
                ClientConfig::default()
                    .with_protocol_version(version.clone())
                    .serve_with_lifecycle((output, input), lifecycle),
            )
            .await
            .unwrap();
            let info = serde_json::to_value(client.peer_info().unwrap()).unwrap();
            assert_eq!(info["protocolVersion"], version.as_str());
            assert_eq!(info["capabilities"], json!({"tools":{}}));
            check_catalog(
                &serde_json::to_value(bounded(client.list_tools(None)).await.unwrap()).unwrap(),
            );
            let result = bounded(client.call_tool(CallToolRequestParams::new("app_get_overview")))
                .await
                .unwrap();
            check_result(
                &serde_json::to_value(result).unwrap(),
                version.as_str(),
                mode == "error",
            );
            bounded(client.cancel()).await.unwrap();
            finish(child, 1).await;
        }
    }
}
