use serde::Serialize;
use serde_json::Value;
use serverless_api::{BranchCondition, Workflow, WorkflowStep};
use std::collections::BTreeSet;

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

pub(crate) fn reachable_functions(workflow: &Workflow) -> Vec<String> {
    let mut visited_steps = BTreeSet::new();
    let mut functions = BTreeSet::new();
    let mut pending = vec![workflow.spec.entrypoint.clone()];

    while let Some(step_name) = pending.pop() {
        if !visited_steps.insert(step_name.clone()) {
            continue;
        }
        let Some(step) = workflow.spec.steps.get(&step_name) else {
            continue;
        };
        if !step.function.trim().is_empty() {
            functions.insert(step.function.clone());
        }
        if let Some(next) = &step.next {
            pending.push(next.clone());
        }
        for branch in &step.branches {
            pending.push(branch.next.clone());
        }
    }

    functions.into_iter().collect()
}

fn select_json_field<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in field.split('.').filter(|part| !part.is_empty()) {
        current = current.get(part)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serverless_api::{WorkflowBranch, WorkflowSpec};
    use std::collections::BTreeMap;

    #[test]
    fn reachable_functions_collects_all_branches_once() {
        let workflow = Workflow {
            spec: WorkflowSpec {
                entrypoint: "classify".to_string(),
                steps: BTreeMap::from([
                    (
                        "classify".to_string(),
                        WorkflowStep {
                            function: "ticket-classify".to_string(),
                            next: Some("score".to_string()),
                            branches: Vec::new(),
                        },
                    ),
                    (
                        "score".to_string(),
                        WorkflowStep {
                            function: "risk-score".to_string(),
                            next: Some("auto".to_string()),
                            branches: vec![WorkflowBranch {
                                next: "human".to_string(),
                                ..Default::default()
                            }],
                        },
                    ),
                    (
                        "auto".to_string(),
                        WorkflowStep {
                            function: "auto-reply".to_string(),
                            next: Some("notify".to_string()),
                            branches: Vec::new(),
                        },
                    ),
                    (
                        "human".to_string(),
                        WorkflowStep {
                            function: "human-escalate".to_string(),
                            next: Some("notify".to_string()),
                            branches: Vec::new(),
                        },
                    ),
                    (
                        "notify".to_string(),
                        WorkflowStep {
                            function: "notify".to_string(),
                            next: None,
                            branches: Vec::new(),
                        },
                    ),
                ]),
            },
            ..Default::default()
        };

        assert_eq!(
            reachable_functions(&workflow),
            vec![
                "auto-reply",
                "human-escalate",
                "notify",
                "risk-score",
                "ticket-classify",
            ]
        );
    }
}
