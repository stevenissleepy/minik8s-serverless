use axum::Router;

use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    health_routes()
        .merge(crate::serving::routes())
        .merge(crate::eventing::routes())
}

pub(crate) fn health_routes() -> Router<AppState> {
    Router::new().route("/healthz", axum::routing::get(healthz))
}

async fn healthz() -> &'static str {
    "ok"
}
