use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde_json::{Value, json};
use serverless_api::{CloudEvent, EventSource};

use crate::eventing::broker::broker_publish;
use crate::state::{AppState, object_namespace};
use crate::status::{patch_event_source_error, patch_event_source_fired};

/// 事件源后台循环（对齐 Knative 的 Source adapter）。周期性扫描所有 EventSource，
/// 各自判断是否到达触发条件（定时 / 文件变化），到了就产生一个 CloudEvent 灌进 Broker。
/// 它只负责「产生事件」，下游的扇出、过滤、调用沿用 Broker / EventTrigger。
pub(crate) async fn event_source_loop(state: AppState, tick: Duration) {
    let mut runtimes: HashMap<String, SourceRuntime> = HashMap::new();
    loop {
        tokio::time::sleep(tick).await;
        let now = Utc::now();
        let sources = state.sources.items();
        let mut live = HashSet::new();
        for source in &sources {
            let key = format!(
                "{}/{}",
                object_namespace(&source.metadata),
                source.metadata.name
            );
            live.insert(key.clone());
            if !runtimes.contains_key(&key) {
                match SourceRuntime::init(source, now) {
                    Ok(runtime) => {
                        runtimes.insert(key.clone(), runtime);
                    }
                    Err(error) => {
                        tracing::warn!(key = %key, error = %error, "invalid event source");
                        patch_event_source_error(&state, source, error).await;
                        runtimes.insert(key.clone(), SourceRuntime::Invalid);
                        continue;
                    }
                }
            }
            let fired = runtimes.get_mut(&key).and_then(|runtime| runtime.poll(now));
            if let Some(data) = fired {
                emit(&state, source, data).await;
            }
        }
        // 已删除的 EventSource，清理其本地状态。
        runtimes.retain(|key, _| live.contains(key));
    }
}

async fn emit(state: &AppState, source: &EventSource, data: Value) {
    let source_id = format!(
        "/eventsources/{}/{}",
        object_namespace(&source.metadata),
        source.metadata.name
    );
    let event = CloudEvent::new(source.spec.event_type.clone(), source_id, data);
    let deliveries = broker_publish(state, &event).await;
    tracing::info!(
        source = %source.metadata.name,
        event_type = %source.spec.event_type,
        delivered = deliveries.len(),
        "event source fired"
    );
    patch_event_source_fired(state, source).await;
}

/// 每个 EventSource 的运行期状态，只在 [`event_source_loop`] 内部维护。
enum SourceRuntime {
    PingInterval {
        interval: Duration,
        last_fire: DateTime<Utc>,
        data: Value,
    },
    PingCron {
        schedule: Schedule,
        next: Option<DateTime<Utc>>,
        data: Value,
    },
    File {
        path: PathBuf,
        poll: Duration,
        last_poll: DateTime<Utc>,
        last_mtime: Option<SystemTime>,
    },
    /// spec 非法（例如 cron 解析失败），不产生任何事件。
    Invalid,
}

impl SourceRuntime {
    fn init(source: &EventSource, now: DateTime<Utc>) -> Result<Self, String> {
        if let Some(ping) = &source.spec.ping {
            if let Some(expr) = &ping.schedule {
                let schedule = Schedule::from_str(expr)
                    .map_err(|error| format!("invalid cron schedule {expr:?}: {error}"))?;
                let next = schedule.after(&now).next();
                return Ok(SourceRuntime::PingCron {
                    schedule,
                    next,
                    data: ping.data.clone(),
                });
            }
            if let Some(interval) = ping.interval_seconds {
                return Ok(SourceRuntime::PingInterval {
                    interval: Duration::from_secs(interval.max(1)),
                    last_fire: now,
                    data: ping.data.clone(),
                });
            }
            return Err("ping requires schedule or intervalSeconds".to_string());
        }
        if let Some(file) = &source.spec.file {
            let poll = if file.interval_seconds == 0 {
                2
            } else {
                file.interval_seconds
            };
            return Ok(SourceRuntime::File {
                path: PathBuf::from(&file.path),
                poll: Duration::from_secs(poll),
                last_poll: now,
                // 记录基线 mtime：只有此后发生变化才触发。
                last_mtime: file_mtime(&file.path),
            });
        }
        Err("eventsource spec must set one of ping / file".to_string())
    }

    fn poll(&mut self, now: DateTime<Utc>) -> Option<Value> {
        match self {
            SourceRuntime::PingInterval {
                interval,
                last_fire,
                data,
            } => {
                if elapsed_since(now, *last_fire) >= *interval {
                    *last_fire = now;
                    Some(data.clone())
                } else {
                    None
                }
            }
            SourceRuntime::PingCron {
                schedule,
                next,
                data,
            } => match *next {
                Some(at) if now >= at => {
                    *next = schedule.after(&now).next();
                    Some(data.clone())
                }
                _ => None,
            },
            SourceRuntime::File {
                path,
                poll,
                last_poll,
                last_mtime,
            } => {
                if elapsed_since(now, *last_poll) < *poll {
                    return None;
                }
                *last_poll = now;
                match (*last_mtime, file_mtime(&*path)) {
                    (Some(prev), Some(current)) if current != prev => {
                        *last_mtime = Some(current);
                        Some(file_event_data(&*path, current))
                    }
                    (None, Some(current)) => {
                        // 文件首次出现，建立基线但不触发。
                        *last_mtime = Some(current);
                        None
                    }
                    _ => None,
                }
            }
            SourceRuntime::Invalid => None,
        }
    }
}

fn elapsed_since(now: DateTime<Utc>, since: DateTime<Utc>) -> Duration {
    now.signed_duration_since(since)
        .to_std()
        .unwrap_or_default()
}

fn file_mtime(path: impl AsRef<Path>) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

fn file_event_data(path: &Path, mtime: SystemTime) -> Value {
    let modified_unix = mtime
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    json!({
        "path": path.display().to_string(),
        "modifiedUnix": modified_unix,
    })
}
