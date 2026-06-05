use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::{Json, Router};
use client_rs::TypedApi;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serverless_api::{
    EventTrigger, Function, FunctionRuntime, FunctionStatus, TriggerTargetKind, Workflow,
};

use crate::http::{HttpResult, api_error, bad_request, decode_json_body};
use crate::python::run_python_function;
use crate::runtime::{RuntimeSnapshot, runtime_key};
use crate::state::{AppState, load_function, load_workflow, object_namespace};
use crate::status::{
    patch_event_trigger_success, patch_function_runtime_status, patch_workflow_status,
    update_function_status,
};
use crate::workflow::{WorkflowInvokeResponse, WorkflowTraceEntry, next_step};

#[derive(Debug, Deserialize)]
pub(crate) struct UploadQuery {
    handler: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct InvokeResponse {
    result: Value,
    runtime: RuntimeSnapshot,
}

#[derive(Debug, Serialize)]
pub(crate) struct EventResponse {
    event_type: String,
    delivered: usize,
    results: Vec<EventDelivery>,
}

#[derive(Debug, Serialize)]
struct EventDelivery {
    namespace: String,
    trigger: String,
    target_kind: String,
    target_name: String,
    result: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunctionStateResponse {
    namespace: String,
    name: String,
    runtime: RuntimeSnapshot,
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", axum::routing::get(healthz))
        .route(
            "/api/v1/namespaces/:namespace/functions/:name/source",
            axum::routing::post(upload_function_source).put(upload_function_source),
        )
        .route(
            "/api/v1/namespaces/:namespace/functions/:name/invoke",
            axum::routing::post(invoke_function),
        )
        .route(
            "/api/v1/namespaces/:namespace/functions/:name/state",
            axum::routing::get(get_function_state),
        )
        .route(
            "/api/v1/namespaces/:namespace/workflows/:name/invoke",
            axum::routing::post(invoke_workflow),
        )
        .route(
            "/api/v1/events/:event_type",
            axum::routing::post(publish_event),
        )
}

async fn healthz() -> &'static str {
    "ok"
}

async fn upload_function_source(
    State(state): State<AppState>,
    Path((namespace, name)): Path<(String, String)>,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> HttpResult<Json<Function>> {
    let source = String::from_utf8(body.to_vec())
        .map_err(|error| bad_request(format!("function source must be UTF-8: {error}")))?;
    if source.trim().is_empty() {
        return Err(bad_request("function source must not be empty"));
    }
    let api = TypedApi::<Function>::namespaced(state.client.clone(), namespace.clone());
    let mut function = api.get(&name).await.map_err(|error| {
        api_error(format!(
            "failed to get Function {namespace}/{name}: {error}"
        ))
    })?;
    function.spec.source.inline = Some(source);
    if let Some(handler) = query.handler.filter(|handler| !handler.trim().is_empty()) {
        function.spec.handler = handler;
    }
    let updated = api.replace(&name, &function).await.map_err(|error| {
        api_error(format!(
            "failed to update Function {namespace}/{name}: {error}"
        ))
    })?;
    update_function_status(
        &state,
        &namespace,
        &name,
        FunctionStatus {
            ready: updated.spec.source.inline.is_some(),
            active_instances: 0,
            in_flight: 0,
            last_invoked_at: updated.status.last_invoked_at,
            last_error: None,
        },
    )
    .await;
    Ok(Json(updated))
}

async fn invoke_function(
    State(state): State<AppState>,
    Path((namespace, name)): Path<(String, String)>,
    body: Bytes,
) -> HttpResult<Json<InvokeResponse>> {
    let input = decode_json_body(body)?;
    invoke_function_inner(&state, &namespace, &name, input)
        .await
        .map(Json)
}

async fn get_function_state(
    State(state): State<AppState>,
    Path((namespace, name)): Path<(String, String)>,
) -> Json<FunctionStateResponse> {
    Json(FunctionStateResponse {
        namespace: namespace.clone(),
        name: name.clone(),
        runtime: state.runtime.snapshot(&runtime_key(&namespace, &name)),
    })
}

async fn invoke_workflow(
    State(state): State<AppState>,
    Path((namespace, name)): Path<(String, String)>,
    body: Bytes,
) -> HttpResult<Json<WorkflowInvokeResponse>> {
    let input = decode_json_body(body)?;
    invoke_workflow_inner(&state, &namespace, &name, input)
        .await
        .map(Json)
}

async fn publish_event(
    State(state): State<AppState>,
    Path(event_type): Path<String>,
    body: Bytes,
) -> HttpResult<Json<EventResponse>> {
    let input = decode_json_body(body)?;
    let mut results = Vec::new();
    for trigger in state
        .triggers
        .items()
        .into_iter()
        .filter(|trigger| trigger.spec.event_type == event_type)
    {
        let namespace = object_namespace(&trigger.metadata);
        let delivery = deliver_event(&state, &namespace, &trigger, input.clone()).await?;
        patch_event_trigger_success(&state, &namespace, &trigger).await;
        results.push(delivery);
    }
    Ok(Json(EventResponse {
        event_type,
        delivered: results.len(),
        results,
    }))
}

async fn deliver_event(
    state: &AppState,
    namespace: &str,
    trigger: &EventTrigger,
    input: Value,
) -> HttpResult<EventDelivery> {
    match trigger.spec.target.kind {
        TriggerTargetKind::Function => {
            let response =
                invoke_function_inner(state, namespace, &trigger.spec.target.name, input).await?;
            Ok(EventDelivery {
                namespace: namespace.to_string(),
                trigger: trigger.metadata.name.clone(),
                target_kind: "Function".to_string(),
                target_name: trigger.spec.target.name.clone(),
                result: response.result,
            })
        }
        TriggerTargetKind::Workflow => {
            let response =
                invoke_workflow_inner(state, namespace, &trigger.spec.target.name, input).await?;
            Ok(EventDelivery {
                namespace: namespace.to_string(),
                trigger: trigger.metadata.name.clone(),
                target_kind: "Workflow".to_string(),
                target_name: trigger.spec.target.name.clone(),
                result: response.result,
            })
        }
    }
}

pub(crate) async fn invoke_function_inner(
    state: &AppState,
    namespace: &str,
    name: &str,
    input: Value,
) -> HttpResult<InvokeResponse> {
    let function = load_function(state, namespace, name).await?;
    match function.spec.runtime {
        FunctionRuntime::Python => {}
    }

    let started = state.runtime.begin(&function);
    patch_function_runtime_status(state, &function, &started, None).await;

    let result = run_python_function(state, &function, input).await;
    let finished = state.runtime.end(&function);
    match result {
        Ok(result) => {
            patch_function_runtime_status(state, &function, &finished, None).await;
            Ok(InvokeResponse {
                result,
                runtime: finished,
            })
        }
        Err(error) => {
            patch_function_runtime_status(state, &function, &finished, Some(error.to_string()))
                .await;
            Err(api_error(format!(
                "function {}/{} failed: {error:#}",
                object_namespace(&function.metadata),
                function.metadata.name
            )))
        }
    }
}

pub(crate) async fn invoke_workflow_inner(
    state: &AppState,
    namespace: &str,
    name: &str,
    input: Value,
) -> HttpResult<WorkflowInvokeResponse> {
    let workflow = load_workflow(state, namespace, name).await?;
    invoke_loaded_workflow(state, namespace, name, workflow, input).await
}

async fn invoke_loaded_workflow(
    state: &AppState,
    namespace: &str,
    name: &str,
    workflow: Workflow,
    input: Value,
) -> HttpResult<WorkflowInvokeResponse> {
    let mut current = workflow.spec.entrypoint.clone();
    let mut value = input;
    let mut trace = Vec::new();

    for _ in 0..64 {
        let step = workflow.spec.steps.get(&current).ok_or_else(|| {
            api_error(format!(
                "workflow {namespace}/{name} references missing step {current}"
            ))
        })?;
        let response = invoke_function_inner(state, namespace, &step.function, value).await?;
        value = response.result;
        trace.push(WorkflowTraceEntry {
            step: current.clone(),
            function: step.function.clone(),
            output: value.clone(),
        });
        match next_step(step, &value) {
            Some(next) => current = next,
            None => {
                patch_workflow_status(state, &workflow, None).await;
                return Ok(WorkflowInvokeResponse {
                    result: value,
                    trace,
                });
            }
        }
    }

    let error = "workflow exceeded 64 steps; possible cycle".to_string();
    patch_workflow_status(state, &workflow, Some(error.clone())).await;
    Err(api_error(error))
}
