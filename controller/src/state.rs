use apimachinery::{DEFAULT_NAMESPACE, Resource};
use client_rs::{Client, Store, TypedApi};
use serverless_api::{EventTrigger, ServerlessService, Workflow};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::http::{HttpResult, api_error};
use crate::serving::RuntimeRegistry;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) client: Client,
    pub(crate) services: Store<ServerlessService>,
    pub(crate) triggers: Store<EventTrigger>,
    pub(crate) workflows: Store<Workflow>,
    pub(crate) runtime: RuntimeRegistry,
    pub(crate) runtime_pod_locks: RuntimePodLocks,
}

#[derive(Clone, Default)]
pub(crate) struct RuntimePodLocks {
    inner: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
}

impl RuntimePodLocks {
    pub(crate) async fn lock_for(&self, key: String) -> Arc<Mutex<()>> {
        let mut locks = self.inner.lock().await;
        locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

pub(crate) async fn load_service(
    state: &AppState,
    namespace: &str,
    name: &str,
) -> HttpResult<ServerlessService> {
    if let Some(service) = find_namespaced(&state.services, namespace, name) {
        return Ok(service);
    }
    TypedApi::<ServerlessService>::namespaced(state.client.clone(), namespace.to_string())
        .get(name)
        .await
        .map_err(|error| {
            api_error(format!(
                "ServerlessService {namespace}/{name} not found: {error}"
            ))
        })
}

pub(crate) async fn load_workflow(
    state: &AppState,
    namespace: &str,
    name: &str,
) -> HttpResult<Workflow> {
    if let Some(workflow) = find_namespaced(&state.workflows, namespace, name) {
        return Ok(workflow);
    }
    TypedApi::<Workflow>::namespaced(state.client.clone(), namespace.to_string())
        .get(name)
        .await
        .map_err(|error| api_error(format!("Workflow {namespace}/{name} not found: {error}")))
}

fn find_namespaced<R>(store: &Store<R>, namespace: &str, name: &str) -> Option<R>
where
    R: Resource,
{
    store
        .items()
        .into_iter()
        .find(|item| item.metadata().name == name && object_namespace(item.metadata()) == namespace)
}

pub(crate) fn object_namespace(metadata: &apimachinery::ObjectMeta) -> String {
    if metadata.namespace.trim().is_empty() {
        DEFAULT_NAMESPACE.to_string()
    } else {
        metadata.namespace.clone()
    }
}
