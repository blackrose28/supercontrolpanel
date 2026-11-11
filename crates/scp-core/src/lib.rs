//! Host-independent shell result contract. No profile or runtime is opened by discovery.
use serde_json::{json, Value};

pub fn success(data: Value) -> Value {
    json!({"schemaVersion":1,"ok":true,"runtimeId":null,"data":data,"error":null})
}

pub fn invalid_arguments() -> Value {
    json!({"schemaVersion":1,"ok":false,"runtimeId":null,"data":null,"error":{
        "code":"INVALID_ARGUMENTS","message":"Unsupported command or arguments. Run scpctl --help.",
        "retryable":false,"reason":"invalid_arguments","nextAction":"inspect_schema","outcome":"not_started"
    }})
}
