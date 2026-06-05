mod handlers;
mod http;
mod informer;
mod python;
mod runtime;
mod scale;
mod state;
mod status;
mod workflow;

use anyhow::{Context, Result};
use clap::Parser;
use client_rs::{Client, ListParams, TypedApi};
use serverless_api::{EventTrigger, Function, Workflow};
use std::net::SocketAddr;
use std::time::Duration;

use crate::informer::{log_informer_event, wait_for_informers};
use crate::runtime::RuntimeRegistry;
use crate::scale::idle_scale_loop;
use crate::state::AppState;

#[derive(Debug, Parser)]
#[command(author, version, about = "Minik8s CRD-backed serverless controller")]
struct Args {
    #[arg(long)]
    api_server: Option<String>,

    #[arg(long, default_value = "0.0.0.0:8082")]
    bind: SocketAddr,

    #[arg(long, default_value = "python3")]
    python_bin: String,

    #[arg(long, default_value_t = 5)]
    idle_check_secs: u64,
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

    let functions = client_rs::spawn_informer::<Function, _>(
        TypedApi::<Function>::all(client.clone()),
        ListParams::default(),
        log_informer_event::<Function>("function"),
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
    wait_for_informers(&functions, &triggers, &workflows).await;

    let state = AppState {
        client,
        functions: functions.store.clone(),
        triggers: triggers.store.clone(),
        workflows: workflows.store.clone(),
        runtime: RuntimeRegistry::default(),
        python_bin: args.python_bin,
    };
    let idle_state = state.clone();
    tokio::spawn(async move {
        idle_scale_loop(idle_state, Duration::from_secs(args.idle_check_secs)).await;
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
