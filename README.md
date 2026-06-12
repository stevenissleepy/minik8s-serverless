# Minik8s Serverless

这个插件采用一个简化版 Knative-like 结构：

- Function 层：`kn func create/deploy` 面向本地源码，负责创建函数模板、构建每个函数自己的 OCI image、push image，然后通过 apiserver 提交 `ServerlessService`。
- Serving 层：`serverless-controller` 面向已经存在的 image，watch `ServerlessService` 和 `Revision`，再创建/维护底层 Pod 和 Service。controller 不 build image、不 pull image，也不直接运行容器。

## 安装

最小 Serving 栈只需要安装下面三个资源：

```sh
kubectl apply -f crates/plugin/serverless/crds/serverlessservices.yaml
kubectl apply -f crates/plugin/serverless/crds/revisions.yaml
kubectl apply -f crates/plugin/serverless/deploy/serverless-controller.yaml
```

这些文件的作用是：

- `serverlessservices.yaml`：注册 `ServerlessService` 这个自定义资源，用来描述一个 serverless 服务，例如 image、port、环境变量、扩缩容配置。
- `revisions.yaml`：注册 `Revision` 这个自定义资源，用来记录某次 image/template 对应的版本。
- `serverless-controller.yaml`：启动真正干活的 controller。它 watch 上面的资源，然后创建 Pod、Service，并处理 invoke、冷启动、扩容和 scale-to-0。

如果需要事件触发或函数链，再安装可选资源：

```sh
kubectl apply -f crates/plugin/serverless/crds/eventtriggers.yaml
kubectl apply -f crates/plugin/serverless/crds/workflows.yaml
```

- `eventtriggers.yaml`：注册 `EventTrigger`，用于把事件源绑定到函数或 workflow。
- `workflows.yaml`：注册 `Workflow`，用于描述函数调用链和分支。

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
