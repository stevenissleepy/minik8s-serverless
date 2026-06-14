use std::time::Duration;

use crate::serving::runtime::RuntimeSnapshot;
use crate::state::AppState;
use crate::status::patch_service_runtime_status;
use serverless_api::ServerlessService;

pub(crate) async fn idle_scale_loop(state: AppState, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        for service in state.services.items() {
            if let Some(snapshot) = state.runtime.ensure_min_instances(&service) {
                publish_scale_intent(&state, &service, &snapshot).await;
            }
            if let Some(snapshot) = state.runtime.scale_idle(&service) {
                publish_scale_intent(&state, &service, &snapshot).await;
            }
        }
    }
}

async fn publish_scale_intent(
    state: &AppState,
    service: &ServerlessService,
    snapshot: &RuntimeSnapshot,
) {
    patch_service_runtime_status(state, service, snapshot, None).await;
}
