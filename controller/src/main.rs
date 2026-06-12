mod eventing;
mod handlers;
mod http;
mod informer;
mod serving;
mod state;
mod status;

use anyhow::{Context, Result};
use clap::Parser;
use client_rs::{Client, ListParams, TypedApi};
use serverless_api::{EventTrigger, Revision, ServerlessService, Workflow};
use std::net::SocketAddr;
use std::time::Duration;

use crate::informer::{log_informer_event, wait_for_informers};
use crate::serving::{RuntimeRegistry, idle_scale_loop, reconcile_loop};
use crate::state::AppState;

#[derive(Debug, Parser)]
#[command(author, version, about = "Minik8s CRD-backed serverless controller")]
struct Args {
    #[arg(long)]
    api_server: Option<String>,

    #[arg(long, default_value = "0.0.0.0:8082")]
    bind: SocketAddr,

    #[arg(long, default_value_t = 5)]
    idle_check_secs: u64,

    #[arg(long, default_value_t = 2)]
    reconcile_secs: u64,
}

fn main() {
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
    if let Err(error) = runtime.block_on(run()) {
        tracing::error!(error = %format!("{error:#}"), "exiting");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args = Args::parse();
    let client = match args.api_server.as_deref() {
        Some(endpoint) => Client::new(endpoint)?,
        None => Client::from_env()?,
    };

    let services = client_rs::spawn_informer::<ServerlessService, _>(
        TypedApi::<ServerlessService>::all(client.clone()),
        ListParams::default(),
        log_informer_event::<ServerlessService>("serverlessservice"),
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
    wait_for_informers(&services, &revisions, &triggers, &workflows).await;

    let state = AppState {
        client,
        services: services.store.clone(),
        triggers: triggers.store.clone(),
        workflows: workflows.store.clone(),
        runtime: RuntimeRegistry::default(),
        runtime_pod_locks: Default::default(),
    };
    let idle_state = state.clone();
    tokio::spawn(async move {
        idle_scale_loop(idle_state, Duration::from_secs(args.idle_check_secs)).await;
    });
    let reconcile_state = state.clone();
    tokio::spawn(async move {
        reconcile_loop(reconcile_state, Duration::from_secs(args.reconcile_secs)).await;
    });

    let app = handlers::routes().with_state(state);
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    tracing::info!(addr = %args.bind, "serverless controller listening");
    axum::serve(listener, app)
        .await
        .context("serverless controller stopped")
}
