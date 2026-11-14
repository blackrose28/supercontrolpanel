//! Shared registry-driven SDK ServerHandler parameterized for desktop HTTP or standalone stdio.
use crate::contracts::{
    operation, resource_operation, ErrorCode, Lifetime, Operation, OperationError, OPERATIONS,
};
use crate::lifecycle::{CancellationToken, TransportControl};
use crate::standalone::{execute_standalone, permitted_standalone_operations};
use crate::validation;
use rmcp::{
    model::*,
    service::{RequestContext, SubscriptionContext},
    ErrorData, RoleServer, ServerHandler,
};
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::KeyringBackend;
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

pub type DiscoveryHandler = dyn Fn() -> Result<Vec<String>, OperationError> + Send + Sync;
pub type OperationHandler = dyn Fn(&str, Value) -> Result<Value, OperationError> + Send + Sync;

#[derive(Debug, Clone, Copy)]
pub enum HostKind {
    Desktop { version: &'static str },
    StandaloneStdio { version: &'static str },
}

#[derive(Clone)]
pub struct SdkHost {
    pub kind: HostKind,
    pub control: TransportControl,
    pub subscriptions: Arc<Mutex<HashMap<String, CancellationToken>>>,
    pub permitted: Arc<DiscoveryHandler>,
    pub execute: Arc<OperationHandler>,
}

impl SdkHost {
    pub fn standalone<B: KeyringBackend + 'static>(
        runtime: Arc<ProfileRuntime<B>>,
        version: &'static str,
    ) -> Self {
        let discovery_rt = Arc::clone(&runtime);
        let exec_rt = Arc::clone(&runtime);
        Self {
            kind: HostKind::StandaloneStdio { version },
            control: TransportControl::default(),
            subscriptions: Default::default(),
            permitted: Arc::new(move || {
                let policy = discovery_rt
                    .get_stdio_policy()
                    .map_err(|e| crate::contracts::map_core_error(&e))?;
                Ok(permitted_standalone_operations(policy)
                    .into_iter()
                    .map(|op| op.name.to_string())
                    .collect())
            }),
            execute: Arc::new(move |name, args| execute_standalone(&exec_rt, version, name, &args)),
        }
    }

    async fn invoke(
        &self,
        op: &'static Operation,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<Value, OperationError> {
        let execute = self.execute.clone();
        let deadline = operation_deadline(Some(op));
        let data = tokio::time::timeout(
            deadline,
            self.control.run(cancel, move || execute(op.name, args)),
        )
        .await
        .map_err(|_| OperationError::new(ErrorCode::Timeout))??;
        if !validation::conforms(&validation::output_schema(op), &data) {
            return Err(OperationError::new(ErrorCode::OperationFailed));
        }
        Ok(standard_data(op, data))
    }

    async fn visible(&self) -> Result<Vec<&'static Operation>, ErrorData> {
        let permitted = self.permitted.clone();
        let names = tokio::time::timeout(
            Duration::from_secs(30),
            self.control
                .run(CancellationToken::new(), move || permitted()),
        )
        .await
        .map_err(|_| ErrorData::internal_error("TIMEOUT", None))?
        .map_err(|e| ErrorData::internal_error(e.legacy_message(), None))?;
        Ok(OPERATIONS
            .iter()
            .filter(|op| names.iter().any(|name| name == op.name))
            .collect())
    }

    async fn require_visible_resources(&self, uris: &[String]) -> Result<(), ErrorData> {
        let visible = self.visible().await?;
        if uris.iter().any(|uri| {
            !visible
                .iter()
                .any(|op| op.resource.is_some_and(|(u, _, _)| u == uri))
        }) {
            return Err(ErrorData::resource_not_found(
                "POLICY_DENIED: refresh discovery and snapshots",
                None,
            ));
        }
        Ok(())
    }
}

