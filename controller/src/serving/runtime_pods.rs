use anyhow::{Context, Result, anyhow};
use api::pod::{ContainerPort, ContainerSpec, EnvVar, Pod, PodPhase, PodSpec};
use api::service::{Protocol, Service, ServicePort, ServiceSpec, ServiceType};
use apimachinery::{LabelSelector, ObjectMeta, Resource, TypeMeta};
use chrono::Utc;
use client_rs::{ListParams, TypedApi};
use serde_json::Value;
use serverless_api::{
    Revision, RevisionSpec, RevisionStatus, ServerlessService, ServerlessServiceStatus,
};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::serving::runtime::runtime_key;
use crate::state::{AppState, object_namespace};

const MANAGED_BY_LABEL: &str = "serverless.minik8s.io/managed-by";
const SERVICE_LABEL: &str = "serverless.minik8s.io/service";
const REVISION_LABEL: &str = "serverless.minik8s.io/revision";
const MANAGED_BY: &str = "serverless-controller";

pub(crate) async fn invoke_service_pod(
    state: &AppState,
    service: &ServerlessService,
    input: Value,
    desired_instances: u32,
) -> Result<Value> {
    reconcile_service_resources(state, service, desired_instances.max(1)).await?;
    let pod = ensure_ready_service_pod(state, service).await?;
    let pod_ip = pod
        .status
        .pod_ip
        .as_deref()
        .ok_or_else(|| anyhow!("serverless pod {} has no Pod IP", pod.metadata.name))?;
    let response = post_json(
        pod_ip.to_string(),
        service.spec.port,
        "/invoke".to_string(),
        input,
    )
    .await
    .with_context(|| {
        format!(
            "failed to call serverless pod {pod_ip}:{}",
            service.spec.port
        )
    })?;
    if !(200..300).contains(&response.status) {
        return Err(anyhow!(
            "serverless runtime returned {}: {}",
            response.status,
            response.body.trim()
        ));
    }
    if response.body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(response.body.trim()).with_context(|| {
        format!(
            "serverless runtime response is not JSON: {}",
            response.body.trim()
        )
    })
}

pub(crate) async fn scale_service_pods(
    state: &AppState,
    service: &ServerlessService,
    desired_instances: u32,
) -> Result<()> {
    reconcile_service_resources(state, service, desired_instances).await
}

pub(crate) async fn reconcile_all_services(state: &AppState) {
    for service in state.services.items() {
        let desired = state
            .runtime
            .snapshot(&runtime_key(
                &object_namespace(&service.metadata),
                &service.metadata.name,
            ))
            .active_instances
            .max(service.spec.scale.min_scale);
        if let Err(error) = reconcile_service_resources(state, &service, desired).await {
            tracing::warn!(
                namespace = %object_namespace(&service.metadata),
                name = %service.metadata.name,
                error = %format!("{error:#}"),
                "failed to reconcile ServerlessService"
            );
        }
    }
    if let Err(error) = cleanup_orphan_runtime_resources(state).await {
        tracing::warn!(
            error = %format!("{error:#}"),
            "failed to cleanup orphan serverless resources"
        );
    }
}

