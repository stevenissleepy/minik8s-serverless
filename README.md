# Minik8s Serverless

这个插件采用一个简化版 Knative-like 结构

- Function 层：`kn func create/deploy` 面向本地源码，负责创建函数模板、构建每个函数自己的 OCI image、push image，然后通过 apiserver 提交 `ServerlessService`。
- Serving 层：`serverless-controller` watch `ServerlessService` 和 `Revision`，创建/维护底层 Revision、Pod 和 Service；`serverless-activator` 是独立数据面入口，处理 invoke/event/workflow 请求、发布扩缩容意图并等待冷启动 Pod 就绪。serverless-controller 不 build image、不 pull image，也不直接运行容器。


## Install

在一个可以使用 kubectl 连接到 Minik8s 集群的机器上

首先从 GitHub Release 安装 `kn`

```sh
tag=v0.1.0
deb="kn-${tag}-linux-amd64.deb"
curl -fLO "https://github.com/stevenissleepy/minik8s-serverless/releases/download/${tag}/${deb}"
sudo apt install "./${deb}"
```

然后下载部署清单，注册 Serverless CRD，并启动 `serverless-controller` 和 `serverless-activator`：

```sh
tag=v0.1.0
base="https://raw.githubusercontent.com/stevenissleepy/minik8s-serverless/${tag}/deploy"
curl -fLO "${base}/serverless-crds.yaml"
curl -fLO "${base}/serverless-core.yaml"
kubectl apply -f serverless-crds.yaml
kubectl apply -f serverless-core.yaml
```


## Usage

### 从 Function 部署

部署一个 Python Function

```sh
kn func create -l python hello
cd hello
# 修改 function/func.py
kn func deploy --registry ghcr.io/myname --api-server http://127.0.0.1:8080
```

部署一个 Rust Function

```sh
kn func create -l rust hello-rs
cd hello-rs
# 修改 src/function.rs
# kn deploy 会先本地构建静态 musl 二进制，再生成 scratch 运行镜像
# 首次使用前需要安装对应 target，例如：rustup target add x86_64-unknown-linux-musl
kn func deploy --registry ghcr.io/myname --api-server http://127.0.0.1:8080
```

`kn func deploy` 的流程是：

```text
本地函数目录
  -> 使用目录里的 Dockerfile 构建 image
  -> push 到 registry
  -> 创建/更新 ServerlessService
  -> serverless-controller 创建 Revision 和 Service
  -> serverless-activator 收到请求后发布 desired scale，serverless-controller 创建 runtime Pod
```

### 从已有镜像部署

```sh
kn service create hello \
  --image ghcr.io/myname/hello:latest \
  --port 8080 \
  --api-server http://127.0.0.1:8080
```

这种方式会跳过 Function 层，不构建源码，直接把已有 image 交给 Serving 层运行。


## 调用函数

函数有两种触发方式，分别为「HTTP 请求」和「绑定事件触发」。

### HTTP 请求

```sh
curl -s http://127.0.0.1:8082/api/v1/namespaces/default/services/hello/invoke \
  -H 'content-type: application/json' \
  -d '{"name":"minik8s"}'
```

### 事件触发

事件链路对齐 Knative 的三段式 **Source → Broker → Trigger → 函数**：

- **Source（`EventSource`）**：会自动产生事件的源，对应 Knative 的 Source。
- **Broker**：Activator 上的 `POST /api/v1/events/:type`，按事件类型扇出。
- **Trigger（`EventTrigger`）**：把某个事件类型订阅到函数 / Workflow。

事件在各段之间以 [CloudEvents 1.0](https://cloudevents.io/) 信封传递（`type` / `source` / `id` / `time` / `data`）；函数收到的是 `data` 负载，因此**同一个函数无需改动即可被 HTTP 和事件两种方式调用**。

`EventSource` 目前支持两类源：

| 类型 | 字段 | 行为 | 对齐 Knative |
|------|------|------|------|
| `ping` | `intervalSeconds` 或 `schedule`(cron) + `data` | 定时产生事件 | `PingSource` |
| `file` | `path` + `intervalSeconds` | 轮询文件 mtime，变化即产生事件 | 自定义 Source |

任何外部系统直接 `POST /api/v1/events/:type` 也是一种「自定义事件源」。

```sh
# 自定义事件源：手动发布一个事件，触发订阅了 comment.created 的函数
curl -s http://127.0.0.1:8082/api/v1/events/comment.created \
  -H 'content-type: application/json' \
  -d '{"text":"good comment"}'
```

复杂应用中的 Trigger 示例见 [`examples/exp-ticket-support/event-trigger.yaml`](examples/exp-ticket-support/event-trigger.yaml)。


## 查看状态

```sh
kubectl get serverlessservices
kubectl get revisions
kubectl get service sks-hello
kubectl get pods -l serverless.minik8s.io/service=hello -o wide
```


## 扩缩容配置

`spec.scale` 支持以下字段（`func.yaml` 的 `scale` 与之一一对应）：

| 字段 | 默认 | 含义 |
|------|------|------|
| `minScale` | 0 | 最低实例数；0 表示空闲时缩容到零 |
| `maxScale` | 10 | 并发扩容的实例上限 |
| `idleSeconds` | 60 | 无请求多少秒后缩回 `minScale` |

冷启动时 Activator 等待 Pod 就绪的上限固定为 300 秒（对齐 Knative revision `timeoutSeconds` 的默认值），足够覆盖加载模型权重这类慢启动函数；调用方的客户端超时要按函数实际冷启动时长设置。


## 完整测试

完整的测试方法参见 [test.md](docs/test.md)
