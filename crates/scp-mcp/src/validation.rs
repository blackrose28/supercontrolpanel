//! The supported JSON Schema subset is also the validator, keeping discovery and execution aligned.
use super::contracts::{ErrorCode, Operation, OperationError};
use serde_json::{json, Value};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
}
fn string() -> Value {
    json!({"type":"string","maxLength":4096})
}
fn identifier() -> Value {
    json!({"type":"string","minLength":1,"maxLength":256})
}
fn port() -> Value {
    json!({"type":"integer","minimum":1,"maximum":65535})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}

pub fn config_schema(partial: bool) -> Value {
    let mut vpn = json!({});
    for key in ["username", "password", "otpSecret"] {
        vpn[key] = string();
    }
    for key in ["terminateOpenVpnWhenHelperExits", "autoReconnectEnabled"] {
        vpn[key] = boolean();
    }
    for key in ["autoReconnectBaseSeconds", "autoReconnectMaxSeconds"] {
        vpn[key] = json!({"type":"integer","minimum":1,"maximum":86400});
    }
    let mut boundary = json!({});
    for key in [
        "addr",
        "authMethodId",
        "loginName",
        "password",
        "oidcUsername",
        "oidcPassword",
        "oidcOtpSecret",
    ] {
        boundary[key] = string();
    }
    boundary["authKind"] = json!({"type":"string","enum":["password","oidc"]});
    boundary["oidcLoginMode"] = json!({"type":"string","enum":["browser","automated"]});
    // Full replacements can retain local executable fields only after comparison in the host.
    if !partial {
        for key in ["command", "executablePath", "ovpnPath", "extraArgs"] {
            vpn[key] = string();
        }
        for key in ["cliPath", "connectSshOptions"] {
            boundary[key] = string();
        }
    }
    let app = object(
        json!({"defaultTerminalCols":port(),"defaultTerminalRows":port()}),
        if partial {
            &[]
        } else {
            &["defaultTerminalCols", "defaultTerminalRows"]
        },
    );
    object(
        json!({"app":app,"openvpn":object(vpn,&[]),"boundary":object(boundary,&[])}),
        if partial { &[] } else { &["app"] },
    )
}

fn server_schema() -> Value {
    object(
        json!({
            "id":string(),"name":identifier(),"host":identifier(),"os":{"type":"string","enum":["linux","windows"]},
            "connectionMethod":{"type":"string","enum":["ssh","boundary"]},"sshPort":port(),"sshUser":identifier(),
            "boundaryTargetId":string(),"boundaryListenPort":{"type":"integer","minimum":0,"maximum":65535},
            "services":{"type":"array","maxItems":128,"items":object(json!({"name":identifier(),"port":port()}), &["name","port"])}
        }),
        &[
            "id",
            "name",
            "host",
            "connectionMethod",
            "sshPort",
            "sshUser",
        ],
    )
}

fn required_arguments(handler: super::contracts::Handler) -> &'static [&'static str] {
    use super::contracts::Handler::*;
    match handler {
        FunctionStatusGet => &["functionId"],
        ServerGet | ServerTunnelStart | ServerTunnelStop | ServerSshStart | ServerSshStop
        | RemoteBrowserOpen => &["serverId"],
        ServerRdpStart | ServerSshPasswordClear => &["serverId"],
        ServerSshPasswordSet => &["serverId", "password"],
        ServerDelete => &["id"],
        ServerUpsert => &["server"],
        CaddyStart => &["folder"],
        ConfigSave => &["config"],
        ConfigSavePartial | SyncSettingsSavePartial => &["patch"],
        RemoteBrowserClose | RemoteBrowserListChildren | RemoteBrowserStatPath => &["browserId"],
        RemoteBrowserCreateDirectory | RemoteBrowserDeletePath => &["browserId", "path"],
        ScpUpload | ScpDownload | ScpStartUpload | ScpStartDownload => {
            &["serverId", "localPath", "remotePath"]
        }
        ScpTransferGet => &["transferId"],
        ConfigSyncPull | ConfigSyncPush | ConfigGitPull | ConfigGitPush => &["repoRoot"],
        BoundarySearchTargets
        | DetectOpenvpnExecutable
        | AppGetOverview
        | FunctionStatusList
        | OpenvpnGetStatus
        | BoundaryGetStatus
        | ServersList
        | ConfigGetPaths
        | OpenvpnStart
        | OpenvpnStop
        | BoundaryAuthStart
        | BoundaryAuthStop
        | CaddyStop
        | SyncSettingsGet
        | ScpTransferList => &[],
    }
}

