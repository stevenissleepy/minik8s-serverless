# Minik8s Serverless

这个插件采用一个简化版 Knative-like 结构：

- Function 层：`kn func create/deploy` 面向本地源码，负责创建函数模板、构建每个函数自己的 OCI image、push image，然后通过 apiserver 提交 `ServerlessService`。
- Serving 层：`serverless-controller` 面向已经存在的 image，watch `ServerlessService` 和 `Revision`，再创建/维护底层 Pod 和 Service。controller 不 build image、不 pull image，也不直接运行容器。

## 安装

Serverless 插件需要先注册 CRD，再启动 controller：

```sh
kubectl apply -f crates/plugin/serverless/deploy/serverless-crds.yaml
kubectl apply -f crates/plugin/serverless/deploy/serverless-controller.yaml
```

这些文件的作用是：

- `crates/plugin/serverless/deploy/serverless-crds.yaml`：注册 Serverless 插件用到的 CRD，包括 `ServerlessService`、`Revision`、`EventTrigger` 和 `Workflow`。
- `ServerlessService` 描述一个 serverless 服务，例如 image、port、环境变量、扩缩容配置。
- `Revision` 记录某次 image/template 对应的版本。
- `EventTrigger` 用于把事件源绑定到函数或 workflow。
- `Workflow` 用于描述函数调用链和分支。
- `serverless-controller.yaml`：启动真正干活的 controller。它 watch 上面的资源，然后创建 Pod、Service，并处理 invoke、冷启动、扩容和 scale-to-0。

也就是说，apply 多个文件不是为了“多启动几个程序”，而是在给 apiserver 注册新的资源类型，并启动一个 controller 去调谐这些资源。没有 CRD，apiserver 不认识 `ServerlessService`；没有 controller，资源只会存在 etcd 里，不会真的创建函数 Pod。

## 从 Python 源码部署

```sh
kn func create -l python hello
cd hello
# 修改 function/func.py；生成的 Dockerfile 会运行 function/app.py
kn func deploy --registry ghcr.io/myname --api-server http://127.0.0.1:8080
```

`kn func deploy` 的流程是：

```text
本地函数目录
  -> 使用目录里的 Dockerfile 构建 image
  -> push 到 registry
  -> 创建/更新 ServerlessService
  -> serverless-controller 创建 Pod 和 Service
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

冷启动时网关等待 Pod 就绪的上限固定为 300 秒（对齐 Knative revision `timeoutSeconds` 的默认值），足够覆盖加载模型权重这类慢启动函数；调用方的客户端超时要按函数实际冷启动时长设置。

## 调用函数

```sh
curl -s http://127.0.0.1:8082/api/v1/namespaces/default/services/hello/invoke \
  -H 'content-type: application/json' \
  -d '{"name":"minik8s"}'
```

## 查看状态

```sh
kubectl get serverlessservices
kubectl get revisions
kubectl get service sks-hello
kubectl get pods -l serverless.minik8s.io/service=hello -o wide
```
