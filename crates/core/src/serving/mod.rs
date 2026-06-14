pub(crate) mod runtime;
pub(crate) mod runtime_pods;

mod handlers;
mod scale;
mod workflow;

pub(crate) use handlers::{invoke_function_inner, invoke_workflow_inner, routes};
pub(crate) use runtime::RuntimeRegistry;
pub(crate) use scale::idle_scale_loop;

use apimachinery::ObjectRef;
use client_rs::WorkQueue;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::serving::runtime_pods::{reconcile_all_services, reconcile_service_by_key};
use crate::state::AppState;

pub(crate) async fn reconcile_loop(
    state: AppState,
    interval: Duration,
    mut events: mpsc::UnboundedReceiver<ObjectRef>,
) {
    reconcile_all_services(&state).await;
    let interval = interval.max(Duration::from_millis(1));
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await;
    let mut events_open = true;

    loop {
        tokio::select! {
            maybe_key = events.recv(), if events_open => {
                match maybe_key {
                    Some(key) => reconcile_event_batch(&state, key, &mut events).await,
                    None => events_open = false,
                }
            }
            _ = ticker.tick() => {
                reconcile_all_services(&state).await;
            }
        }
    }
}

async fn reconcile_event_batch(
    state: &AppState,
    first: ObjectRef,
    events: &mut mpsc::UnboundedReceiver<ObjectRef>,
) {
    let mut queue = WorkQueue::default();
    queue.add(first);
    while let Ok(key) = events.try_recv() {
        queue.add(key);
    }
    while let Some(key) = queue.pop() {
        reconcile_service_by_key(state, &key).await;
    }
}