impl ServerHandler for SdkHost {
    fn get_info(&self) -> ServerConfig {
        let (name, version) = match self.kind {
            HostKind::Desktop { version } => ("super-control-panel", version),
            HostKind::StandaloneStdio { version } => ("scpctl-stdio", version),
        };
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .build(),
        )
        .with_server_info(Implementation::new(name, version))
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self
            .visible()
            .await?
            .into_iter()
            .map(|op| standard_tool(self.kind, op))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let op = operation(&request.name)
            .ok_or_else(|| ErrorData::invalid_params("Unknown tool", None))?;
        let args = Value::Object(request.arguments.unwrap_or_default());
        validation::validate_arguments(op, &args)
            .map_err(|_| ErrorData::invalid_params("Invalid tool arguments", None))?;
        match self.invoke(op, args, context.ct).await {
            Ok(data) => Ok(CallToolResult::structured(data).into()),
            Err(error) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(error.legacy_message())]).into())
            }
        }
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resources = self
            .visible()
            .await?
            .into_iter()
            .filter_map(|op| op.resource)
            .map(|(uri, title, _)| Resource::new(uri, title).with_mime_type("application/json"))
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let op = resource_operation(&request.uri)
            .ok_or_else(|| ErrorData::resource_not_found("Unknown resource", None))?;
        let data = self
            .invoke(op, json!({}), context.ct)
            .await
            .map_err(|e| ErrorData::resource_not_found(e.legacy_message(), None))?;
        Ok(
            ReadResourceResult::new(vec![ResourceContents::text(data.to_string(), request.uri)
                .with_mime_type("application/json")])
            .into(),
        )
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let uri = request.uri;
        self.require_visible_resources(std::slice::from_ref(&uri))
            .await?;
        let permit = self
            .control
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                ErrorData::internal_error(
                    "CONFLICT: update stream capacity reached; poll snapshots",
                    None,
                )
            })?;
        let cancel = self.control.stop.child_token();
        if let Some(old) = self
            .subscriptions
            .lock()
            .map_err(|_| ErrorData::internal_error("OPERATION_FAILED", None))?
            .insert(uri.clone(), cancel.clone())
        {
            old.cancel();
        }
        spawn_subscription_loop(self.clone(), uri, cancel, permit, context);
        Ok(())
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if let Some(cancel) = self
            .subscriptions
            .lock()
            .map_err(|_| ErrorData::internal_error("OPERATION_FAILED", None))?
            .remove(&request.uri)
        {
            cancel.cancel();
        }
        Ok(())
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        let uris = requested
            .resource_subscriptions
            .as_ref()?
            .iter()
            .filter(|uri| resource_operation(uri).is_some())
            .take(6)
            .cloned()
            .collect();
        let mut filter = SubscriptionFilter::new();
        filter.resource_subscriptions = Some(uris);
        Some(filter)
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let _permit = self
            .control
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                ErrorData::internal_error(
                    "CONFLICT: update stream capacity reached; poll snapshots",
                    None,
                )
            })?;
        let uris = context
            .accepted()
            .resource_subscriptions
            .clone()
            .unwrap_or_default();
        let mut updates = self.control.updates.subscribe();
        loop {
            self.require_visible_resources(&uris).await?;
            for uri in &uris {
                tokio::select! {
                    _ = context.cancelled() => return Ok(()),
                    _ = self.control.stop.cancelled() => return Ok(()),
                    result = tokio::time::timeout(Duration::from_secs(5), context.sink().notify_resource_updated(uri.clone())) => {
                        if !matches!(result, Ok(Ok(()))) { return Ok(()); }
                    }
                }
            }
            tokio::select! {
                _ = context.cancelled() => return Ok(()),
                _ = self.control.stop.cancelled() => return Ok(()),
                _ = updates.changed() => {},
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
        }
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
        ])
    }
}

fn spawn_subscription_loop(
    host: SdkHost,
    uri: String,
    cancel: CancellationToken,
    permit: tokio::sync::OwnedSemaphorePermit,
    context: RequestContext<RoleServer>,
) {
    tokio::spawn(async move {
        let _permit = permit;
        let mut updates = host.control.updates.subscribe();
        loop {
            if context.peer.is_transport_closed()
                || host
                    .require_visible_resources(std::slice::from_ref(&uri))
                    .await
                    .is_err()
            {
                break;
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                result = tokio::time::timeout(
                    Duration::from_secs(5),
                    context.peer.notify_resource_updated(ResourceUpdatedNotificationParam::new(uri.clone())),
                ) => {
                    if !matches!(result, Ok(Ok(()))) { break; }
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = updates.changed() => {},
                _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
        }
    });
}

fn operation_deadline(op: Option<&Operation>) -> Duration {
    match op.map(|op| op.lifetime) {
        Some(Lifetime::Request) => Duration::from_secs(30 * 60),
        Some(Lifetime::Query) | None => Duration::from_secs(30),
        _ => Duration::from_secs(5 * 60),
    }
}

fn result_key(op: &Operation) -> &'static str {
    if op.name == "function_status_list" {
        "statuses"
    } else {
        "result"
    }
}

pub fn standard_data(op: &Operation, data: Value) -> Value {
    json!({(result_key(op)): data})
}

fn standard_tool(kind: HostKind, op: &Operation) -> Result<Tool, ErrorData> {
    let policy_note = match kind {
        HostKind::Desktop { .. } => {
            "Desktop-owned starts continue after disconnect; query state before retrying. Sensitive calls require allow-with-token in local settings. Protected import/Git capabilities remain unavailable."
        }
        HostKind::StandaloneStdio { .. } => {
            "Headless stdio host owns runtime sessions until exit; query state before retrying. Sensitive calls fail closed unless permitted by local stdio policy."
        }
    };
    serde_json::from_value(json!({
        "name": op.name,
        "title": op.title,
        "description": format!("{}. Category: {}. {}", op.title, op.category, policy_note),
        "inputSchema": validation::input_schema(op),
        "outputSchema": validation::output_object(json!({(result_key(op)): validation::output_schema(op)})),
        "annotations": {"readOnlyHint": op.read_only, "destructiveHint": !op.read_only}
    }))
    .map_err(|_| ErrorData::internal_error("OPERATION_FAILED", None))
}
