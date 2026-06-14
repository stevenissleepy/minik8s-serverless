use chrono::Utc;
use client_rs::{PatchType, TypedApi};
use serde_json::json;
use serverless_api::{
    EventSource, EventSourceStatus, EventTrigger, EventTriggerStatus, ServerlessService, Workflow,
    WorkflowStatus,
};

use crate::serving::runtime::RuntimeSnapshot;
use crate::serving::runtime_pods::{service_revision_name, service_url};
use crate::state::{AppState, object_namespace};

/// Activator-owned runtime signal. This is intentionally a merge patch because
/// serverless-controller owns observed fields such as `activeInstances`.
pub(crate) async fn patch_service_runtime_status(
    state: &AppState,
    service: &ServerlessService,
    snapshot: &RuntimeSnapshot,
    last_error: Option<String>,
) {
    let namespace = object_namespace(&service.metadata);
    let last_error = match last_error {
        Some(error) => json!(error),
        None => serde_json::Value::Null,
    };
    patch_service_status(
        state,
        &namespace,
        &service.metadata.name,
        json!({
            "status": {
                "desiredInstances": snapshot.active_instances,
                "inFlight": snapshot.in_flight,
                "lastInvokedAt": Utc::now(),
                "lastError": last_error,
            }
        }),
    )
    .await;
}

/// Activator-owned prewarm signal. It intentionally avoids touching
/// `inFlight` and `lastInvokedAt` because no user request has entered the
/// target function yet.
pub(crate) async fn patch_service_desired_status(
    state: &AppState,
    service: &ServerlessService,
    desired_instances: u32,
) {
    let namespace = object_namespace(&service.metadata);
    patch_service_status(
        state,
        &namespace,
        &service.metadata.name,
        json!({
            "status": {
                "desiredInstances": desired_instances,
            }
        }),
    )
    .await;
}

/// serverless-controller-owned observation after reconciling runtime resources.
pub(crate) async fn patch_service_observed_status(
    state: &AppState,
    service: &ServerlessService,
    active_instances: u32,
    last_error: Option<String>,
) {
    let namespace = object_namespace(&service.metadata);
    let patch = match last_error {
        Some(error) => json!({
            "status": {
                "ready": false,
                "latestRevision": service_revision_name(service),
                "activeInstances": active_instances,
                "url": service_url(service),
                "lastError": error,
            }
        }),
        None => json!({
            "status": {
                "ready": true,
                "latestRevision": service_revision_name(service),
                "activeInstances": active_instances,
                "url": service_url(service),
            }
        }),
    };
    patch_service_status(state, &namespace, &service.metadata.name, patch).await;
}

async fn patch_service_status(
    state: &AppState,
    namespace: &str,
    name: &str,
    patch: serde_json::Value,
) {
    let api =
        TypedApi::<ServerlessService>::namespaced(state.client.clone(), namespace.to_string());
    if let Err(error) = api.patch_status(name, PatchType::Merge, &patch).await {
        tracing::warn!(namespace, name, error = %error, "failed to update ServerlessService status");
    }
}

pub(crate) async fn patch_workflow_status(
    state: &AppState,
    workflow: &Workflow,
    last_error: Option<String>,
) {
    let namespace = object_namespace(&workflow.metadata);
    let api = TypedApi::<Workflow>::namespaced(state.client.clone(), namespace.clone());
    let status = WorkflowStatus {
        ready: workflow.spec.steps.contains_key(&workflow.spec.entrypoint),
        last_invoked_at: Some(Utc::now()),
        last_error,
    };
    if let Err(error) = api.replace_status(&workflow.metadata.name, &status).await {
        tracing::warn!(
            namespace,
            name = %workflow.metadata.name,
            error = %error,
            "failed to update workflow status"
        );
    }
}

pub(crate) async fn patch_event_source_fired(state: &AppState, source: &EventSource) {
    let namespace = object_namespace(&source.metadata);
    let api = TypedApi::<EventSource>::namespaced(state.client.clone(), namespace.clone());
    let status = EventSourceStatus {
        ready: true,
        event_count: source.status.event_count + 1,
        last_event_at: Some(Utc::now()),
        last_error: None,
    };
    if let Err(error) = api.replace_status(&source.metadata.name, &status).await {
        tracing::warn!(
            namespace,
            name = %source.metadata.name,
            error = %error,
            "failed to update eventsource status"
        );
    }
}

pub(crate) async fn patch_event_source_error(
    state: &AppState,
    source: &EventSource,
    message: String,
) {
    let namespace = object_namespace(&source.metadata);
    let api = TypedApi::<EventSource>::namespaced(state.client.clone(), namespace.clone());
    let status = EventSourceStatus {
        ready: false,
        event_count: source.status.event_count,
        last_event_at: source.status.last_event_at,
        last_error: Some(message),
    };
    if let Err(error) = api.replace_status(&source.metadata.name, &status).await {
        tracing::warn!(
            namespace,
            name = %source.metadata.name,
            error = %error,
            "failed to update eventsource status"
        );
    }
}

pub(crate) async fn patch_event_trigger_success(
    state: &AppState,
    namespace: &str,
    trigger: &EventTrigger,
) {
    let api = TypedApi::<EventTrigger>::namespaced(state.client.clone(), namespace.to_string());
    let status = EventTriggerStatus {
        ready: true,
        delivered_count: trigger.status.delivered_count + 1,
        last_delivered_at: Some(Utc::now()),
        last_error: None,
    };
    if let Err(error) = api.replace_status(&trigger.metadata.name, &status).await {
        tracing::warn!(
            namespace,
            name = %trigger.metadata.name,
            error = %error,
            "failed to update eventtrigger status"
        );
    }
}
