use std::time::Duration;

use crate::serving::runtime::RuntimeSnapshot;
use crate::serving::runtime_pods::{cleanup_orphan_runtime_resources, scale_service_pods};
use crate::state::AppState;
use crate::status::patch_service_runtime_status;
use serverless_api::ServerlessService;

pub(crate) async fn idle_scale_loop(state: AppState, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        for service in state.services.items() {
            if let Some(snapshot) = state.runtime.ensure_min_instances(&service) {
                apply_service_scale(&state, &service, &snapshot).await;
            }
            if let Some(snapshot) = state.runtime.scale_idle(&service) {
                apply_service_scale(&state, &service, &snapshot).await;
            }
        }
        if let Err(error) = cleanup_orphan_runtime_resources(&state).await {
            tracing::warn!(
                error = %format!("{error:#}"),
                "failed to cleanup orphan serverless runtime resources"
            );
        }
    }
}

async fn apply_service_scale(
    state: &AppState,
    service: &ServerlessService,
    snapshot: &RuntimeSnapshot,
) {
    if let Err(error) = scale_service_pods(state, service, snapshot.active_instances).await {
        tracing::warn!(
            namespace = %crate::state::object_namespace(&service.metadata),
            name = %service.metadata.name,
            error = %format!("{error:#}"),
            "failed to scale serverless pods"
        );
    }
    patch_service_runtime_status(state, service, snapshot, None).await;
}
