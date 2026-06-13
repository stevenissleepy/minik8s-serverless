use anyhow::{Context, Result, anyhow, bail};
use apimachinery::{DEFAULT_NAMESPACE, ObjectMeta, TypeMeta};
use clap::{Parser, Subcommand};
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
struct FuncConfig {
    name: String,
    #[serde(default = "default_runtime")]
    runtime: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    scale: FuncScale,
    #[serde(default)]
    concurrency: FuncConcurrency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FuncScale {
    #[serde(rename = "minScale", default)]
    min_scale: u32,
    #[serde(rename = "maxScale", default = "default_max_scale")]
    max_scale: u32,
    #[serde(rename = "idleSeconds", default = "default_idle_seconds")]
    idle_seconds: u64,
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
struct FuncConcurrency {
    #[serde(default = "default_target_concurrency")]
    target: u32,
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
    if args.language.to_ascii_lowercase() != "python" {
        bail!("only python functions are currently supported");
    }
    let dir = PathBuf::from(&args.name);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::create_dir_all(dir.join("function")).with_context(|| "failed to create function/")?;
    fs::create_dir_all(dir.join("tests")).with_context(|| "failed to create tests/")?;
    fs::write(dir.join("function").join("__init__.py"), "")
        .with_context(|| "failed to write function/__init__.py")?;
    fs::write(
        dir.join("function").join("func.py"),
        python_template(&args.name),
    )
    .with_context(|| "failed to write function/func.py")?;
    fs::write(
        dir.join("tests").join("test_func.py"),
        python_test_template(),
    )
    .with_context(|| "failed to write tests/test_func.py")?;
    fs::write(dir.join("function").join("app.py"), python_app_template())
        .with_context(|| "failed to write function/app.py")?;
    fs::write(dir.join("requirements.txt"), "")
        .with_context(|| "failed to write requirements.txt")?;
    fs::write(dir.join("Dockerfile"), python_dockerfile(default_port()))
        .with_context(|| "failed to write Dockerfile")?;
    fs::write(dir.join("pyproject.toml"), python_pyproject(&args.name))
        .with_context(|| "failed to write pyproject.toml")?;
    let config = FuncConfig {
        name: args.name.clone(),
        runtime: "python".to_string(),
        port: default_port(),
        env: BTreeMap::new(),
        scale: FuncScale::default(),
        concurrency: FuncConcurrency::default(),
    };
    fs::write(dir.join("func.yaml"), serde_yaml::to_string(&config)?)
        .with_context(|| "failed to write func.yaml")?;
    println!("created python function template {}", dir.display());
    Ok(())
}

async fn deploy_function(args: DeployFuncArgs) -> Result<()> {
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
    if config.runtime.to_ascii_lowercase() != "python" {
        bail!("only python functions are currently supported");
    }
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

    build_python_function_image(&source_dir, &config, &image)?;
    if args.push {
        run_command(
            Command::new("docker").arg("push").arg(&image),
            "docker push",
        )?;
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

fn build_python_function_image(source_dir: &Path, config: &FuncConfig, image: &str) -> Result<()> {
    if !source_dir.join("function").join("func.py").exists() {
        bail!("{} is missing function/func.py", source_dir.display());
    }
    if !source_dir.join("function").join("app.py").exists() {
        bail!("{} is missing function/app.py", source_dir.display());
    }
    if !source_dir.join("Dockerfile").exists() {
        bail!("{} is missing Dockerfile", source_dir.display());
    }
    let context_dir = build_context_dir(&config.name)?;
    copy_dir(source_dir, &context_dir)?;
    let result = run_command(
        Command::new("docker")
            .arg("build")
            .arg("-t")
            .arg(image)
            .arg(&context_dir),
        "docker build",
    );
    let _ = fs::remove_dir_all(&context_dir);
    result
}

fn build_context_dir(name: &str) -> Result<PathBuf> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("minik8s-kn-{name}-{}-{now}", std::process::id()));
    fs::create_dir_all(&path).with_context(|| format!("failed to create {}", path.display()))?;
    Ok(path)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
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

fn run_command(command: &mut Command, description: &str) -> Result<()> {
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

fn python_template(name: &str) -> String {
    format!(
        r#"import json


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else None
        result = {{"function": "{name}", "event": event}}
        payload = json.dumps(result).encode("utf-8")
        await send({{
            "type": "http.response.start",
            "status": 200,
            "headers": [[b"content-type", b"application/json"]],
        }})
        await send({{
            "type": "http.response.body",
            "body": payload,
        }})

    def alive(self):
        return True, "alive"

    def ready(self):
        return True, "ready"
"#
    )
}

fn python_test_template() -> &'static str {
    r#"from function.func import new


def test_new():
    assert new() is not None
"#
}

fn python_app_template() -> &'static str {
    r#"import asyncio
import json
import os
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from function.func import new


PORT = int(os.environ.get("PORT", "8080"))


async def call_function(function, method, path, headers, body):
    response = {
        "status": 200,
        "headers": [(b"content-type", b"application/json")],
        "body": bytearray(),
    }
    received = False

    async def receive():
        nonlocal received
        if received:
            return {"type": "http.disconnect"}
        received = True
        return {"type": "http.request", "body": body, "more_body": False}

    async def send(message):
        if message["type"] == "http.response.start":
            response["status"] = int(message.get("status", 200))
            response["headers"] = message.get("headers", [])
        elif message["type"] == "http.response.body":
            response["body"].extend(message.get("body", b""))

    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": method,
        "scheme": "http",
        "path": "/" if path == "/invoke" else path,
        "raw_path": path.encode("utf-8"),
        "query_string": b"",
        "headers": headers,
        "server": ("0.0.0.0", PORT),
        "client": ("0.0.0.0", 0),
        "minik8s": {
            "namespace": os.environ.get("FUNCTION_NAMESPACE", "default"),
            "name": os.environ.get("FUNCTION_NAME", ""),
        },
    }
    result = function.handle(scope, receive, send)
    if asyncio.iscoroutine(result):
        await result
    return response


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path in ("/healthz", "/health/liveness", "/health/readiness"):
            self.send_json({"ok": True})
            return
        self.forward()

    def do_POST(self):
        self.forward()

    def forward(self):
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length) if length else b""
        headers = [
            (key.lower().encode("latin-1"), value.encode("latin-1"))
            for key, value in self.headers.items()
        ]
        try:
            response = asyncio.run(
                call_function(self.server.function, self.command, self.path, headers, body)
            )
            self.send_response(response["status"])
            payload = bytes(response["body"])
            has_length = False
            for name, value in response["headers"]:
                header = name.decode("latin-1")
                if header.lower() == "content-length":
                    has_length = True
                self.send_header(header, value.decode("latin-1"))
            if not has_length:
                self.send_header("content-length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except Exception as exc:
            self.send_json({"error": str(exc), "traceback": traceback.format_exc()}, status=500)

    def send_json(self, value, status=200):
        payload = json.dumps(value).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, format, *args):
        return


def main():
    server = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    server.function = new()
    server.serve_forever()


if __name__ == "__main__":
    main()
"#
}

fn python_pyproject(name: &str) -> String {
    format!(
        r#"[project]
name = "{name}"
version = "0.1.0"
requires-python = ">=3.12"
"#
    )
}

fn python_dockerfile(port: u16) -> String {
    format!(
        r#"FROM python:3.12-slim
WORKDIR /workspace
COPY . /workspace
RUN if [ -f requirements.txt ]; then pip install --no-cache-dir -r requirements.txt; fi
ENV PORT={port}
EXPOSE {port}
CMD ["python", "-m", "function.app"]
"#
    )
}

fn default_runtime() -> String {
    "python".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_target_concurrency() -> u32 {
    10
}

fn default_max_scale() -> u32 {
    10
}

fn default_idle_seconds() -> u64 {
    60
}
