use serde::Serialize;
use serde_json::Value;
use serverless_api::{CloudEvent, EventTrigger, TriggerTargetKind};

use crate::serving::{invoke_function_inner, invoke_workflow_inner};
use crate::state::{AppState, object_namespace};
use crate::status::patch_event_trigger_success;

/// 一次事件投递到单个 EventTrigger 目标的结果。
#[derive(Debug, Serialize)]
pub(crate) struct EventDelivery {
    pub(crate) namespace: String,
    pub(crate) trigger: String,
    pub(crate) target_kind: String,
    pub(crate) target_name: String,
    pub(crate) result: Value,
}

/// Broker 扇出：把一个 CloudEvent 投递给所有 `eventType` 匹配的 EventTrigger。
///
/// HTTP 入口（`POST /api/v1/events/:type`）和各类事件源都汇入这里，对应 Knative 中
/// 「Source / 外部 → Broker → Trigger → 订阅者」的中间一段。单个 Trigger 投递失败只记录
/// 日志并继续，不影响其它订阅者。
pub(crate) async fn broker_publish(state: &AppState, event: &CloudEvent) -> Vec<EventDelivery> {
    let mut results = Vec::new();
    for trigger in state
        .triggers
        .items()
        .into_iter()
        .filter(|trigger| trigger.spec.event_type == event.event_type)
    {
        let namespace = object_namespace(&trigger.metadata);
        match deliver(state, &namespace, &trigger, event.data.clone()).await {
            Ok(delivery) => {
                patch_event_trigger_success(state, &namespace, &trigger).await;
                results.push(delivery);
            }
            Err(error) => {
                tracing::warn!(
                    namespace,
                    trigger = %trigger.metadata.name,
                    event_type = %event.event_type,
                    error = %error,
                    "event delivery failed"
                );
            }
        }
    }
    results
}

async fn deliver(
    state: &AppState,
    namespace: &str,
    trigger: &EventTrigger,
    data: Value,
) -> Result<EventDelivery, String> {
    match trigger.spec.target.kind {
        TriggerTargetKind::Function => {
            let response = invoke_function_inner(state, namespace, &trigger.spec.target.name, data)
                .await
                .map_err(|(_, message)| message)?;
            Ok(EventDelivery {
                namespace: namespace.to_string(),
                trigger: trigger.metadata.name.clone(),
                target_kind: "Function".to_string(),
                target_name: trigger.spec.target.name.clone(),
                result: response.result,
            })
        }
        TriggerTargetKind::Workflow => {
            let response = invoke_workflow_inner(state, namespace, &trigger.spec.target.name, data)
                .await
                .map_err(|(_, message)| message)?;
            Ok(EventDelivery {
                namespace: namespace.to_string(),
                trigger: trigger.metadata.name.clone(),
                target_kind: "Workflow".to_string(),
                target_name: trigger.spec.target.name.clone(),
                result: response.result,
            })
        }
    }
}
