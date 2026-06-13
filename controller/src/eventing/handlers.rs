use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::{Json, Router};
use serde::Serialize;
use serverless_api::CloudEvent;

use crate::eventing::broker::{EventDelivery, broker_publish};
use crate::http::{HttpResult, decode_json_body};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub(crate) struct EventResponse {
    event_type: String,
    delivered: usize,
    results: Vec<EventDelivery>,
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/v1/events/:event_type",
        axum::routing::post(publish_event),
    )
}

/// HTTP 事件入口（Broker ingress）：把 POST 进来的 JSON 包成一个 CloudEvent，再交给 Broker 扇出。
/// 这条路就是「自定义事件源」——任何外部系统都能往这里投递事件。
async fn publish_event(
    State(state): State<AppState>,
    Path(event_type): Path<String>,
    body: Bytes,
) -> HttpResult<Json<EventResponse>> {
    let data = decode_json_body(body)?;
    let event = CloudEvent::new(event_type.clone(), "/api/v1/events", data);
    let results = broker_publish(&state, &event).await;
    Ok(Json(EventResponse {
        event_type,
        delivered: results.len(),
        results,
    }))
}
