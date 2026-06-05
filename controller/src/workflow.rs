use serde::Serialize;
use serde_json::Value;
use serverless_api::{BranchCondition, WorkflowStep};

#[derive(Debug, Serialize)]
pub(crate) struct WorkflowInvokeResponse {
    pub(crate) result: Value,
    pub(crate) trace: Vec<WorkflowTraceEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkflowTraceEntry {
    pub(crate) step: String,
    pub(crate) function: String,
    pub(crate) output: Value,
}

pub(crate) fn next_step(step: &WorkflowStep, output: &Value) -> Option<String> {
    for branch in &step.branches {
        if branch_matches(&branch.when, output) {
            return Some(branch.next.clone());
        }
    }
    step.next.clone()
}

fn branch_matches(condition: &BranchCondition, output: &Value) -> bool {
    let selected = condition
        .field
        .as_deref()
        .and_then(|field| select_json_field(output, field))
        .unwrap_or(output);
    if let Some(expected) = &condition.equals
        && selected == expected
    {
        return true;
    }
    if let Some(needle) = &condition.contains {
        return selected
            .as_str()
            .map(|value| value.contains(needle))
            .unwrap_or_else(|| selected.to_string().contains(needle));
    }
    condition.equals.is_none() && condition.contains.is_none()
}

fn select_json_field<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in field.split('.').filter(|part| !part.is_empty()) {
        current = current.get(part)?;
    }
    Some(current)
}