pub fn input_schema(op: &Operation) -> Value {
    use super::contracts::Handler::*;
    let mut properties = json!({});
    let required = required_arguments(op.handler);
    for key in required {
        properties[*key] = identifier();
    }
    match op.handler {
        BoundarySearchTargets => {
            properties["filter"] = string();
            properties["limit"] = json!({"type":"integer","minimum":1,"maximum":100});
        }
        ServerSshPasswordSet => {
            properties["password"] =
                json!({"type":"string","minLength":1,"maxLength":4096,"writeOnly":true});
        }
        OpenvpnStart => {
            properties["username"] = string();
            properties["password"] = string();
        }
        ServerUpsert => properties["server"] = server_schema(),
        ConfigSave => properties["config"] = config_schema(false),
        ConfigSavePartial => properties["patch"] = config_schema(true),
        SyncSettingsSavePartial => properties["patch"] = object(json!({"repoRoot":string()}), &[]),
        RemoteBrowserListChildren | RemoteBrowserStatPath => properties["path"] = string(),
        RemoteBrowserDeletePath | ScpUpload | ScpDownload | ScpStartUpload | ScpStartDownload => {
            properties["recursive"] = boolean()
        }
        _ => {}
    }
    if matches!(
        op.handler,
        BoundaryAuthStart | ServerTunnelStart | ServerSshStart | CaddyStart
    ) {
        properties["cols"] = port();
        properties["rows"] = port();
    }
    object(properties, required)
}

pub fn validate_arguments(op: &Operation, args: &Value) -> Result<(), OperationError> {
    if conforms(&input_schema(op), args) {
        Ok(())
    } else {
        Err(OperationError::new(ErrorCode::InvalidArguments))
    }
}

pub fn conforms(schema: &Value, value: &Value) -> bool {
    if let Some(choices) = schema.get("anyOf").and_then(Value::as_array) {
        return choices.iter().any(|shape| conforms(shape, value));
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array) {
        if !choices.contains(value) {
            return false;
        }
    }
    match schema["type"].as_str() {
        Some("null") => value.is_null(),
        Some("object") => validate_object(schema, value),
        Some("string") => value.as_str().is_some_and(|s| {
            !s.contains('\0')
                && s.len() <= schema["maxLength"].as_u64().unwrap_or(u64::MAX) as usize
                && s.len() >= schema["minLength"].as_u64().unwrap_or(0) as usize
        }),
        Some("integer") => value.as_u64().is_some_and(|n| {
            n >= schema["minimum"].as_u64().unwrap_or(0)
                && n <= schema["maximum"].as_u64().unwrap_or(u64::MAX)
        }),
        Some("boolean") => value.is_boolean(),
        Some("array") => value.as_array().is_some_and(|items| {
            items.len() <= schema["maxItems"].as_u64().unwrap_or(u64::MAX) as usize
                && items.iter().all(|item| conforms(&schema["items"], item))
        }),
        _ => false,
    }
}
fn validate_object(schema: &Value, value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    let Some(required) = schema["required"].as_array() else {
        return false;
    };
    required
        .iter()
        .all(|key| key.as_str().is_some_and(|key| map.contains_key(key)))
        && map.iter().all(|(key, value)| {
            schema["properties"]
                .get(key)
                .is_some_and(|shape| conforms(shape, value))
        })
}

