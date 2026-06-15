use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::{
    FuncConcurrency, FuncConfig, FuncScale, build_context_dir, copy_dir, default_idle_seconds,
    default_max_scale, default_port, default_target_concurrency, run_command,
};

pub(crate) fn create(dir: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(dir.join("src"))
        .with_context(|| format!("failed to create {}", dir.join("src").display()))?;
    fs::write(dir.join("Cargo.toml"), cargo_toml(name))?;
    fs::write(dir.join("src").join("main.rs"), runtime_main())?;
    fs::write(dir.join("src").join("function.rs"), function_template())?;
    fs::write(dir.join("Dockerfile"), dockerfile(name, default_port()))?;
    let config = FuncConfig {
        name: name.to_string(),
        runtime: "rust".to_string(),
        port: default_port(),
        env: BTreeMap::new(),
        scale: FuncScale {
            min_scale: 0,
            max_scale: default_max_scale(),
            idle_seconds: default_idle_seconds(),
        },
        concurrency: FuncConcurrency {
            target: default_target_concurrency(),
        },
    };
    fs::write(dir.join("func.yaml"), serde_yaml::to_string(&config)?)?;
    Ok(())
}

pub(crate) fn build_image(source_dir: &Path, config: &FuncConfig, image: &str) -> Result<()> {
    if !source_dir.join("Cargo.toml").exists() {
        bail!("{} is missing Cargo.toml", source_dir.display());
    }
    if !source_dir.join("src").join("main.rs").exists() {
        bail!("{} is missing src/main.rs", source_dir.display());
    }

    let build_dir = build_context_dir(&format!("{}-rust-build", config.name))?;
    let image_dir = build_context_dir(&format!("{}-rust-image", config.name))?;
    let result = (|| {
        copy_dir(source_dir, &build_dir)?;
        let bin_name = cargo_binary_name(&build_dir)?;
        let target = linux_musl_target()?;
        ensure_rust_target_installed(target)?;
        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("--release")
                .arg("--target")
                .arg(target)
                .current_dir(&build_dir),
            "cargo build --release --target",
        )?;

        let binary = build_dir
            .join("target")
            .join(target)
            .join("release")
            .join(&bin_name);
        if !binary.exists() {
            bail!(
                "cargo build did not produce expected binary {}",
                binary.display()
            );
        }
        let image_binary = image_dir.join("function");
        fs::copy(&binary, &image_binary).with_context(|| {
            format!(
                "failed to copy {} to {}",
                binary.display(),
                image_binary.display()
            )
        })?;
        make_executable(&image_binary)?;
        fs::write(
            image_dir.join("Dockerfile"),
            runtime_image_dockerfile(config.port),
        )?;
        run_command(
            Command::new("docker")
                .arg("build")
                .arg("-t")
                .arg(image)
                .arg(&image_dir),
            "docker build",
        )
    })();
    let _ = fs::remove_dir_all(&build_dir);
    let _ = fs::remove_dir_all(&image_dir);
    result
}

fn cargo_binary_name(source_dir: &Path) -> Result<String> {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version=1")
        .current_dir(source_dir)
        .output()
        .with_context(|| format!("failed to run cargo metadata in {}", source_dir.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "cargo metadata failed with status {}: {stderr}",
            output.status
        );
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "failed to parse cargo metadata for {}",
            source_dir.display()
        )
    })?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("cargo metadata did not report packages"))?;
    let root_package = if let Some(root_package_id) =
        metadata.get("root_package").and_then(Value::as_str)
    {
        packages
            .iter()
            .find(|package| package.get("id").and_then(Value::as_str) == Some(root_package_id))
            .ok_or_else(|| anyhow!("cargo metadata root package {root_package_id} was not found"))?
    } else {
        let expected_manifest = source_dir.join("Cargo.toml").canonicalize()?;
        packages
            .iter()
            .find(|package| {
                package
                    .get("manifest_path")
                    .and_then(Value::as_str)
                    .and_then(|path| Path::new(path).canonicalize().ok())
                    .as_ref()
                    == Some(&expected_manifest)
            })
            .or_else(|| {
                if packages.len() == 1 {
                    packages.first()
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("cargo metadata did not report a package for Cargo.toml"))?
    };
    let targets = root_package
        .get("targets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("cargo metadata root package has no targets"))?;
    for target in targets {
        let is_bin = target
            .get("kind")
            .and_then(Value::as_array)
            .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("bin")));
        if is_bin {
            return target
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| anyhow!("cargo binary target is missing name"));
        }
    }
    bail!("cargo package does not define a binary target")
}

