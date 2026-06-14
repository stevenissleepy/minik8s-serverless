use apimachinery::Resource;
use client_rs::{InformerEvent, InformerHandle, Store};
use serverless_api::{EventSource, EventTrigger, Revision, ServerlessService, Workflow};
use std::time::Duration;

pub(crate) fn log_informer_event<K>(
    kind: &'static str,
) -> impl FnMut(InformerEvent, &Store<K>) + Send + 'static
where
    K: Resource,
{
    move |event, _| match event {
        InformerEvent::Error(error) => {
            tracing::warn!(kind, error = %error, "serverless informer error")
        }
        InformerEvent::Synced => tracing::info!(kind, "serverless informer synced"),
        _ => {}
    }
}

pub(crate) async fn wait_for_informers(
    services: &InformerHandle<ServerlessService>,
    revisions: &InformerHandle<Revision>,
    triggers: &InformerHandle<EventTrigger>,
    workflows: &InformerHandle<Workflow>,
    sources: &InformerHandle<EventSource>,
) {
    while !services.has_synced()
        || !revisions.has_synced()
        || !triggers.has_synced()
        || !workflows.has_synced()
        || !sources.has_synced()
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
