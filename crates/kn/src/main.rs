use anyhow::{Context, Result, anyhow, bail};
use apimachinery::{DEFAULT_NAMESPACE, ObjectMeta, TypeMeta};
use clap::{ArgAction, Parser, Subcommand};
use client_rs::{Client, TypedApi};
use serde::{Deserialize, Serialize};
use serverless_api::{
    ServerlessConcurrency, ServerlessScale, ServerlessService, ServerlessServiceSpec,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

mod runtime;

use runtime::FunctionRuntime;

#[derive(Debug, Parser)]
#[command(author, version, about = "Minik8s Knative-like CLI")]
struct Cli {
    #[command(subcommand)]
    command: KnCommand,
}

#[derive(Debug, Subcommand)]
enum KnCommand {
    #[command(name = "func", alias = "function")]
    Func(FuncArgs),
    #[command(name = "service", alias = "svc")]
    Service(ServiceArgs),
}

#[derive(Debug, Parser)]
struct FuncArgs {
    #[command(subcommand)]
    command: FuncCommand,
}

#[derive(Debug, Subcommand)]
enum FuncCommand {
    /// Create a local function template directory.
    Create(CreateFuncArgs),
    /// Build, push, and deploy the current function directory.
    Deploy(DeployFuncArgs),
}

#[derive(Debug, Parser)]
struct CreateFuncArgs {
    #[arg(short = 'l', long = "language", default_value = "python")]
    language: String,
    name: String,
}

#[derive(Debug, Parser)]
struct DeployFuncArgs {
    #[arg(long)]
    registry: Option<String>,
    #[arg(long)]
    image: Option<String>,
    #[arg(long)]
    namespace: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    api_server: Option<String>,
    #[arg(long, default_value_t = true)]
    push: bool,
    #[arg(long = "no-push", action = ArgAction::SetTrue)]
    no_push: bool,
    #[arg(long = "no-deploy", action = ArgAction::SetTrue)]
    no_deploy: bool,
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[derive(Debug, Parser)]
struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Create or update a ServerlessService from an existing image.
    Create(CreateServiceArgs),
}

#[derive(Debug, Parser)]
struct CreateServiceArgs {
    name: String,
    #[arg(long)]
    image: String,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long)]
    namespace: Option<String>,
    #[arg(long)]
    api_server: Option<String>,
    #[arg(long, default_value_t = 0)]
    min_scale: u32,
    #[arg(long, default_value_t = 10)]
    max_scale: u32,
    #[arg(long, default_value_t = 10)]
    target_concurrency: u32,
    #[arg(long = "env")]
    env: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FuncConfig {
    pub(crate) name: String,
    #[serde(default = "default_runtime")]
    pub(crate) runtime: String,
    #[serde(default = "default_port")]
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) scale: FuncScale,
    #[serde(default)]
    pub(crate) concurrency: FuncConcurrency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FuncScale {
    #[serde(rename = "minScale", default)]
    pub(crate) min_scale: u32,
    #[serde(rename = "maxScale", default = "default_max_scale")]
    pub(crate) max_scale: u32,
    #[serde(rename = "idleSeconds", default = "default_idle_seconds")]
    pub(crate) idle_seconds: u64,
}

