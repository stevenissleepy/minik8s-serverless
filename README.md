# Minik8s Serverless

这是一个独立的 Minik8s 扩展仓库。Rust workspace 通过固定 Git commit 依赖
Minik8s 的 API、客户端和基础类型，因此不要求与 Minik8s 源码放在相邻目录。

```sh
cargo build --workspace
cargo test --workspace
```

这个插件采用一个简化版 Knative-like 结构：

- Function 层：`kn func create/deploy` 面向本地源码，负责创建函数模板、构建每个函数自己的 OCI image、push image，然后通过 apiserver 提交 `ServerlessService`。
- Serving 层：`serverless-controller` watch `ServerlessService` 和 `Revision`，创建/维护底层 Revision、Pod 和 Service；`serverless-activator` 是独立数据面入口，处理 invoke/event/workflow 请求、发布扩缩容意图并等待冷启动 Pod 就绪。serverless-controller 不 build image、不 pull image，也不直接运行容器。

## 安装

Serverless 插件需要先注册 CRD，再启动 serverless-controller 和 serverless-activator：

```sh
kubectl apply -f deploy/serverless-crds.yaml
kubectl apply -f deploy/serverless-core.yaml
```

这些文件的作用是：

- `deploy/serverless-crds.yaml`：注册 Serverless 插件用到的 CRD，包括 `ServerlessService`、`Revision`、`EventTrigger`、`EventSource` 和 `Workflow`。
- `ServerlessService` 描述一个 serverless 服务，例如 image、port、环境变量、扩缩容配置。
- `Revision` 记录某次 image/template 对应的版本。
- `EventTrigger` 用于把事件源绑定到函数或 workflow。
- `EventSource` 描述一个会自动产生事件的源，例如定时事件或文件变化事件。
- `Workflow` 用于描述函数调用链和分支。
- `serverless-core.yaml`：启动 `serverless-controller` 和 `serverless-activator`。serverless-controller watch 上面的资源并调谐 Revision、Pod、Service；serverless-activator 作为独立入口处理 invoke、冷启动等待、并发扩容意图和 scale-to-0。

也就是说，apply 多个文件不是为了“多启动几个程序”，而是在给 apiserver 注册新的资源类型，并启动 serverless-controller/serverless-activator 两个职责不同的组件。当前核心组件是 2 个 Pod、2 个 container、2 个 image：`ghcr.io/stevenissleepy/serverless-controller:latest` 只包含 controller 二进制，`ghcr.io/stevenissleepy/serverless-activator:latest` 只包含 activator 二进制。没有 CRD，apiserver 不认识 `ServerlessService`；没有 serverless-controller，资源只会存在 etcd 里，不会真的创建函数 Pod；没有 serverless-activator，请求入口和冷启动流量承接不存在。

## 从源码部署

```sh
kn func create -l python hello
cd hello
# 修改 function/func.py；生成的 Dockerfile 会运行 function/app.py
kn func deploy --registry ghcr.io/myname --api-server http://127.0.0.1:8080
```

Rust 函数使用同一套 Function 流程：

```sh
kn func create -l rust hello-rs
cd hello-rs
# 修改 src/function.rs；kn deploy 会先本地构建静态 musl 二进制，再生成 scratch 运行镜像
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

## 从已有镜像部署

```sh
kn service create hello \
  --image ghcr.io/myname/hello:latest \
  --port 8080 \
  --api-server http://127.0.0.1:8080
```

这种方式会跳过 Function 层，不构建源码，直接把已有 image 交给 Serving 层运行。

## 扩缩容配置

`spec.scale` 支持以下字段（`func.yaml` 的 `scale` 与之一一对应）：

| 字段 | 默认 | 含义 |
|------|------|------|
| `minScale` | 0 | 最低实例数；0 表示空闲时缩容到零 |
| `maxScale` | 10 | 并发扩容的实例上限 |
| `idleSeconds` | 60 | 无请求多少秒后缩回 `minScale` |

冷启动时 Activator 等待 Pod 就绪的上限固定为 300 秒（对齐 Knative revision `timeoutSeconds` 的默认值），足够覆盖加载模型权重这类慢启动函数；调用方的客户端超时要按函数实际冷启动时长设置。

## 调用函数

函数有两种触发方式，对应 lab 要求的「HTTP 请求」和「绑定事件触发」。

### 1. HTTP 请求

```sh
curl -s http://127.0.0.1:8082/api/v1/namespaces/default/services/hello/invoke \
  -H 'content-type: application/json' \
  -d '{"name":"minik8s"}'
```

### 2. 事件触发

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
