use apimachinery::{DEFAULT_NAMESPACE, Resource};
use client_rs::{Client, Store, TypedApi};
use serverless_api::{EventTrigger, Function, Workflow};

use crate::http::{HttpResult, api_error};
use crate::runtime::RuntimeRegistry;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) client: Client,
    pub(crate) functions: Store<Function>,
    pub(crate) triggers: Store<EventTrigger>,
    pub(crate) workflows: Store<Workflow>,
    pub(crate) runtime: RuntimeRegistry,
    pub(crate) python_bin: String,
}

pub(crate) async fn load_function(
    state: &AppState,
    namespace: &str,
    name: &str,
) -> HttpResult<Function> {
    if let Some(function) = find_namespaced(&state.functions, namespace, name) {
        return Ok(function);
    }
    TypedApi::<Function>::namespaced(state.client.clone(), namespace.to_string())
        .get(name)
        .await
        .map_err(|error| api_error(format!("Function {namespace}/{name} not found: {error}")))
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