pub fn protect_config_replacement(current: &Value, proposed: &Value) -> Result<(), OperationError> {
    for (section, key) in [
        ("openvpn", "command"),
        ("openvpn", "executablePath"),
        ("openvpn", "ovpnPath"),
        ("openvpn", "extraArgs"),
        ("boundary", "cliPath"),
        ("boundary", "connectSshOptions"),
    ] {
        if current.get(section).and_then(|v| v.get(key))
            != proposed.get(section).and_then(|v| v.get(key))
        {
            return Err(OperationError::new(ErrorCode::PolicyDenied));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacements_cannot_change_or_erase_local_launch_configuration() {
        let current = json!({"openvpn":{"command":"local","executablePath":"vpn","ovpnPath":"profile","extraArgs":"safe"},"boundary":{"cliPath":"boundary","connectSshOptions":"safe"}});
        protect_config_replacement(&current, &current).unwrap();
        for (section, key) in [
            ("openvpn", "command"),
            ("openvpn", "executablePath"),
            ("openvpn", "ovpnPath"),
            ("openvpn", "extraArgs"),
            ("boundary", "cliPath"),
            ("boundary", "connectSshOptions"),
        ] {
            let mut proposed = current.clone();
            proposed[section][key] = json!("attacker");
            assert!(
                protect_config_replacement(&current, &proposed).is_err(),
                "{section}.{key}"
            );
            proposed[section].as_object_mut().unwrap().remove(key);
            assert!(protect_config_replacement(&current, &proposed).is_err());
        }
    }
}

pub fn output_object(properties: Value) -> Value {
    let keys: Vec<_> = properties
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    object(properties.clone(), &keys)
}
fn output_string() -> Value {
    json!({"type":"string"})
}
fn output_number() -> Value {
    json!({"type":"integer","minimum":0})
}
fn output_array(items: Value) -> Value {
    json!({"type":"array","items":items})
}
fn nullable(shape: Value) -> Value {
    json!({"anyOf":[shape,{"type":"null"}]})
}

pub fn output_schema(op: &Operation) -> Value {
    use super::{contracts::Handler::*, public_dto::*};
    let start = || output_object(json!({"functionId":output_string(),"sessionId":output_string()}));
    let server = || {
        output_object(json!({
            "id":output_string(),"name":output_string(),"host":output_string(),"os":output_string(),
            "connectionMethod":output_string(),"sshPort":output_number(),"sshUser":output_string(),
            "boundaryTargetId":output_string(),"boundaryListenPort":output_number(),
            "services":output_array(output_object(json!({"name":output_string(),"port":output_number()})))
        }))
    };
    match op.handler {
        BoundarySearchTargets => output_array(output_object(json!({
            "id":output_string(),"name":output_string(),"targetType":output_string(),
            "description":output_string(),"address":output_string()
        }))),
        DetectOpenvpnExecutable => output_object(json!({"path":output_string()})),
        ServerRdpStart | ServerSshPasswordSet | ServerSshPasswordClear => output_object(json!({})),
        AppGetOverview => output_object(
            json!({"appVersion":output_string(),"activeFunctions":output_number(),"knownFunctionCount":output_number()}),
        ),
        FunctionStatusList => output_array(SafeFunctionStatus::schema()),
        FunctionStatusGet => SafeFunctionStatus::schema(),
        OpenvpnGetStatus | BoundaryGetStatus => nullable(SafeFunctionStatus::schema()),
        ServersList | ServerUpsert | ServerDelete => output_array(server()),
        ServerGet => server(),
        ConfigGetPaths => output_object(json!({"appConfigPath":output_string()})),
        OpenvpnStart | BoundaryAuthStart | ServerTunnelStart | ServerSshStart | CaddyStart => {
            start()
        }
        ConfigSave | ConfigSavePartial => SafeAppConfig::schema(),
        SyncSettingsGet | SyncSettingsSavePartial => SafeSyncSettings::schema(),
        RemoteBrowserOpen => {
            output_object(json!({"browserId":output_string(),"rootPath":output_string()}))
        }
        RemoteBrowserListChildren => output_array(output_object(
            json!({"name":output_string(),"path":output_string(),"kind":output_string()}),
        )),
        RemoteBrowserStatPath => {
            output_object(json!({"path":output_string(),"kind":output_string(),"exists":boolean()}))
        }
        ScpUpload | ScpDownload | ConfigGitPull | ConfigGitPush => {
            output_object(json!({"result":output_string()}))
        }
        ScpStartUpload | ScpStartDownload => output_object(json!({"sessionId":output_string()})),
        ScpTransferList => output_array(SafeTransfer::schema()),
        ScpTransferGet => SafeTransfer::schema(),
        OpenvpnStop
        | BoundaryAuthStop
        | ServerTunnelStop
        | ServerSshStop
        | CaddyStop
        | RemoteBrowserClose
        | RemoteBrowserCreateDirectory
        | RemoteBrowserDeletePath
        | ConfigSyncPull
        | ConfigSyncPush => output_object(json!({})),
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;
    use crate::{
        contracts::operation,
        public_dto::{SafeAppConfig, SafeSyncSettings},
    };
    #[test]
    fn safe_output_schemas_match_public_fixtures_and_reject_credentials() {
        for (name, value) in [
            (
                "config_save",
                serde_json::to_value(SafeAppConfig::default()).unwrap(),
            ),
            (
                "sync_settings_get",
                serde_json::to_value(SafeSyncSettings::default()).unwrap(),
            ),
        ] {
            let schema = output_schema(operation(name).unwrap());
            assert!(conforms(&schema, &value), "{name}");
            let mut with_secret = value;
            with_secret["ccAgentToken"] = json!("secret");
            assert!(!conforms(&schema, &with_secret));
        }
    }
}
