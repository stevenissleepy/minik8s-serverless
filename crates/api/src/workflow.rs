use anyhow::{Result, anyhow};
use apimachinery::{HasStatus, ObjectMeta, Resource, TypeMeta, Validatable};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::{GROUP, VERSION};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(flatten)]
    pub types: TypeMeta,
    pub metadata: ObjectMeta,
    pub spec: WorkflowSpec,
    #[serde(default)]
    pub status: WorkflowStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub entrypoint: String,
    #[serde(default)]
    pub steps: BTreeMap<String, WorkflowStep>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub function: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<WorkflowBranch>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowBranch {
    #[serde(default)]
    pub when: BranchCondition,
    pub next: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BranchCondition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStatus {
    #[serde(default)]
    pub ready: bool,
    #[serde(
        rename = "lastInvokedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_invoked_at: Option<DateTime<Utc>>,
    #[serde(rename = "lastError", default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Resource for Workflow {
    type DynamicType = ();

    fn kind(_: &()) -> Cow<'_, str> {
        "Workflow".into()
    }

    fn plural(_: &()) -> Cow<'_, str> {
        "workflows".into()
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

impl HasStatus for Workflow {
    type Status = WorkflowStatus;

    fn status(&self) -> &Self::Status {
        &self.status
    }

    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

impl Validatable for Workflow {
    fn validate_spec(&self) -> Result<()> {
        if self.spec.entrypoint.trim().is_empty() {
            return Err(anyhow!("workflow spec.entrypoint is required"));
        }
        if !self.spec.steps.contains_key(&self.spec.entrypoint) {
            return Err(anyhow!(
                "workflow spec.entrypoint must name an existing step"
            ));
        }
        for (name, step) in &self.spec.steps {
            if step.function.trim().is_empty() {
                return Err(anyhow!("workflow step {name} function is required"));
            }
            if let Some(next) = step.next.as_deref()
                && !self.spec.steps.contains_key(next)
            {
                return Err(anyhow!("workflow step {name} next step {next} not found"));
            }
            for branch in &step.branches {
                if branch.next.trim().is_empty() {
                    return Err(anyhow!("workflow step {name} branch next is required"));
                }
                if !self.spec.steps.contains_key(&branch.next) {
                    return Err(anyhow!(
                        "workflow step {name} branch next step {} not found",
                        branch.next
                    ));
                }
            }
        }
        Ok(())
    }
}
