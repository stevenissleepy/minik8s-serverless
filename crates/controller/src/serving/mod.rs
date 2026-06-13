pub(crate) mod runtime;
pub(crate) mod runtime_pods;

mod handlers;
mod scale;
mod workflow;

pub(crate) use handlers::{invoke_function_inner, invoke_workflow_inner, routes};
pub(crate) use runtime::RuntimeRegistry;
pub(crate) use scale::idle_scale_loop;

use std::time::Duration;

use crate::serving::runtime_pods::reconcile_all_services;
use crate::state::AppState;

pub(crate) async fn reconcile_loop(state: AppState, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        reconcile_all_services(&state).await;
    }
}
