use anyhow::{Result, anyhow};
use apimachinery::{HasStatus, ObjectMeta, Resource, TypeMeta, Validatable};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;

use crate::{GROUP, VERSION};

/// 事件源（对齐 Knative 的 Source 概念）。它周期性 / 受外部条件触发地产生 CloudEvent，
/// 发往 Broker（apiserver-less 的 `/api/v1/events`），再由 EventTrigger 过滤转发到函数或 Workflow。
///
/// 目前支持两类源：
/// - `ping`：时间计划源，对齐 Knative `PingSource`；
/// - `file`：文件修改源，监听某个路径，mtime 变化即触发。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventSource {
    #[serde(flatten)]
    pub types: TypeMeta,
    pub metadata: ObjectMeta,
    pub spec: EventSourceSpec,
    #[serde(default)]
    pub status: EventSourceStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventSourceSpec {
    /// 该源发出的 CloudEvent `type`，EventTrigger 用同名 `eventType` 订阅。
    #[serde(rename = "eventType")]
    pub event_type: String,
    /// 时间计划源（与 `file` 二选一）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping: Option<PingSource>,
    /// 文件修改源（与 `ping` 二选一）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileSource>,
}

/// 时间计划源，对齐 Knative `PingSource`。`schedule`（cron 表达式）与 `intervalSeconds`
/// 二选一：前者更贴近 Knative，后者更直观。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PingSource {
    /// cron 表达式（7 段：`秒 分 时 日 月 周 年`）。例如 `0/10 * * * * * *` 表示每 10 秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// 周期秒数，最简单的定时方式。
    #[serde(
        rename = "intervalSeconds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub interval_seconds: Option<u64>,
    /// 每次触发携带的事件负载（投递给函数作为输入）。
    #[serde(default)]
    pub data: Value,
}

/// 文件修改源：周期轮询 `path` 的修改时间，发生变化即产生一个事件。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileSource {
    /// 监听的文件路径。
    pub path: String,
    /// 轮询间隔秒数；为 0 时控制器按默认值（2 秒）轮询。
    #[serde(rename = "intervalSeconds", default)]
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventSourceStatus {
    #[serde(default)]
    pub ready: bool,
    #[serde(rename = "eventCount", default)]
    pub event_count: u64,
    #[serde(
        rename = "lastEventAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_event_at: Option<DateTime<Utc>>,
    #[serde(rename = "lastError", default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Resource for EventSource {
    type DynamicType = ();

    fn kind(_: &()) -> Cow<'_, str> {
        "EventSource".into()
    }

    fn plural(_: &()) -> Cow<'_, str> {
        "eventsources".into()
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

impl HasStatus for EventSource {
    type Status = EventSourceStatus;

    fn status(&self) -> &Self::Status {
        &self.status
    }

    fn status_mut(&mut self) -> &mut Self::Status {
        &mut self.status
    }
}

impl Validatable for EventSource {
    fn validate_spec(&self) -> Result<()> {
        if self.spec.event_type.trim().is_empty() {
            return Err(anyhow!("eventsource spec.eventType is required"));
        }
        match (&self.spec.ping, &self.spec.file) {
            (Some(_), Some(_)) => {
                return Err(anyhow!(
                    "eventsource spec must set exactly one of ping / file"
                ));
            }
            (None, None) => {
                return Err(anyhow!("eventsource spec must set one of ping / file"));
            }
            (Some(ping), None) => {
                if ping.schedule.is_none() && ping.interval_seconds.is_none() {
                    return Err(anyhow!(
                        "eventsource ping requires schedule or intervalSeconds"
                    ));
                }
                if ping.interval_seconds == Some(0) {
                    return Err(anyhow!(
                        "eventsource ping.intervalSeconds must be greater than 0"
                    ));
                }
            }
            (None, Some(file)) => {
                if file.path.trim().is_empty() {
                    return Err(anyhow!("eventsource file.path is required"));
                }
            }
        }
        Ok(())
    }
}
