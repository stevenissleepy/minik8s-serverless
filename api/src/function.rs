use anyhow::{Result, anyhow};
use apimachinery::{HasStatus, ObjectMeta, Resource, TypeMeta, Validatable};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::{DEFAULT_HANDLER, GROUP, VERSION};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Function {
    #[serde(flatten)]
    pub types: TypeMeta,
    pub metadata: ObjectMeta,
    pub spec: FunctionSpec,
    #[serde(default)]
    pub status: FunctionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FunctionRuntime {
    Python,
}

impl Default for FunctionRuntime {
    fn default() -> Self {
        Self::Python
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FunctionSpec {
    #[serde(default)]
    pub runtime: FunctionRuntime,
    #[serde(default)]
    pub source: FunctionSource,
    #[serde(default = "default_handler")]
    pub handler: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub concurrency: FunctionConcurrency,
    #[serde(default)]
    pub scale: FunctionScale,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionConcurrency {
    #[serde(default = "default_target_concurrency")]
    pub target: u32,
}

impl Default for FunctionConcurrency {
    fn default() -> Self {
        Self {
            target: default_target_concurrency(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionScale {
    #[serde(rename = "minInstances", default)]
    pub min_instances: u32,
    #[serde(rename = "maxInstances", default = "default_max_instances")]
    pub max_instances: u32,
    #[serde(rename = "idleSeconds", default = "default_idle_seconds")]
    pub idle_seconds: u64,
}

impl Default for FunctionScale {
    fn default() -> Self {
        Self {
            min_instances: 0,
            max_instances: default_max_instances(),
            idle_seconds: default_idle_seconds(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FunctionStatus {
    #[serde(default)]
    pub ready: bool,
    #[serde(rename = "activeInstances", default)]
    pub active_instances: u32,
    #[serde(rename = "inFlight", default)]
    pub in_flight: u32,
    #[serde(
        rename = "lastInvokedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_invoked_at: Option<DateTime<Utc>>,
    #[serde(rename = "lastError", default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Resource for Function {
    type DynamicType = ();

    fn kind(_: &()) -> Cow<'_, str> {
        "Function".into()
    }

    fn plural(_: &()) -> Cow<'_, str> {
        "functions".into()
    }

    fn group(_: &()) -> Cow<'_, str> {
        GROUP.into()
    }

    fn version(_: &()) -> Cow<'_, str> {
        VERSION.into()
    }

    fn metadata(&self) -> &ObjectMeta {
        &self.metadata
    }

    fn metadata_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

impl HasStatus for Function {
    type Status = FunctionStatus;

    fn status(&self) -> &Self::Status {
        &self.status
    }

    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

impl Validatable for Function {
    fn validate_spec(&self) -> Result<()> {
        if self.spec.handler.trim().is_empty() {
            return Err(anyhow!("function spec.handler is required"));
        }
        if self.spec.concurrency.target == 0 {
            return Err(anyhow!(
                "function spec.concurrency.target must be greater than 0"
            ));
        }
        if self.spec.scale.max_instances == 0 {
            return Err(anyhow!(
                "function spec.scale.maxInstances must be greater than 0"
            ));
        }
        if self.spec.scale.min_instances > self.spec.scale.max_instances {
            return Err(anyhow!(
                "function spec.scale.minInstances must not exceed maxInstances"
            ));
        }
        Ok(())
    }
}

fn default_handler() -> String {
    DEFAULT_HANDLER.to_string()
}

fn default_target_concurrency() -> u32 {
    10
}

fn default_max_instances() -> u32 {
    10
}

fn default_idle_seconds() -> u64 {
    60
}
