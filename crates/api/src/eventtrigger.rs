use anyhow::{Result, anyhow};
use apimachinery::{HasStatus, ObjectMeta, Resource, TypeMeta, Validatable};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use crate::{GROUP, VERSION};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventTrigger {
    #[serde(flatten)]
    pub types: TypeMeta,
    pub metadata: ObjectMeta,
    pub spec: EventTriggerSpec,
    #[serde(default)]
    pub status: EventTriggerStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventTriggerSpec {
    #[serde(rename = "eventType")]
    pub event_type: String,
    pub target: TriggerTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TriggerTargetKind {
    Function,
    Workflow,
}

impl Default for TriggerTargetKind {
    fn default() -> Self {
        Self::Function
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerTarget {
    pub kind: TriggerTargetKind,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventTriggerStatus {
    #[serde(default)]
    pub ready: bool,
    #[serde(rename = "deliveredCount", default)]
    pub delivered_count: u64,
    #[serde(
        rename = "lastDeliveredAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_delivered_at: Option<DateTime<Utc>>,
    #[serde(rename = "lastError", default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Resource for EventTrigger {
    type DynamicType = ();

    fn kind(_: &()) -> Cow<'_, str> {
        "EventTrigger".into()
    }

    fn plural(_: &()) -> Cow<'_, str> {
        "eventtriggers".into()
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

impl HasStatus for EventTrigger {
    type Status = EventTriggerStatus;

    fn status(&self) -> &Self::Status {
        &self.status
    }

    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

impl Validatable for EventTrigger {
    fn validate_spec(&self) -> Result<()> {
        if self.spec.event_type.trim().is_empty() {
            return Err(anyhow!("eventtrigger spec.eventType is required"));
        }
        if self.spec.target.name.trim().is_empty() {
            return Err(anyhow!("eventtrigger spec.target.name is required"));
        }
        Ok(())
    }
}
