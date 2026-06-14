mod eventing;
mod handlers;
mod http;
mod informer;
mod serving;
mod state;
mod status;

use anyhow::{Context, Result};
use apimachinery::ObjectRef;
use clap::Parser;
use client_rs::{Client, InformerEvent, ListParams, Store, TypedApi};
use serverless_api::{EventSource, EventTrigger, Revision, ServerlessService, Workflow};
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::eventing::event_source_loop;
use crate::informer::{log_informer_event, wait_for_informers};
use crate::serving::{RuntimeRegistry, idle_scale_loop, reconcile_loop};
use crate::state::AppState;

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Minik8s serverless-controller resource reconciler"
)]
struct ServingControllerArgs {
    #[arg(long)]
    api_server: Option<String>,

    #[arg(long, default_value = "0.0.0.0:8083")]
    bind: SocketAddr,

    #[arg(long, default_value_t = 2)]
    reconcile_secs: u64,
}

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Minik8s serverless Activator data-plane gateway"
)]
struct ActivatorArgs {
    #[arg(long)]
    api_server: Option<String>,

    #[arg(long, default_value = "0.0.0.0:8082")]
    bind: SocketAddr,

    #[arg(long, default_value_t = 5)]
    idle_check_secs: u64,
}

pub fn controller_main() {
    run_process(serving_controller_run());
}

pub fn activator_main() {
    run_process(activator_run());
}

fn run_process(run: impl Future<Output = Result<()>>) {
    logger::init_tracing();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(error) => {
            tracing::error!(error = %error, "failed to build tokio runtime");
            std::process::exit(1);
        }
    };
    if let Err(error) = runtime.block_on(run) {
        tracing::error!(error = %format!("{error:#}"), "exiting");
        std::process::exit(1);
    }
}

async fn serving_controller_run() -> Result<()> {
    let args = ServingControllerArgs::parse();
    let client = build_client(args.api_server.as_deref())?;
    let (reconcile_tx, reconcile_rx) = mpsc::unbounded_channel();
    let (services, revisions, triggers, workflows, sources) =
        spawn_resource_informers(&client, Some(reconcile_tx));
    wait_for_informers(&services, &revisions, &triggers, &workflows, &sources).await;

    let state = AppState {
        client,
        services: services.store.clone(),
        triggers: triggers.store.clone(),
        workflows: workflows.store.clone(),
        sources: sources.store.clone(),
        runtime: RuntimeRegistry::default(),
        runtime_pod_locks: Default::default(),
    };
    let reconcile_state = state.clone();
    tokio::spawn(async move {
        reconcile_loop(
            reconcile_state,
            Duration::from_secs(args.reconcile_secs),
            reconcile_rx,
        )
        .await;
    });

    let app = handlers::health_routes().with_state(state);
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    tracing::info!(addr = %args.bind, "serverless-controller health endpoint listening");
    axum::serve(listener, app)
        .await
        .context("serverless-controller stopped")
}

async fn activator_run() -> Result<()> {
    let args = ActivatorArgs::parse();
    let client = build_client(args.api_server.as_deref())?;
    let (services, revisions, triggers, workflows, sources) =
        spawn_resource_informers(&client, None);
    wait_for_informers(&services, &revisions, &triggers, &workflows, &sources).await;

    let state = AppState {
        client,
        services: services.store.clone(),
        triggers: triggers.store.clone(),
        workflows: workflows.store.clone(),
        sources: sources.store.clone(),
        runtime: RuntimeRegistry::default(),
        runtime_pod_locks: Default::default(),
    };
    let idle_state = state.clone();
    tokio::spawn(async move {
        idle_scale_loop(idle_state, Duration::from_secs(args.idle_check_secs)).await;
    });
    let source_state = state.clone();
    tokio::spawn(async move {
        event_source_loop(source_state, Duration::from_secs(1)).await;
    });

    let app = handlers::routes().with_state(state);
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    tracing::info!(addr = %args.bind, "serverless activator listening");
    axum::serve(listener, app)
        .await
        .context("serverless activator stopped")
}

fn build_client(api_server: Option<&str>) -> Result<Client> {
    match api_server {
        Some(endpoint) => Ok(Client::new(endpoint)?),
        None => Ok(Client::from_env()?),
    }
}

fn spawn_resource_informers(
    client: &Client,
    service_reconcile_tx: Option<mpsc::UnboundedSender<ObjectRef>>,
) -> (
    client_rs::InformerHandle<ServerlessService>,
    client_rs::InformerHandle<Revision>,
    client_rs::InformerHandle<EventTrigger>,
    client_rs::InformerHandle<Workflow>,
    client_rs::InformerHandle<EventSource>,
) {
    let services = client_rs::spawn_informer::<ServerlessService, _>(
        TypedApi::<ServerlessService>::all(client.clone()),
        ListParams::default(),
        service_event_handler(service_reconcile_tx),
    );
    let revisions = client_rs::spawn_informer::<Revision, _>(
        TypedApi::<Revision>::all(client.clone()),
        ListParams::default(),
        log_informer_event::<Revision>("revision"),
    );
    let triggers = client_rs::spawn_informer::<EventTrigger, _>(
        TypedApi::<EventTrigger>::all(client.clone()),
        ListParams::default(),
        log_informer_event::<EventTrigger>("eventtrigger"),
    );
    let workflows = client_rs::spawn_informer::<Workflow, _>(
        TypedApi::<Workflow>::all(client.clone()),
        ListParams::default(),
        log_informer_event::<Workflow>("workflow"),
    );
    let sources = client_rs::spawn_informer::<EventSource, _>(
        TypedApi::<EventSource>::all(client.clone()),
        ListParams::default(),
        log_informer_event::<EventSource>("eventsource"),
    );
    (services, revisions, triggers, workflows, sources)
}

fn service_event_handler(
    reconcile_tx: Option<mpsc::UnboundedSender<ObjectRef>>,
) -> impl FnMut(InformerEvent, &Store<ServerlessService>) + Send + 'static {
    let mut log = log_informer_event::<ServerlessService>("serverlessservice");
    move |event, store| {
        let key = match &event {
            InformerEvent::Added(key)
            | InformerEvent::Modified(key)
            | InformerEvent::Deleted(key) => Some(key.clone()),
            InformerEvent::Synced | InformerEvent::Error(_) => None,
        };
        log(event, store);
        if let (Some(tx), Some(key)) = (&reconcile_tx, key) {
            let _ = tx.send(key);
        }
    }
}
