use anyhow::{Result, anyhow};
use apimachinery::{HasStatus, ObjectMeta, Resource, TypeMeta, Validatable};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::{GROUP, VERSION};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    #[serde(flatten)]
    pub types: TypeMeta,
    pub metadata: ObjectMeta,
    pub spec: RevisionSpec,
    #[serde(default)]
    pub status: RevisionStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionSpec {
    #[serde(rename = "serviceName")]
    pub service_name: String,
    pub image: String,
    pub port: u16,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RevisionStatus {
    #[serde(default)]
    pub ready: bool,
    #[serde(rename = "createdAt", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

impl Resource for Revision {
    type DynamicType = ();

    fn kind(_: &()) -> Cow<'_, str> {
        "Revision".into()
    }

    fn plural(_: &()) -> Cow<'_, str> {
        "revisions".into()
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

impl HasStatus for Revision {
    type Status = RevisionStatus;

    fn status(&self) -> &Self::Status {
        &self.status
    }

    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

impl Validatable for Revision {
    fn validate_spec(&self) -> Result<()> {
        if self.spec.service_name.trim().is_empty() {
            return Err(anyhow!("revision spec.serviceName is required"));
        }
        if self.spec.image.trim().is_empty() {
            return Err(anyhow!("revision spec.image is required"));
        }
        if self.spec.port == 0 {
            return Err(anyhow!("revision spec.port must be greater than 0"));
        }
        Ok(())
    }
}
