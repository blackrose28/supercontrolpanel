//! Reviewed additions reuse desktop actions; no second runtime or credential store.
use super::contracts::{ErrorCode, Handler, OperationError};
use crate::models::BoundaryTargetSummary;
use serde_json::{json, Value};

pub(super) trait ParityHost {
    fn search(&self, filter: Option<String>) -> Result<Vec<BoundaryTargetSummary>, String>;
    fn detect(&self) -> Result<String, String>;
    fn rdp(&self, server_id: String) -> Result<(), String>;
    fn set_password(&self, server_id: String, password: String) -> Result<(), String>;
    fn clear_password(&self, server_id: String) -> Result<(), String>;
}

/// Called only after shared registry authorization and schema validation.
pub(super) fn dispatch(
    handler: Handler,
    args: &Value,
    host: &impl ParityHost,
) -> Result<Value, String> {
    let server_id = || args["serverId"].as_str().unwrap().to_owned();
    match handler {
        Handler::BoundarySearchTargets => {
            let filter = args
                .get("filter")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
            let targets = host.search(filter)?;
            Ok(json!(targets
                .into_iter()
                .take(limit)
                .map(SafeTarget::from)
                .collect::<Vec<_>>()))
        }
        Handler::DetectOpenvpnExecutable => {
            let path = host.detect().map_err(|error| {
                if error == "OpenVPN executable not found in PATH/common locations" {
                    OperationError::new(ErrorCode::NotFound).legacy_message()
                } else {
                    error
                }
            })?;
            Ok(json!({"path":path}))
        }
        Handler::ServerRdpStart => {
            host.rdp(server_id())?;
            Ok(json!({}))
        }
        Handler::ServerSshPasswordSet => {
            host.set_password(server_id(), args["password"].as_str().unwrap().to_owned())?;
            Ok(json!({}))
        }
        Handler::ServerSshPasswordClear => {
            host.clear_password(server_id())?;
            Ok(json!({}))
        }
        _ => Err(OperationError::new(ErrorCode::CapabilityUnavailable).legacy_message()),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SafeTarget {
    id: String,
    name: String,
    target_type: String,
    description: String,
    address: String,
}
impl From<BoundaryTargetSummary> for SafeTarget {
    fn from(target: BoundaryTargetSummary) -> Self {
        let BoundaryTargetSummary {
            id,
            name,
            target_type,
            description,
            address,
        } = target;
        Self {
            id,
            name,
            target_type,
            description,
            address,
        }
    }
}

fn validate_rdp_host(
    os: &str,
    interactive: bool,
    windows_server: bool,
    host: &str,
) -> Result<(), String> {
    if !matches!(os, "windows" | "macos" | "linux") || !interactive || !windows_server {
        return Err(OperationError::new(ErrorCode::CapabilityUnavailable).legacy_message());
    }
    // This value enters a native /v: argument or rdp:// URL; forbid URL/option syntax.
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c))
    {
        return Err(OperationError::new(ErrorCode::InvalidArguments).legacy_message());
    }
    Ok(())
}

