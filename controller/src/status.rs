use chrono::Utc;
use client_rs::TypedApi;
use serverless_api::{
    EventTrigger, EventTriggerStatus, ServerlessService, ServerlessServiceStatus, Workflow,
    WorkflowStatus,
};

use crate::serving::runtime::RuntimeSnapshot;
use crate::serving::runtime_pods::service_status;
use crate::state::{AppState, object_namespace};

pub(crate) async fn patch_service_runtime_status(
    state: &AppState,
    service: &ServerlessService,
    snapshot: &RuntimeSnapshot,
    last_error: Option<String>,
) {
    let namespace = object_namespace(&service.metadata);
    update_service_status(
        state,
        &namespace,
        &service.metadata.name,
        service_status(
            service,
            snapshot.active_instances,
            snapshot.in_flight,
            last_error,
        ),
    )
    .await;
}

pub(crate) async fn update_service_status(
    state: &AppState,
    namespace: &str,
    name: &str,
    status: ServerlessServiceStatus,
) {
    let api =
        TypedApi::<ServerlessService>::namespaced(state.client.clone(), namespace.to_string());
    if let Err(error) = api.replace_status(name, &status).await {
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
