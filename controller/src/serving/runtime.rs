use serde::Serialize;
use serverless_api::ServerlessService;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::state::object_namespace;

#[derive(Clone, Default)]
pub(crate) struct RuntimeRegistry {
    inner: Arc<RwLock<BTreeMap<String, RuntimeEntry>>>,
}

#[derive(Debug, Clone)]
struct RuntimeEntry {
    active_instances: u32,
    in_flight: u32,
    last_used: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) active_instances: u32,
    pub(crate) in_flight: u32,
}

impl RuntimeRegistry {
    pub(crate) fn begin(&self, service: &ServerlessService) -> RuntimeSnapshot {
        let key = service_key(service);
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let entry = guard.entry(key).or_insert_with(|| RuntimeEntry {
            active_instances: service.spec.scale.min_scale,
            in_flight: 0,
            last_used: Instant::now(),
        });
        if entry.active_instances == 0 {
            entry.active_instances = 1;
        }
        let target = service.spec.concurrency.target.max(1);
        let max_instances = service
            .spec
            .scale
            .max_scale
            .max(service.spec.scale.min_scale)
            .max(1);
        if entry.in_flight >= entry.active_instances.saturating_mul(target)
            && entry.active_instances < max_instances
        {
            entry.active_instances += 1;
        }
        entry.in_flight += 1;
        entry.last_used = Instant::now();
        RuntimeSnapshot {
            active_instances: entry.active_instances,
            in_flight: entry.in_flight,
        }
    }

    pub(crate) fn end(&self, service: &ServerlessService) -> RuntimeSnapshot {
        let key = service_key(service);
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let entry = guard.entry(key).or_insert_with(|| RuntimeEntry {
            active_instances: service.spec.scale.min_scale,
            in_flight: 0,
            last_used: Instant::now(),
        });
        entry.in_flight = entry.in_flight.saturating_sub(1);
        entry.last_used = Instant::now();
        RuntimeSnapshot {
            active_instances: entry.active_instances,
            in_flight: entry.in_flight,
        }
    }

    pub(crate) fn scale_idle(&self, service: &ServerlessService) -> Option<RuntimeSnapshot> {
        let key = service_key(service);
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let entry = guard.get_mut(&key)?;
        let min_instances = service.spec.scale.min_scale;
        if entry.in_flight == 0
            && entry.active_instances > min_instances
            && entry.last_used.elapsed() >= Duration::from_secs(service.spec.scale.idle_seconds)
        {
            entry.active_instances = min_instances;
            return Some(RuntimeSnapshot {
                active_instances: entry.active_instances,
                in_flight: entry.in_flight,
            });
        }
        None
    }

    pub(crate) fn ensure_min_instances(
        &self,
        service: &ServerlessService,
    ) -> Option<RuntimeSnapshot> {
        let min_instances = service.spec.scale.min_scale;
        if min_instances == 0 {
            return None;
        }
        let key = service_key(service);
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let entry = guard.entry(key).or_insert_with(|| RuntimeEntry {
            active_instances: 0,
            in_flight: 0,
            last_used: Instant::now(),
        });
        if entry.active_instances < min_instances {
            entry.active_instances = min_instances;
            return Some(RuntimeSnapshot {
                active_instances: entry.active_instances,
                in_flight: entry.in_flight,
            });
        }
        None
    }

    pub(crate) fn snapshot(&self, key: &str) -> RuntimeSnapshot {
        let guard = self.inner.read().unwrap_or_else(|error| error.into_inner());
        match guard.get(key) {
            Some(entry) => RuntimeSnapshot {
                active_instances: entry.active_instances,
                in_flight: entry.in_flight,
            },
            None => RuntimeSnapshot {
                active_instances: 0,
                in_flight: 0,
            },
        }
    }
}

pub(crate) fn service_key(service: &ServerlessService) -> String {
    runtime_key(&object_namespace(&service.metadata), &service.metadata.name)
}

pub(crate) fn runtime_key(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}
