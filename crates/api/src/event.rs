use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// 对齐 [CloudEvents 1.0](https://cloudevents.io/) 的事件信封，也是 Knative Eventing 在
/// 各个 Source / Broker / Trigger 之间传递的事件格式。事件源产生事件、Broker 扇出、
/// Trigger 过滤时都围绕这里的 `type` 字段。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CloudEvent {
    #[serde(rename = "specversion")]
    pub spec_version: String,
    pub id: String,
    /// 事件来源标识，例如 `/eventsources/default/cron-tick`。
    pub source: String,
    /// 事件类型，EventTrigger 据此过滤订阅（对应 Knative Trigger 的 filter）。
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<DateTime<Utc>>,
    #[serde(
        rename = "datacontenttype",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub data_content_type: Option<String>,
    /// 事件负载，投递给函数时作为其输入。
    #[serde(default)]
    pub data: Value,
}

impl CloudEvent {
    /// 构造一个 CloudEvents 1.0 事件，自动生成 `id` 和 `time`。
    pub fn new(event_type: impl Into<String>, source: impl Into<String>, data: Value) -> Self {
        Self {
            spec_version: "1.0".to_string(),
            id: Uuid::new_v4().to_string(),
            source: source.into(),
            event_type: event_type.into(),
            time: Some(Utc::now()),
            data_content_type: Some("application/json".to_string()),
            data,
        }
    }
}