pub(crate) async fn cleanup_orphan_runtime_resources(state: &AppState) -> Result<()> {
    let services = state
        .services
        .items()
        .into_iter()
        .map(|service| {
            (
                object_namespace(&service.metadata),
                service.metadata.name.clone(),
            )
        })
        .collect::<BTreeSet<_>>();

    let pod_api = TypedApi::<Pod>::all(state.client.clone());
    for pod in pod_api
        .list(&ListParams::default())
        .await
        .context("failed to list serverless pods for cleanup")?
        .items
        .into_iter()
        .filter(is_managed_runtime_object)
    {
        if let Some(service_name) = pod.metadata.labels.get(SERVICE_LABEL)
            && !services.contains(&(object_namespace(&pod.metadata), service_name.clone()))
        {
            let api =
                TypedApi::<Pod>::namespaced(state.client.clone(), object_namespace(&pod.metadata));
            delete_pod_if_exists(&api, &pod.metadata.name).await?;
        }
    }

    let service_api = TypedApi::<Service>::all(state.client.clone());
    for k8s_service in service_api
        .list(&ListParams::default())
        .await
        .context("failed to list serverless Services for cleanup")?
        .items
        .into_iter()
        .filter(is_managed_runtime_object)
    {
        if let Some(service_name) = k8s_service.metadata.labels.get(SERVICE_LABEL)
            && !services.contains(&(
                object_namespace(&k8s_service.metadata),
                service_name.clone(),
            ))
        {
            let api = TypedApi::<Service>::namespaced(
                state.client.clone(),
                object_namespace(&k8s_service.metadata),
            );
            match api.delete(&k8s_service.metadata.name).await {
                Ok(_) => {}
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    Ok(())
}

async fn reconcile_service_resources(
    state: &AppState,
    service: &ServerlessService,
    desired_instances: u32,
) -> Result<()> {
    let namespace = object_namespace(&service.metadata);
    let lock = state
        .runtime_pod_locks
        .lock_for(runtime_key(&namespace, &service.metadata.name))
        .await;
    let _guard = lock.lock().await;

    let revision_name = service_revision_name(service);
    ensure_revision(state, service, &revision_name).await?;
    ensure_k8s_service(state, service, &revision_name).await?;

    let api = TypedApi::<Pod>::namespaced(state.client.clone(), namespace.clone());
    let pods = service_pods(state, service).await?;
    let mut active = Vec::new();

    for pod in pods {
        if pod.metadata.labels.get(REVISION_LABEL).map(String::as_str)
            != Some(revision_name.as_str())
            || matches!(pod.status.phase, PodPhase::Failed | PodPhase::Succeeded)
        {
            delete_pod_if_exists(&api, &pod.metadata.name).await?;
        } else {
            active.push(pod);
        }
    }

    if active.len() < desired_instances as usize {
        for _ in 0..(desired_instances as usize - active.len()) {
            let pod = build_service_pod(service, &revision_name);
            api.create(&pod).await.with_context(|| {
                format!(
                    "failed to create serverless pod for {namespace}/{}",
                    service.metadata.name
                )
            })?;
        }
        update_revision_status(state, service, &revision_name, true).await;
        return Ok(());
    }

    if active.len() > desired_instances as usize {
        active.sort_by_key(|pod| pod.metadata.creation_timestamp);
        for pod in active.into_iter().skip(desired_instances as usize) {
            delete_pod_if_exists(&api, &pod.metadata.name).await?;
        }
    }

    update_revision_status(state, service, &revision_name, true).await;
    Ok(())
}

async fn ensure_revision(
    state: &AppState,
    service: &ServerlessService,
    revision_name: &str,
) -> Result<()> {
    let namespace = object_namespace(&service.metadata);
    let api = TypedApi::<Revision>::namespaced(state.client.clone(), namespace.clone());
    let revision = Revision {
        types: TypeMeta::for_resource::<Revision>(),
        metadata: ObjectMeta {
            name: revision_name.to_string(),
            namespace,
            labels: runtime_labels(service, revision_name),
            owner_references: vec![service.owner_ref()],
            ..Default::default()
        },
        spec: RevisionSpec {
            service_name: service.metadata.name.clone(),
            image: service.spec.image.clone(),
            port: service.spec.port,
            env: service.spec.env.clone(),
        },
        status: Default::default(),
    };
    api.apply(&revision)
        .await
        .with_context(|| format!("failed to apply Revision {revision_name}"))?;
    Ok(())
}

async fn ensure_k8s_service(
    state: &AppState,
    service: &ServerlessService,
    revision_name: &str,
) -> Result<()> {
    let namespace = object_namespace(&service.metadata);
    let api = TypedApi::<Service>::namespaced(state.client.clone(), namespace.clone());
    let name = k8s_service_name(service);
    let existing_cluster_ip = match api.get(&name).await {
        Ok(existing) => existing.spec.cluster_ip,
        Err(error) if error.is_not_found() => None,
        Err(error) => return Err(error.into()),
    };
    let k8s_service = Service {
        types: TypeMeta::for_resource::<Service>(),
        metadata: ObjectMeta {
            name: name.clone(),
            namespace,
            labels: runtime_labels(service, revision_name),
            owner_references: vec![service.owner_ref()],
            ..Default::default()
        },
        spec: ServiceSpec {
            type_: ServiceType::ClusterIP,
            selector: LabelSelector {
                match_labels: BTreeMap::from([
                    (MANAGED_BY_LABEL.to_string(), MANAGED_BY.to_string()),
                    (SERVICE_LABEL.to_string(), service.metadata.name.clone()),
                    (REVISION_LABEL.to_string(), revision_name.to_string()),
                ]),
            },
            ports: vec![ServicePort {
                name: Some("http".to_string()),
                port: service.spec.port,
                target_port: service.spec.port,
                node_port: None,
                protocol: Protocol::Tcp,
            }],
            cluster_ip: existing_cluster_ip,
        },
        status: Default::default(),
    };
    api.apply(&k8s_service)
        .await
        .with_context(|| format!("failed to apply Service {name}"))?;
    Ok(())
}

async fn update_revision_status(
    state: &AppState,
    service: &ServerlessService,
    revision_name: &str,
    ready: bool,
) {
    let namespace = object_namespace(&service.metadata);
    let api = TypedApi::<Revision>::namespaced(state.client.clone(), namespace);
    let status = RevisionStatus {
        ready,
        created_at: Some(Utc::now()),
    };
    if let Err(error) = api.replace_status(revision_name, &status).await {
        tracing::warn!(
            revision = revision_name,
            error = %error,
            "failed to update Revision status"
        );
    }
}

// 冷启动等待上限，对齐 Knative revision timeoutSeconds 的默认值。
const STARTUP_TIMEOUT_SECONDS: u64 = 300;

async fn ensure_ready_service_pod(state: &AppState, service: &ServerlessService) -> Result<Pod> {
    for _ in 0..STARTUP_TIMEOUT_SECONDS * 2 {
        let mut pods = ready_service_pods(state, service).await?;
        if !pods.is_empty() {
            let index = pod_pick_index(pods.len());
            pods.rotate_left(index);
            for pod in pods {
                if runtime_http_ready(&pod, service.spec.port).await {
                    return Ok(pod);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(anyhow!(
        "timed out waiting for ServerlessService {}/{} pod after {STARTUP_TIMEOUT_SECONDS}s",
        object_namespace(&service.metadata),
        service.metadata.name
    ))
}

async fn ready_service_pods(state: &AppState, service: &ServerlessService) -> Result<Vec<Pod>> {
    let mut pods = service_pods(state, service).await?;
    pods.retain(|pod| {
        pod.status.phase == PodPhase::Running
            && pod.status.pod_ip.is_some()
            && pod
                .status
                .container_statuses
                .iter()
                .any(|status| status.name == "user-container" && status.ready)
    });
    pods.sort_by_key(|pod| pod.metadata.creation_timestamp);
    Ok(pods)
}

async fn runtime_http_ready(pod: &Pod, port: u16) -> bool {
    let Some(pod_ip) = pod.status.pod_ip.as_deref() else {
        return false;
    };
    matches!(
        get_status(pod_ip.to_string(), port, "/healthz".to_string()).await,
        Ok(status) if (200..300).contains(&status)
    )
}

async fn service_pods(state: &AppState, service: &ServerlessService) -> Result<Vec<Pod>> {
    let namespace = object_namespace(&service.metadata);
    let api = TypedApi::<Pod>::namespaced(state.client.clone(), namespace);
    let pods = api
        .list(&ListParams::default())
        .await
        .context("failed to list serverless pods")?;
    Ok(pods
        .items
        .into_iter()
        .filter(|pod| {
            pod.metadata
                .labels
                .get(MANAGED_BY_LABEL)
                .map(String::as_str)
                == Some(MANAGED_BY)
                && pod.metadata.labels.get(SERVICE_LABEL).map(String::as_str)
                    == Some(service.metadata.name.as_str())
        })
        .collect())
}

async fn delete_pod_if_exists(api: &TypedApi<Pod>, name: &str) -> Result<()> {
    match api.delete(name).await {
        Ok(_) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn build_service_pod(service: &ServerlessService, revision_name: &str) -> Pod {
    let namespace = object_namespace(&service.metadata);
    Pod {
        types: TypeMeta::for_resource::<Pod>(),
        metadata: ObjectMeta {
            generate_name: Some(format!("sks-{}-", service.metadata.name)),
            namespace,
            labels: runtime_labels(service, revision_name),
            owner_references: vec![service.owner_ref()],
            ..Default::default()
        },
        spec: PodSpec {
            containers: vec![ContainerSpec {
                name: "user-container".to_string(),
                image: service.spec.image.clone(),
                env: service_env(service),
                ports: vec![ContainerPort {
                    container_port: service.spec.port,
                    protocol: Some("TCP".to_string()),
                }],
                ..Default::default()
            }],
            restart_policy: Some("Always".to_string()),
            ..Default::default()
        },
        status: Default::default(),
    }
}

fn service_env(service: &ServerlessService) -> Vec<EnvVar> {
    let mut env = service
        .spec
        .env
        .iter()
        .map(|(name, value)| EnvVar {
            name: name.clone(),
            value: value.clone(),
            value_from: None,
        })
        .collect::<Vec<_>>();
    env.extend([
        EnvVar {
            name: "FUNCTION_NAMESPACE".to_string(),
            value: object_namespace(&service.metadata),
            value_from: None,
        },
        EnvVar {
            name: "FUNCTION_NAME".to_string(),
            value: service.metadata.name.clone(),
            value_from: None,
        },
    ]);
    env
}

fn runtime_labels(service: &ServerlessService, revision_name: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MANAGED_BY_LABEL.to_string(), MANAGED_BY.to_string()),
        (SERVICE_LABEL.to_string(), service.metadata.name.clone()),
        (REVISION_LABEL.to_string(), revision_name.to_string()),
    ])
}

fn is_managed_runtime_object<R: Resource>(object: &R) -> bool {
    object
        .metadata()
        .labels
        .get(MANAGED_BY_LABEL)
        .map(String::as_str)
        == Some(MANAGED_BY)
}

fn k8s_service_name(service: &ServerlessService) -> String {
    format!("sks-{}", service.metadata.name)
}

pub(crate) fn service_revision_name(service: &ServerlessService) -> String {
    format!(
        "{}-{}",
        service.metadata.name,
        service_revision_hash(service)
    )
}

fn service_revision_hash(service: &ServerlessService) -> String {
    let mut hasher = DefaultHasher::new();
    service.spec.image.hash(&mut hasher);
    service.spec.port.hash(&mut hasher);
    service.spec.env.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{hash:016x}").chars().take(10).collect()
}

pub(crate) fn service_url(service: &ServerlessService) -> String {
    format!(
        "http://{}.{}.svc.cluster.local:{}",
        k8s_service_name(service),
        object_namespace(&service.metadata),
        service.spec.port
    )
}

pub(crate) fn service_status(
    service: &ServerlessService,
    active_instances: u32,
    in_flight: u32,
    last_error: Option<String>,
) -> ServerlessServiceStatus {
    ServerlessServiceStatus {
        ready: true,
        latest_revision: Some(service_revision_name(service)),
        active_instances,
        in_flight,
        url: Some(service_url(service)),
        last_invoked_at: Some(Utc::now()),
        last_error,
    }
}

struct HttpResponse {
    status: u16,
    body: String,
}

async fn post_json(host: String, port: u16, path: String, input: Value) -> Result<HttpResponse> {
    tokio::task::spawn_blocking(move || post_json_blocking(&host, port, &path, &input))
        .await
        .context("serverless runtime HTTP task failed")?
}

async fn get_status(host: String, port: u16, path: String) -> Result<u16> {
    tokio::task::spawn_blocking(move || get_status_blocking(&host, port, &path))
        .await
        .context("serverless runtime health check task failed")?
}

fn get_status_blocking(host: &str, port: u16, path: &str) -> Result<u16> {
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("failed to connect to {host}:{port}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .context("failed to set read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .context("failed to set write timeout")?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    )
    .context("failed to write serverless health check")?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .context("failed to read serverless health check response")?;
    Ok(parse_http_response(&response)?.status)
}

fn post_json_blocking(host: &str, port: u16, path: &str, input: &Value) -> Result<HttpResponse> {
    let body = serde_json::to_string(input).context("failed to serialize serverless input")?;
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("failed to connect to {host}:{port}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .context("failed to set read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .context("failed to set write timeout")?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .context("failed to write serverless request")?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .context("failed to read serverless response")?;
    parse_http_response(&response)
}

fn parse_http_response(bytes: &[u8]) -> Result<HttpResponse> {
    let response = String::from_utf8_lossy(bytes);
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed HTTP response from serverless runtime"))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| anyhow!("missing HTTP status from serverless runtime"))?
        .parse::<u16>()
        .context("invalid HTTP status from serverless runtime")?;
    Ok(HttpResponse {
        status,
        body: body.to_string(),
    })
}

fn pod_pick_index(len: usize) -> usize {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as usize % len)
        .unwrap_or(0)
}