fn linux_musl_target() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64-unknown-linux-musl"),
        "aarch64" => Ok("aarch64-unknown-linux-musl"),
        arch => {
            bail!("unsupported Rust function image architecture {arch}; supported: x86_64, aarch64")
        }
    }
}

fn ensure_rust_target_installed(target: &str) -> Result<()> {
    let output = Command::new("rustup")
        .arg("target")
        .arg("list")
        .arg("--installed")
        .output();
    let Ok(output) = output else {
        return Ok(());
    };
    if !output.status.success() {
        return Ok(());
    }
    let installed = String::from_utf8_lossy(&output.stdout);
    if installed.lines().any(|line| line.trim() == target) {
        return Ok(());
    }
    bail!(
        "Rust target {target} is required to build lightweight scratch function images; install it with `rustup target add {target}`"
    )
}

fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to chmod {}", path.display()))?;
    }
    Ok(())
}

fn cargo_toml(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
serde_json = "1"

[workspace]
"#
    )
}

fn function_template() -> &'static str {
    r#"use crate::FunctionContext;
use serde_json::{json, Value};

pub fn handle(event: Value, ctx: &FunctionContext) -> Value {
    json!({
        "function": ctx.name.clone(),
        "event": event,
    })
}
"#
}

fn runtime_main() -> &'static str {
    r#"use serde_json::{json, Value};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

mod function;

#[derive(Clone)]
pub struct FunctionContext {
    pub namespace: String,
    pub name: String,
}

fn main() {
    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{port}");
    let ctx = Arc::new(FunctionContext {
        namespace: env::var("FUNCTION_NAMESPACE").unwrap_or_else(|_| "default".to_string()),
        name: env::var("FUNCTION_NAME").unwrap_or_default(),
    });
    let listener = TcpListener::bind(&addr).unwrap_or_else(|error| {
        panic!("failed to bind {addr}: {error}");
    });
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let ctx = Arc::clone(&ctx);
                thread::spawn(move || handle_connection(stream, &ctx));
            }
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }
}

fn handle_connection(mut stream: TcpStream, ctx: &FunctionContext) {
    if let Err(error) = serve(&mut stream, ctx) {
        let body = json!({"error": error.to_string()}).to_string();
        let _ = write_response(&mut stream, 500, "application/json", body.as_bytes());
    }
}

fn serve(stream: &mut TcpStream, ctx: &FunctionContext) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().unwrap_or(0);
            }
        }
    }
    if method == "GET" && matches!(path, "/healthz" | "/health/liveness" | "/health/readiness") {
        return write_json(stream, 200, &json!({"ok": true}));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let event = if body.iter().any(|byte| !byte.is_ascii_whitespace()) {
        serde_json::from_slice::<Value>(&body)?
    } else {
        Value::Null
    };
    let result = function::handle(event, ctx);
    write_json(stream, 200, &result)
}

fn write_json(
    stream: &mut TcpStream,
    status: u16,
    value: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let body = serde_json::to_vec(value)?;
    write_response(stream, status, "application/json", &body)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}
"#
}

fn dockerfile(name: &str, port: u16) -> String {
    format!(
        r#"FROM rust:1.88-slim AS build
WORKDIR /workspace
RUN rustup target add x86_64-unknown-linux-musl
COPY . /workspace
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM scratch
COPY --from=build /workspace/target/x86_64-unknown-linux-musl/release/{name} /function
ENV PORT={port}
EXPOSE {port}
CMD ["/function"]
"#
    )
}

fn runtime_image_dockerfile(port: u16) -> String {
    format!(
        r#"FROM scratch
COPY function /function
ENV PORT={port}
EXPOSE {port}
CMD ["/function"]
"#
    )
}
