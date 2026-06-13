use axum::Router;

use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", axum::routing::get(healthz))
        .merge(crate::serving::routes())
        .merge(crate::eventing::routes())
}

async fn healthz() -> &'static str {
    "ok"
}
