use chrono::Utc;
use client_rs::TypedApi;
use serverless_api::{
    EventTrigger, EventTriggerStatus, Function, FunctionStatus, Workflow, WorkflowStatus,
};

use crate::runtime::RuntimeSnapshot;
use crate::state::{AppState, object_namespace};

pub(crate) async fn patch_function_runtime_status(
    state: &AppState,
    function: &Function,
    snapshot: &RuntimeSnapshot,
    last_error: Option<String>,
) {
    let namespace = object_namespace(&function.metadata);
    update_function_status(
        state,
        &namespace,
        &function.metadata.name,
        FunctionStatus {
            ready: function.spec.source.inline.is_some(),
            active_instances: snapshot.active_instances,
            in_flight: snapshot.in_flight,
            last_invoked_at: Some(Utc::now()),
            last_error,
        },
    )
    .await;
}

pub(crate) async fn update_function_status(
    state: &AppState,
    namespace: &str,
    name: &str,
    status: FunctionStatus,
) {
    let api = TypedApi::<Function>::namespaced(state.client.clone(), namespace.to_string());
    if let Err(error) = api.replace_status(name, &status).await {
        tracing::warn!(namespace, name, error = %error, "failed to update function status");
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
