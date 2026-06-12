use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::Value;
use serverless_api::Workflow;

use crate::http::{HttpResult, api_error, decode_json_body};
use crate::serving::runtime::{RuntimeSnapshot, runtime_key};
use crate::serving::runtime_pods::invoke_service_pod;
use crate::serving::workflow::{WorkflowInvokeResponse, WorkflowTraceEntry, next_step};
use crate::state::{AppState, load_service, load_workflow, object_namespace};
use crate::status::{patch_service_runtime_status, patch_workflow_status};

#[derive(Debug, Serialize)]
pub(crate) struct InvokeResponse {
    pub(crate) result: Value,
    pub(crate) runtime: RuntimeSnapshot,
}

#[derive(Debug, Serialize)]
pub(crate) struct FunctionStateResponse {
    namespace: String,
    name: String,
    runtime: RuntimeSnapshot,
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/namespaces/:namespace/services/:name/invoke",
            axum::routing::post(invoke_function),
        )
        .route(
            "/api/v1/namespaces/:namespace/services/:name/state",
            axum::routing::get(get_function_state),
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

pub(crate) async fn invoke_function_inner(
    state: &AppState,
    namespace: &str,
    name: &str,
    input: Value,
) -> HttpResult<InvokeResponse> {
    let service = load_service(state, namespace, name).await?;

    let (started, in_flight) = state.runtime.begin(&service);
    patch_service_runtime_status(state, &service, &started, None).await;

    let result = invoke_service_pod(state, &service, input, started.active_instances).await;
    let finished = in_flight.finish();
    match result {
        Ok(result) => {
            patch_service_runtime_status(state, &service, &finished, None).await;
            Ok(InvokeResponse {
                result,
                runtime: finished,
            })
        }
        Err(error) => {
            patch_service_runtime_status(state, &service, &finished, Some(error.to_string())).await;
            Err(api_error(format!(
                "function {}/{} failed: {error:#}",
                object_namespace(&service.metadata),
                service.metadata.name
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