impl Default for FuncScale {
    fn default() -> Self {
        Self {
            min_scale: 0,
            max_scale: default_max_scale(),
            idle_seconds: default_idle_seconds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FuncConcurrency {
    #[serde(default = "default_target_concurrency")]
    pub(crate) target: u32,
}

impl Default for FuncConcurrency {
    fn default() -> Self {
        Self {
            target: default_target_concurrency(),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    match Cli::parse().command {
        KnCommand::Func(args) => match args.command {
            FuncCommand::Create(args) => create_function(args),
            FuncCommand::Deploy(args) => deploy_function(args).await,
        },
        KnCommand::Service(args) => match args.command {
            ServiceCommand::Create(args) => create_service(args).await,
        },
    }
}

fn create_function(args: CreateFuncArgs) -> Result<()> {
    let runtime = FunctionRuntime::from_language(&args.language)?;
    let dir = PathBuf::from(&args.name);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    runtime.create(&dir, &args.name)?;
    println!(
        "created {} function template {}",
        runtime.name(),
        dir.display()
    );
    Ok(())
}

async fn deploy_function(args: DeployFuncArgs) -> Result<()> {
    let push = args.push && !args.no_push;
    let deploy = !args.no_deploy;
    let source_dir = args.path.canonicalize().with_context(|| {
        format!(
            "failed to resolve function directory {}",
            args.path.display()
        )
    })?;
    if !source_dir.is_dir() {
        bail!("{} is not a directory", source_dir.display());
    }
    let mut config = read_func_config(&source_dir)?;
    if let Some(name) = args.name {
        config.name = name;
    }
    let runtime = FunctionRuntime::from_config(&config.runtime)?;
    let image = match args.image {
        Some(image) => image,
        None => {
            let registry = args
                .registry
                .as_deref()
                .ok_or_else(|| anyhow!("--registry is required unless --image is set"))?;
            format!("{}/{}:latest", registry.trim_end_matches('/'), config.name)
        }
    };

    runtime.build_image(&source_dir, &config, &image)?;
    if push {
        run_command(
            Command::new("docker").arg("push").arg(&image),
            "docker push",
        )?;
    }
    if !deploy {
        println!("function image {} built", image);
        return Ok(());
    }

    let namespace = args.namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let client = client(args.api_server.as_deref())?;
    apply_serverless_service(&client, namespace, &config, &image).await?;
    println!(
        "serverlessservice.serverless.minik8s.io/{} deployed with image {}",
        config.name, image
    );
    Ok(())
}

async fn create_service(args: CreateServiceArgs) -> Result<()> {
    let namespace = args.namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
    let client = client(args.api_server.as_deref())?;
    let env = parse_env(args.env)?;
    let service = ServerlessService {
        types: TypeMeta::for_resource::<ServerlessService>(),
        metadata: ObjectMeta {
            name: args.name.clone(),
            namespace: namespace.to_string(),
            ..Default::default()
        },
        spec: ServerlessServiceSpec {
            image: args.image,
            port: args.port,
            env,
            scale: ServerlessScale {
                min_scale: args.min_scale,
                max_scale: args.max_scale,
                idle_seconds: default_idle_seconds(),
            },
            concurrency: ServerlessConcurrency {
                target: args.target_concurrency,
            },
        },
        status: Default::default(),
    };
    TypedApi::<ServerlessService>::namespaced(client, namespace.to_string())
        .apply(&service)
        .await
        .with_context(|| format!("failed to apply ServerlessService {}", args.name))?;
    println!(
        "serverlessservice.serverless.minik8s.io/{} configured",
        args.name
    );
    Ok(())
}

async fn apply_serverless_service(
    client: &Client,
    namespace: &str,
    config: &FuncConfig,
    image: &str,
) -> Result<()> {
    let service = ServerlessService {
        types: TypeMeta::for_resource::<ServerlessService>(),
        metadata: ObjectMeta {
            name: config.name.clone(),
            namespace: namespace.to_string(),
            ..Default::default()
        },
        spec: ServerlessServiceSpec {
            image: image.to_string(),
            port: config.port,
            env: config.env.clone(),
            scale: ServerlessScale {
                min_scale: config.scale.min_scale,
                max_scale: config.scale.max_scale,
                idle_seconds: config.scale.idle_seconds,
            },
            concurrency: ServerlessConcurrency {
                target: config.concurrency.target,
            },
        },
        status: Default::default(),
    };
    TypedApi::<ServerlessService>::namespaced(client.clone(), namespace.to_string())
        .apply(&service)
        .await
        .with_context(|| format!("failed to apply ServerlessService {}", config.name))?;
    Ok(())
}

fn read_func_config(source_dir: &Path) -> Result<FuncConfig> {
    let path = source_dir.join("func.yaml");
    if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        return serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()));
    }
    let name = source_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow!(
                "failed to infer function name from {}",
                source_dir.display()
            )
        })?;
    Ok(FuncConfig {
        name: name.to_string(),
        runtime: "python".to_string(),
        port: default_port(),
        env: BTreeMap::new(),
        scale: FuncScale::default(),
        concurrency: FuncConcurrency::default(),
    })
}

pub(crate) fn build_context_dir(name: &str) -> Result<PathBuf> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("minik8s-kn-{name}-{}-{now}", std::process::id()));
    fs::create_dir_all(&path).with_context(|| format!("failed to create {}", path.display()))?;
    Ok(path)
}

pub(crate) fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if name == ".git" || name == "target" {
                continue;
            }
            fs::create_dir_all(&dst_path)
                .with_context(|| format!("failed to create {}", dst_path.display()))?;
            copy_dir(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

pub(crate) fn run_command(command: &mut Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to start {description}"))?;
    if !status.success() {
        bail!("{description} failed with status {status}");
    }
    Ok(())
}

fn parse_env(items: Vec<String>) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    for item in items {
        let (name, value) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--env must use NAME=VALUE, got {item}"))?;
        if name.trim().is_empty() {
            bail!("--env name must not be empty");
        }
        env.insert(name.to_string(), value.to_string());
    }
    Ok(env)
}

fn client(api_server: Option<&str>) -> Result<Client> {
    match api_server {
        Some(endpoint) => Ok(Client::new(endpoint)?),
        None => Ok(Client::from_env()?),
    }
}

pub(crate) fn default_runtime() -> String {
    "python".to_string()
}

pub(crate) fn default_port() -> u16 {
    8080
}

pub(crate) fn default_target_concurrency() -> u32 {
    10
}

pub(crate) fn default_max_scale() -> u32 {
    10
}

pub(crate) fn default_idle_seconds() -> u64 {
    60
}
