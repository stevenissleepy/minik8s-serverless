use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::Value;
use serverless_api::{EventTrigger, TriggerTargetKind};

use crate::http::{HttpResult, decode_json_body};
use crate::serving::{invoke_function_inner, invoke_workflow_inner};
use crate::state::{AppState, object_namespace};
use crate::status::patch_event_trigger_success;

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

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/v1/events/:event_type",
        axum::routing::post(publish_event),
    )
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
