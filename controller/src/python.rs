use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use serverless_api::Function;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

use crate::state::{AppState, object_namespace};

pub(crate) async fn run_python_function(
    state: &AppState,
    function: &Function,
    input: Value,
) -> Result<Value> {
    let source = function
        .spec
        .source
        .inline
        .as_deref()
        .ok_or_else(|| anyhow!("function source is empty; upload source before invoking"))?;
    let handler = function.spec.handler.trim();
    let namespace = object_namespace(&function.metadata);
    let name = function.metadata.name.clone();
    let script = python_wrapper(source, handler, &namespace, &name);
    let mut child = tokio::process::Command::new(&state.python_bin)
        .arg("-c")
        .arg(script)
        .envs(&function.spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", state.python_bin))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.to_string().as_bytes())
            .await
            .context("failed to write function input")?;
    }
    let output = child.wait_with_output().await.context("function process")?;
    if !output.status.success() {
        return Err(anyhow!(
            "python exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout).context("function output is not UTF-8")?;
    if stdout.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(stdout.trim())
        .with_context(|| format!("function output is not JSON: {}", stdout.trim()))
}

fn python_wrapper(source: &str, handler: &str, namespace: &str, name: &str) -> String {
    format!(
        r#"{source}

import json as __minik8s_json
import sys as __minik8s_sys

__minik8s_raw = __minik8s_sys.stdin.read()
__minik8s_event = __minik8s_json.loads(__minik8s_raw) if __minik8s_raw.strip() else None
__minik8s_handler = globals().get({handler:?})
if not callable(__minik8s_handler):
    raise RuntimeError("handler {handler} is not callable")
__minik8s_context = {{"namespace": {namespace:?}, "name": {name:?}}}
__minik8s_result = __minik8s_handler(__minik8s_event, __minik8s_context)
__minik8s_sys.stdout.write(__minik8s_json.dumps(__minik8s_result))
"#
    )
}
