# Minik8s Serverless 测试内容

## 启动 serverless

启动一台 `control-plane` 和两台 `node`，先确认三个 Node 都 Ready，且每个 Node 都已经分配
`spec.podCIDR`：

```sh
kubectl get nodes -o wide
```

部署网络、Service 转发和 Serverless 插件：

```sh
kubectl apply -f deploy/addons/kube-flannel.yaml
kubectl apply -f deploy/addons/kube-proxy.yaml
kubectl apply -f crates/plugin/serverless/deploy/serverless-crds.yaml
kubectl apply -f crates/plugin/serverless/deploy/serverless-controller.yaml
```

确认插件启动成功：

```sh
kubectl get nodes -o wide
kubectl get pods -A -o wide
```

期望：

- 三个 Node 都是 `Ready`。
- `kube-flannel` 下每个 Node 一个 Flannel Pod，均为 Running。
- `kube-system` 下每个 Node 一个 `kube-proxy` Pod，均为 Running。
- `kube-system/serverless-controller` 在 control-plane 上 Running。

## Serving 层

Serving 层直接使用已有镜像，不经过 Function 层。

```sh
CONTROL_PLANE=<control-plane-ip>

kn service create sentiment \
  --image stevenissleepy/sentiment:latest \
  --port 8080 \
  --api-server http://$CONTROL_PLANE:8080
```

对应的 YAML 示例在 `crates/plugin/serverless/examples/exp-sentiment/sentiment.yaml`。

调用函数：

```sh
curl -s http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/sentiment/invoke \
  -H 'content-type: application/json' \
  -d '{"text":"good image"}' | jq
```

查看资源和运行状态：

```sh
kubectl get serverlessservices
kubectl get revisions
kubectl get service
kubectl get pods -l serverless.minik8s.io/service=sentiment -o wide
curl -s http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/sentiment/state | jq
```

期望：

- 返回 JSON 中 `.result.label` 为 `positive`。
- controller 创建了 `Revision/sentiment-*`、`Service/sks-sentiment` 和 `sks-sentiment-*` runtime Pod。
- 每个 `ServerlessService` 的用户函数运行在独立 runtime Pod 中。

## Function 层

Function 层从本地 Python 函数目录上传函数。需要一个各节点都能 pull 的 `<registry>`。

```sh
CONTROL_PLANE=<control-plane-ip>
REGISTRY=<registry>

rm -rf sentiment
kn func create -l python sentiment
cp crates/plugin/serverless/examples/exp-sentiment/sentiment.py sentiment/function/func.py
kn func deploy sentiment --registry $REGISTRY --api-server http://$CONTROL_PLANE:8080
```

查看上传后的函数：

```sh
kubectl get serverlessservices
kubectl get revisions
curl -s http://$CONTROL_PLANE:8080/apis/serverless.minik8s.io/v1alpha1/namespaces/default/serverlessservices/sentiment | jq '.spec, .status'
```

期望：

- `kn func create -l python` 能创建 Python 函数模板。
- `kn func deploy` 创建或更新 `ServerlessService/sentiment`。
- `ServerlessService/sentiment` 的 `spec.image` 是 `$REGISTRY/sentiment:latest`。

## 集成测试

下面的流程用于从一个已经启动 Serverless 插件的三节点集群上做端到端验证。它先通过
Function 层构建并 push 函数镜像，再由 Serving 层运行这个镜像，最后验证事件、状态和
scale-to-zero。

```sh
CONTROL_PLANE=<control-plane-ip>
REGISTRY=<registry>

kubectl get nodes -o wide
kubectl get pods -A -o wide

rm -rf sentiment-it
kn func create -l python sentiment-it
cp crates/plugin/serverless/examples/exp-sentiment/sentiment.py sentiment-it/function/func.py
kn func deploy sentiment-it --registry $REGISTRY --api-server http://$CONTROL_PLANE:8080

curl -s http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/sentiment-it/invoke \
  -H 'content-type: application/json' \
  -d '{"text":"good image"}' | jq

kubectl get serverlessservices
kubectl get revisions
kubectl get service
kubectl get pods -l serverless.minik8s.io/service=sentiment-it -o wide
curl -s http://$CONTROL_PLANE:8080/apis/serverless.minik8s.io/v1alpha1/namespaces/default/serverlessservices/sentiment-it | jq '.spec.image, .status'
curl -s http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/sentiment-it/state | jq

sleep 75
curl -s http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/sentiment-it/state | jq
kubectl get pods -l serverless.minik8s.io/service=sentiment-it -o wide
```

通过标准：

- 三个 Node、Flannel、kube-proxy 和 `serverless-controller` 都是 Running。
- `kn func deploy` 创建或更新 `ServerlessService/sentiment-it`。
- `ServerlessService/sentiment-it` 的 `spec.image` 是 `$REGISTRY/sentiment-it:latest`。
- invoke 返回 JSON，且 `.result.label` 为 `positive`。
- controller 创建了 `Revision/sentiment-it-*`、`Service/sks-sentiment-it` 和 runtime Pod。
- 等待 `idleSeconds` 后，`state.runtime.active_instances` 回到 0，对应 runtime Pod 被删除。

## 复杂应用

复杂应用使用“智能客服工单处理系统”，相关代码放在
`crates/plugin/serverless/examples/exp-ticket-support/`。

应用包含 5 个函数：

- `ticket-classify`：判断工单类型，例如 refund、technical、complaint。
- `risk-score`：模型类 Workload，根据工单内容和用户等级计算风险分数。
- `auto-reply`：低风险工单自动回复。
- `human-escalate`：高风险工单转人工处理。
- `notify`：输出最终通知结果。

Workflow 定义在 `crates/plugin/serverless/examples/exp-ticket-support/workflow.yaml`：

```text
ticket-classify
  -> risk-score
  -> risk >= 80: human-escalate -> notify
  -> risk < 80: auto-reply -> notify
```

EventTrigger 定义在 `crates/plugin/serverless/examples/exp-ticket-support/event-trigger.yaml`，
用于把 `ticket.created` 事件触发到 `ticket-router` Workflow。
