use anyhow::{Result, anyhow};
use apimachinery::{HasStatus, ObjectMeta, Resource, TypeMeta, Validatable};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::{GROUP, VERSION};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerlessService {
    #[serde(flatten)]
    pub types: TypeMeta,
    pub metadata: ObjectMeta,
    pub spec: ServerlessServiceSpec,
    #[serde(default)]
    pub status: ServerlessServiceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerlessServiceSpec {
    pub image: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub scale: ServerlessScale,
    #[serde(default)]
    pub concurrency: ServerlessConcurrency,
}

impl Default for ServerlessServiceSpec {
    fn default() -> Self {
        Self {
            image: String::new(),
            port: default_port(),
            env: BTreeMap::new(),
            scale: ServerlessScale::default(),
            concurrency: ServerlessConcurrency::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerlessConcurrency {
    #[serde(default = "default_target_concurrency")]
    pub target: u32,
}

impl Default for ServerlessConcurrency {
    fn default() -> Self {
        Self {
            target: default_target_concurrency(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerlessScale {
    #[serde(rename = "minScale", alias = "minInstances", default)]
    pub min_scale: u32,
    #[serde(
        rename = "maxScale",
        alias = "maxInstances",
        default = "default_max_scale"
    )]
    pub max_scale: u32,
    #[serde(rename = "idleSeconds", default = "default_idle_seconds")]
    pub idle_seconds: u64,
}

impl Default for ServerlessScale {
    fn default() -> Self {
        Self {
            min_scale: 0,
            max_scale: default_max_scale(),
            idle_seconds: default_idle_seconds(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerlessServiceStatus {
    #[serde(default)]
    pub ready: bool,
    #[serde(
        rename = "latestRevision",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub latest_revision: Option<String>,
    #[serde(rename = "activeInstances", default)]
    pub active_instances: u32,
    #[serde(rename = "desiredInstances", default)]
    pub desired_instances: u32,
    #[serde(rename = "inFlight", default)]
    pub in_flight: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(
        rename = "lastInvokedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_invoked_at: Option<DateTime<Utc>>,
    #[serde(rename = "lastError", default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Resource for ServerlessService {
    type DynamicType = ();

    fn kind(_: &()) -> Cow<'_, str> {
        "ServerlessService".into()
    }

    fn plural(_: &()) -> Cow<'_, str> {
        "serverlessservices".into()
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

impl HasStatus for ServerlessService {
    type Status = ServerlessServiceStatus;

    fn status(&self) -> &Self::Status {
        &self.status
    }

    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

impl Validatable for ServerlessService {
    fn validate_spec(&self) -> Result<()> {
        if self.spec.image.trim().is_empty() {
            return Err(anyhow!("serverlessservice spec.image is required"));
        }
        if self.spec.port == 0 {
            return Err(anyhow!(
                "serverlessservice spec.port must be greater than 0"
            ));
        }
        if self.spec.concurrency.target == 0 {
            return Err(anyhow!(
                "serverlessservice spec.concurrency.target must be greater than 0"
            ));
        }
        if self.spec.scale.max_scale == 0 {
            return Err(anyhow!(
                "serverlessservice spec.scale.maxScale must be greater than 0"
            ));
        }
        if self.spec.scale.min_scale > self.spec.scale.max_scale {
            return Err(anyhow!(
                "serverlessservice spec.scale.minScale must not exceed maxScale"
            ));
        }
        Ok(())
    }
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
