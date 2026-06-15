use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::{
    FuncConcurrency, FuncConfig, FuncScale, build_context_dir, copy_dir, default_idle_seconds,
    default_max_scale, default_port, default_target_concurrency, run_command,
};

pub(crate) fn create(dir: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(dir.join("function"))
        .with_context(|| format!("failed to create {}", dir.join("function").display()))?;
    fs::create_dir_all(dir.join("tests"))
        .with_context(|| format!("failed to create {}", dir.join("tests").display()))?;
    fs::write(dir.join("function").join("__init__.py"), "")?;
    fs::write(dir.join("function").join("func.py"), python_template(name))?;
    fs::write(
        dir.join("tests").join("test_func.py"),
        python_test_template(),
    )?;
    fs::write(dir.join("function").join("app.py"), python_app_template())?;
    fs::write(dir.join("requirements.txt"), "")?;
    fs::write(dir.join("Dockerfile"), python_dockerfile(default_port()))?;
    fs::write(dir.join("pyproject.toml"), python_pyproject(name))?;
    let config = FuncConfig {
        name: name.to_string(),
        runtime: "python".to_string(),
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