pub(super) struct DesktopParityHost<'a>(pub &'a tauri::AppHandle);
impl ParityHost for DesktopParityHost<'_> {
    fn search(&self, filter: Option<String>) -> Result<Vec<BoundaryTargetSummary>, String> {
        // Existing desktop search matches name, ID and description, using App/CLI auth.
        crate::commands::search_boundary_targets(self.0.clone(), filter)
    }
    fn detect(&self) -> Result<String, String> {
        crate::services::detect_openvpn_executable_checked()?
            .ok_or_else(|| "OpenVPN executable not found in PATH/common locations".into())
    }
    fn rdp(&self, server_id: String) -> Result<(), String> {
        let server = crate::services::load_server_by_id(self.0, &server_id)?;
        let interactive = std::env::consts::OS != "linux"
            || ["DISPLAY", "WAYLAND_DISPLAY"]
                .iter()
                .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()));
        validate_rdp_host(
            std::env::consts::OS,
            interactive,
            server.os == crate::server_mgmt::ServerOs::Windows,
            server.host.trim(),
        )?;
        crate::commands::start_server_rdp(self.0.clone(), server_id)
    }
    fn set_password(&self, server_id: String, password: String) -> Result<(), String> {
        crate::commands::set_server_ssh_password(self.0.clone(), server_id, password)
    }
    fn clear_password(&self, server_id: String) -> Result<(), String> {
        crate::commands::clear_server_ssh_password(self.0.clone(), server_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{
        contracts::{execute, operation},
        validation::{conforms, output_schema},
    };
    use std::cell::RefCell;

    const CATEGORIES: &[&str] = &[
        "read-only",
        "runtime-control",
        "config-mutation",
        "file-transfer",
    ];

    #[test]
    fn slice5_parity_contracts_are_validated_and_sensitive() {
        for (name, args, sensitive) in [
            (
                "boundary_search_targets",
                json!({"filter":"prod", "limit":10}),
                false,
            ),
            ("detect_openvpn_executable", json!({}), false),
            ("server_rdp_start", json!({"serverId":"windows"}), true),
            (
                "server_ssh_password_set",
                json!({"serverId":"linux", "password":"secret"}),
                true,
            ),
            (
                "server_ssh_password_clear",
                json!({"serverId":"linux"}),
                true,
            ),
        ] {
            let op = operation(name).expect(name);
            assert_eq!(!op.read_only, sensitive);
            execute(name, "allow-with-token", CATEGORIES, &args, |_| Ok(())).unwrap();
            let mut called = false;
            let result = execute(name, "require-ui-approval", CATEGORIES, &args, |_| {
                called = true;
                Ok(())
            });
            assert_eq!(result.is_err(), sensitive);
            assert_eq!(called, !sensitive);
            assert!(
                execute::<()>(name, "allow-with-token", &[], &args, |_| panic!(
                    "category bypass"
                ))
                .is_err()
            );
            let mut typo = args.clone();
            typo["typo"] = json!(true);
            assert!(
                execute::<()>(name, "allow-with-token", CATEGORIES, &typo, |_| panic!(
                    "validation bypass"
                ))
                .is_err()
            );
        }
        for (name, args) in [
            ("boundary_search_targets", json!({"limit":0})),
            ("boundary_search_targets", json!({"limit":101})),
            ("boundary_search_targets", json!({"filter":42})),
            (
                "server_ssh_password_set",
                json!({"serverId":"x", "password":""}),
            ),
            ("server_ssh_password_set", json!({"serverId":"x"})),
            ("server_rdp_start", json!({"serverId":""})),
        ] {
            assert_eq!(
                execute::<()>(name, "allow-with-token", CATEGORIES, &args, |_| panic!(
                    "invalid call"
                ))
                .unwrap_err()
                .code,
                ErrorCode::InvalidArguments
            );
        }
    }

    #[derive(Default)]
    struct Host {
        calls: RefCell<Vec<Value>>,
        failure: Option<&'static str>,
    }
    impl Host {
        fn record(&self, args: Value) -> Result<(), String> {
            self.calls.borrow_mut().push(args);
            self.failure.map_or(Ok(()), |e| Err(e.into()))
        }
    }
    impl ParityHost for Host {
        fn search(&self, filter: Option<String>) -> Result<Vec<BoundaryTargetSummary>, String> {
            self.record(json!(["search", filter]))?;
            Ok((0..150)
                .map(|i| BoundaryTargetSummary {
                    id: format!("ttcp_{i}"),
                    name: "Production".into(),
                    target_type: "tcp".into(),
                    description: "Database".into(),
                    address: "db.internal".into(),
                })
                .collect())
        }
        fn detect(&self) -> Result<String, String> {
            self.record(json!(["detect"]))?;
            Ok("/usr/sbin/openvpn".into())
        }
        fn rdp(&self, id: String) -> Result<(), String> {
            self.record(json!(["rdp", id]))
        }
        fn set_password(&self, id: String, password: String) -> Result<(), String> {
            self.record(json!(["set", id, password]))
        }
        fn clear_password(&self, id: String) -> Result<(), String> {
            self.record(json!(["clear", id]))
        }
    }
    fn call(name: &str, args: Value, host: &Host) -> Result<Value, OperationError> {
        execute(
            name,
            "allow-with-token",
            &["read-only", "runtime-control", "config-mutation"],
            &args,
            |op| dispatch(op.handler, &args, host),
        )
    }
    #[test]
    fn search_forwards_filter_and_bounds_dedicated_public_results() {
        let host = Host::default();
        for (args, count, filter) in [
            (json!({}), 100, json!(null)),
            (
                json!({"filter":"Database", "limit":2}),
                2,
                json!("Database"),
            ),
        ] {
            let data = call("boundary_search_targets", args, &host).unwrap();
            assert_eq!(data.as_array().unwrap().len(), count);
            assert_eq!(
                host.calls.borrow().last().unwrap(),
                &json!(["search", filter])
            );
            assert_eq!(
                data[0],
                json!({"id":"ttcp_0", "name":"Production", "targetType":"tcp", "description":"Database", "address":"db.internal"})
            );
            assert!(conforms(
                &output_schema(operation("boundary_search_targets").unwrap()),
                &data
            ));
        }
        let host = Host {
            failure: Some("authentication failed: secret-token"),
            ..Default::default()
        };
        let error = call("boundary_search_targets", json!({}), &host).unwrap_err();
        assert_eq!(error.code, ErrorCode::OperationFailed);
        assert!(!serde_json::to_string(&error)
            .unwrap()
            .contains("secret-token"));
    }
    #[test]
    fn detection_distinguishes_missing_from_host_failures() {
        assert_eq!(
            call("detect_openvpn_executable", json!({}), &Host::default()).unwrap(),
            json!({"path":"/usr/sbin/openvpn"})
        );
        for (failure, code) in [
            (
                "OpenVPN executable not found in PATH/common locations",
                ErrorCode::NotFound,
            ),
            ("permission denied: secret", ErrorCode::OperationFailed),
        ] {
            let host = Host {
                failure: Some(failure),
                ..Default::default()
            };
            let error = call("detect_openvpn_executable", json!({}), &host).unwrap_err();
            assert_eq!(error.code, code);
            assert!(!serde_json::to_string(&error).unwrap().contains("secret"));
        }
    }
    #[test]
    fn credentials_are_write_only_and_host_errors_are_safe() {
        for (name, args, expected) in [
            (
                "server_ssh_password_set",
                json!({"serverId":"s", "password":"private"}),
                json!(["set", "s", "private"]),
            ),
            (
                "server_ssh_password_clear",
                json!({"serverId":"s"}),
                json!(["clear", "s"]),
            ),
            (
                "server_rdp_start",
                json!({"serverId":"s"}),
                json!(["rdp", "s"]),
            ),
        ] {
            let host = Host::default();
            assert_eq!(call(name, args.clone(), &host).unwrap(), json!({}));
            assert_eq!(host.calls.borrow()[0], expected);
            let host = Host {
                failure: Some("keyring/launcher private"),
                ..Default::default()
            };
            let error = call(name, args.clone(), &host).unwrap_err();
            assert_eq!(error.code, ErrorCode::OperationFailed);
            assert!(!serde_json::to_string(&error).unwrap().contains("private"));
            let host = Host {
                failure: Some("server not found"),
                ..Default::default()
            };
            assert_eq!(
                call(name, args, &host).unwrap_err().code,
                ErrorCode::NotFound
            );
        }
        assert!(operation("server_ssh_password_get").is_none());
    }
    #[test]
    fn rdp_requires_supported_interactive_host_and_safe_windows_destination() {
        for os in ["windows", "macos", "linux"] {
            for address in ["desktop.internal", "192.0.2.1", "[::1]", "2001:db8::1"] {
                validate_rdp_host(os, true, true, address).unwrap();
            }
        }
        for (os, interactive, windows, address, code) in [
            (
                "freebsd",
                true,
                true,
                "pc",
                ErrorCode::CapabilityUnavailable,
            ),
            ("linux", false, true, "pc", ErrorCode::CapabilityUnavailable),
            ("linux", true, false, "pc", ErrorCode::CapabilityUnavailable),
            ("windows", true, true, "", ErrorCode::InvalidArguments),
            (
                "macos",
                true,
                true,
                "pc?command=x",
                ErrorCode::InvalidArguments,
            ),
            (
                "linux",
                true,
                true,
                "user@pc/path",
                ErrorCode::InvalidArguments,
            ),
            (
                "linux",
                true,
                true,
                "pc\n/option",
                ErrorCode::InvalidArguments,
            ),
        ] {
            assert_eq!(
                validate_rdp_host(os, interactive, windows, address).unwrap_err(),
                OperationError::new(code).legacy_message()
            );
        }
    }
}
