# Minik8s Serverless 测试内容

## 启动 serverless

启动一台 `control-plane` 和两台 `node`，先确认三个 Node 都 Ready，且每个 Node 都已经分配
`spec.podCIDR`：

```sh
kubectl get nodes -o wide
```

部署网络和 Serverless 插件；kube-proxy 由 `kubeadm init` 自动安装：

```sh
kubectl apply -f deploy/addons/kube-flannel.yaml
kubectl apply -f crates/plugin/serverless/deploy/serverless-crds.yaml
kubectl apply -f crates/plugin/serverless/deploy/serverless-core.yaml
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
- `kube-system/serverless-controller` 和 `kube-system/serverless-activator` 在 control-plane 上 Running。

## Serving 层

Serving 层直接使用已有镜像，不经过 Function 层。

```sh
CONTROL_PLANE=<control-plane-ip>

kn service create sentiment-it \
  --image stevenissleepy/sentiment-it:latest \
  --port 8080 \
  --api-server http://$CONTROL_PLANE:8080
```

对应的 YAML 示例在 `crates/plugin/serverless/examples/exp-sentiment/sentiment.yaml`。

调用函数：

```sh
curl -s http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/sentiment-it/invoke \
  -H 'content-type: application/json' \
  -d '{"text":"good image"}' | jq
```

查看资源和运行状态：

```sh
kubectl get serverlessservices
kubectl get revisions
kubectl get service
kubectl get pods -l serverless.minik8s.io/service=sentiment-it -o wide
curl -s http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/sentiment-it/state | jq
```

期望：

- 返回 JSON 中 `.result.label` 为 `positive`。
- serverless-controller 创建了 `Revision/sentiment-it-*`、`Service/sks-sentiment-it` 和 `sks-sentiment-it-*` runtime Pod；HTTP 请求入口经过 `serverless-activator`。
- 每个 `ServerlessService` 的用户函数运行在独立 runtime Pod 中。

## Function 层

Function 层从本地 Python 函数目录上传函数。这里固定使用
`stevenissleepy/sentiment-it:latest` 镜像。

```sh
CONTROL_PLANE=<control-plane-ip>
REGISTRY=stevenissleepy

rm -rf sentiment-it
kn func create -l python sentiment-it
cp crates/plugin/serverless/examples/exp-sentiment/sentiment.py sentiment-it/function/func.py
kn func deploy sentiment-it --registry $REGISTRY --api-server http://$CONTROL_PLANE:8080
```

查看上传后的函数：

```sh
kubectl get serverlessservices
kubectl get revisions
curl -s http://$CONTROL_PLANE:8080/apis/serverless.minik8s.io/v1alpha1/namespaces/default/serverlessservices/sentiment-it | jq '.spec, .status'
```

期望：

- `kn func create -l python` 能创建 Python 函数模板。
- `kn func deploy` 创建或更新 `ServerlessService/sentiment-it`。
- `ServerlessService/sentiment-it` 的 `spec.image` 是 `stevenissleepy/sentiment-it:latest`。

## 集成测试

下面的流程用于从一个已经启动 Serverless 插件的三节点集群上做端到端验证。它先通过
Function 层构建并 push 函数镜像，再由 Serving 层运行这个镜像，最后验证事件、状态和
scale-to-zero。

```sh
CONTROL_PLANE=<control-plane-ip>
REGISTRY=stevenissleepy

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

- 三个 Node、Flannel、kube-proxy、`serverless-controller` 和 `serverless-activator` 都是 Running。
- `kn func deploy` 创建或更新 `ServerlessService/sentiment-it`。
- `ServerlessService/sentiment-it` 的 `spec.image` 是 `stevenissleepy/sentiment-it:latest`。
- invoke 返回 JSON，且 `.result.label` 为 `positive`。
- serverless-controller 创建了 `Revision/sentiment-it-*`、`Service/sks-sentiment-it` 和 runtime Pod，serverless-activator 负责请求承接和冷启动等待。
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

Workflow 如下：

```txt
+----------------------+
| classify             |
| ticket-classify      |
+----------+-----------+
           |
           v
+----------------------+
| score                |
| risk-score           |
+----------+-----------+
           |
           v
     +-----+------+
     | decision   |
     | == human ? |
     +--+------+--+
        |      |
   yes  |      | no
        |      |
        v      v
+----------+  +------------+
| human    |  | auto       |
| escalate |  | reply      |
+----+-----+  +------+-----+
     |               |
     +-------+-------+
             |
             v
     +---------------+
     | notify        |
     | final result  |
     +---------------+
```

测试命令：

```sh
CONTROL_PLANE=<control-plane-ip>
REGISTRY=stevenissleepy

# deploy 五个 container
rm -rf ticket-classify
kn func create -l rust ticket-classify
cp crates/plugin/serverless/examples/exp-ticket-support/functions/ticket_classify.rs ticket-classify/src/function.rs
kn func deploy ticket-classify --registry "$REGISTRY" --api-server "http://$CONTROL_PLANE:8080"

rm -rf risk-score
kn func create -l python risk-score
cp crates/plugin/serverless/examples/exp-ticket-support/functions/risk_score.py risk-score/function/func.py
kn func deploy risk-score --registry "$REGISTRY" --api-server "http://$CONTROL_PLANE:8080"

rm -rf auto-reply
kn func create -l rust auto-reply
cp crates/plugin/serverless/examples/exp-ticket-support/functions/auto_reply.rs auto-reply/src/function.rs
kn func deploy auto-reply --registry "$REGISTRY" --api-server "http://$CONTROL_PLANE:8080"

rm -rf human-escalate
kn func create -l rust human-escalate
cp crates/plugin/serverless/examples/exp-ticket-support/functions/human_escalate.rs human-escalate/src/function.rs
kn func deploy human-escalate --registry "$REGISTRY" --api-server "http://$CONTROL_PLANE:8080"

rm -rf notify
kn func create -l rust notify
cp crates/plugin/serverless/examples/exp-ticket-support/functions/notify.rs notify/src/function.rs
kn func deploy notify --registry "$REGISTRY" --api-server "http://$CONTROL_PLANE:8080"

# 创建 workflow
kubectl apply -f crates/plugin/serverless/examples/exp-ticket-support/workflow.yaml
kubectl apply -f crates/plugin/serverless/examples/exp-ticket-support/event-trigger.yaml
kubectl get serverlessservices
kubectl get workflows
kubectl get eventtriggers

# 第一次调用前还没有 runtime Pod
kubectl get pods -l serverless.minik8s.io/managed-by=serverless-controller -o wide

# 直接调用 workflow，低风险工单走 auto-reply
curl -s "http://$CONTROL_PLANE:30082/api/v1/namespaces/default/workflows/ticket-router/invoke" \
  -H 'content-type: application/json' \
  -d '{"ticket_id":"T-001","user_level":"normal","text":"password login error"}' | jq

# 通过事件触发 workflow，高风险工单走 human-escalate
curl -s "http://$CONTROL_PLANE:30082/api/v1/events/ticket.created" \
  -H 'content-type: application/json' \
  -d '{"ticket_id":"T-002","user_level":"vip","text":"angry terrible complaint refund security leak"}' | jq
```

期望：

- `kubectl get serverlessservices` 能看到 `ticket-classify`、`risk-score`、`auto-reply`、`human-escalate` 和 `notify` 五个函数。
- `kubectl get workflows` 能看到 `ticket-router`，`kubectl get eventtriggers` 能看到 `ticket-created`。
- 低风险直接调用返回的 `.trace[].step` 依次包含 `classify`、`score`、`auto`、`notify`，最终 `.result.action` 为 `auto-reply`。
- 高风险事件触发返回的 `.delivered` 为 `1`，`.results[0].target_kind` 为 `Workflow`，最终 `.results[0].result.action` 为 `human-escalate`。
- `risk-score` 是模型类 Workload；它根据工单文本、分类和用户等级计算 `risk`、`risk_level` 和 `decision`，下游 `auto-reply` 或 `human-escalate` 会消费这些字段。

## 复杂应用补充验收

### 更新函数并区分新旧结果

先调用旧版 `risk-score`，记录更新前输出：

```sh
RISK_URL="http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/risk-score/invoke"

curl -s "$RISK_URL" \
  -H 'content-type: application/json' \
  -d '{"ticket_id":"U-001","user_level":"vip","category":"complaint","text":"angry security leak"}' | jq '.result'
```

将复杂应用中的 `risk-score` 更新为 v2。v2 示例代码放在
`crates/plugin/serverless/examples/exp-ticket-support/functions/risk_score_v2.py`，
它会返回 `model_version` 和 `instance`，并支持 `sleep_ms` 参数。

```sh
cp crates/plugin/serverless/examples/exp-ticket-support/functions/risk_score_v2.py \
  risk-score/function/func.py

kn func deploy risk-score \
  --image "$REGISTRY/risk-score:v2" \
  --api-server "http://$CONTROL_PLANE:8080"

kn service create risk-score \
  --image "$REGISTRY/risk-score:v2" \
  --port 8080 \
  --max-scale 4 \
  --target-concurrency 1 \
  --api-server "http://$CONTROL_PLANE:8080"

kubectl get serverlessservices
kubectl get revisions
curl -s "http://$CONTROL_PLANE:8080/apis/serverless.minik8s.io/v1alpha1/namespaces/default/serverlessservices/risk-score" \
  | jq '.spec.image, .spec.scale, .spec.concurrency, .status.latestRevision'

curl -s "$RISK_URL" \
  -H 'content-type: application/json' \
  -d '{"ticket_id":"U-002","user_level":"vip","category":"complaint","text":"angry security leak"}' | jq '.result'
```

期望：

- 更新前的 `risk-score` 输出没有 `model_version`。
- 更新后 `ServerlessService/risk-score` 的 `.spec.image` 为 `$REGISTRY/risk-score:v2`。
- `kubectl get revisions` 能看到新的 `risk-score-*` revision。
- 更新后的调用结果包含 `"model_version": "risk-v2"`，且 `risk`、`decision` 仍由输入参数计算得出。

### 扩容到多个实例并验证请求分布

使用更新后的 `risk-score` 直接承压。它仍然是复杂应用 Workflow 中的模型函数；
这里直接调用函数入口，是为了稳定观察该函数本身的扩容行为。

```sh
STATE_URL="http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/risk-score/state"

rm -rf /tmp/risk-score-load
mkdir -p /tmp/risk-score-load

for i in $(seq 1 20); do
  (
    curl -s "$RISK_URL" \
      -H 'content-type: application/json' \
      -d "{\"ticket_id\":\"LOAD-$i\",\"user_level\":\"vip\",\"category\":\"complaint\",\"text\":\"angry urgent security leak\",\"sleep_ms\":5000}" \
      > "/tmp/risk-score-load/$i.json"
  ) &
done

sleep 1
curl -s "$STATE_URL" | jq '.runtime'
kubectl get pods -l serverless.minik8s.io/service=risk-score -o wide

wait

jq -c '.result | {ticket_id,risk,decision,model_version,instance}' /tmp/risk-score-load/*.json
curl -s "$STATE_URL" | jq '.runtime'

# 等待扩容出的 4 个 runtime Pod 都 Ready，再重新发送一批请求验证多实例分发。
for i in $(seq 1 40); do
  kubectl get pods -l serverless.minik8s.io/service=risk-score -o wide
  READY_COUNT=$(kubectl get pods -l serverless.minik8s.io/service=risk-score -o wide \
    | awk 'NR>1 && $2=="1/1" && $3=="Running" {c++} END {print c+0}')
  [ "$READY_COUNT" -ge 4 ] && break
  sleep 3
done

rm -rf /tmp/risk-score-warm
mkdir -p /tmp/risk-score-warm

for i in $(seq 1 20); do
  (
    curl -s "$RISK_URL" \
      -H 'content-type: application/json' \
      -d "{\"ticket_id\":\"WARM-$i\",\"user_level\":\"vip\",\"category\":\"complaint\",\"text\":\"angry urgent security leak\",\"sleep_ms\":500}" \
      > "/tmp/risk-score-warm/$i.json"
  ) &
done
wait

jq -r '.result.instance' /tmp/risk-score-warm/*.json | sort | uniq -c
```

期望：

- 并发请求进行中时，`state.runtime.active_instances` 大于 `1`，通常会达到 `maxScale: 4`。
- `kubectl get pods -l serverless.minik8s.io/service=risk-score -o wide` 能看到多个 `risk-score` runtime Pod。
- 冷扩容第一批请求可能全部由最先 Ready 的实例处理；等待 4 个 runtime Pod 都 Ready 后重新发送请求，`jq -r '.result.instance' ... | sort | uniq -c` 至少出现两个不同的 `instance`，说明请求可以被多个函数实例处理。
- 扩容策略为：每个 `ServerlessService` 维护 `active_instances` 和 `in_flight`；当 `in_flight >= active_instances * concurrency.target` 且未超过 `scale.maxScale` 时增加一个实例。这里 `target: 1`、`maxScale: 4`，所以 20 个并发请求会触发多实例扩容。

### 并发量 20 压测

使用 `seq`、`xargs` 和 `curl` 对复杂应用中的 `risk-score` 做 20 并发压测：

```sh
rm -rf /tmp/risk-score-bench
mkdir -p /tmp/risk-score-bench

time seq 1 20 | xargs -P20 -I{} sh -c '
  curl -s "$0" \
    -H "content-type: application/json" \
    -d "{\"ticket_id\":\"BENCH-$1\",\"user_level\":\"vip\",\"category\":\"complaint\",\"text\":\"angry terrible refund security leak\",\"sleep_ms\":500}" \
    > "/tmp/risk-score-bench/$1.json"
' "$RISK_URL" {}

jq -s 'length as $total | map(select(.result.model_version == "risk-v2")) | length as $ok | {total: $total, ok: $ok}' \
  /tmp/risk-score-bench/*.json

curl -s "$STATE_URL" | jq '.runtime'
kubectl get pods -l serverless.minik8s.io/service=risk-score -o wide
```

期望：

- 压测命令中 `xargs -P20` 是并发量为 20 的参数材料。
- `time` 输出可作为压测耗时材料，`jq` 汇总中的 `total` 应为 `20`、`ok` 应为 `20`。
- 压测期间或压测刚结束时，`risk-score` 仍保持多个 runtime Pod，`state.runtime.active_instances` 不小于 `2`。

### 删除函数并展示删除后结果

删除复杂应用中的 `auto-reply` 函数，然后再次调用低风险 Workflow 分支：

```sh
kubectl delete serverlessservice auto-reply
sleep 8

kubectl get serverlessservices
kubectl get pods -l serverless.minik8s.io/service=auto-reply -o wide

curl -i -s "http://$CONTROL_PLANE:30082/api/v1/namespaces/default/services/auto-reply/invoke" \
  -H 'content-type: application/json' \
  -d '{"ticket_id":"D-001","category":"technical","risk":20,"decision":"auto"}'

curl -i -s "http://$CONTROL_PLANE:30082/api/v1/namespaces/default/workflows/ticket-router/invoke" \
  -H 'content-type: application/json' \
  -d '{"ticket_id":"D-002","user_level":"normal","text":"password login error"}'
```

期望：

- `kubectl get serverlessservices` 中不再出现 `auto-reply`。
- `kubectl get pods -l serverless.minik8s.io/service=auto-reply -o wide` 为空，说明该函数的 runtime Pod 已清理。
- 直接调用 `auto-reply` 返回 500，错误信息包含 `ServerlessService default/auto-reply not found`。
- 低风险 Workflow 会走到 `auto` 步骤，因此删除后再次调用 Workflow 也返回 500，并暴露缺失 `auto-reply` 的错误。

如需继续演示完整复杂应用，可以恢复该函数：

```sh
kn func deploy auto-reply --registry "$REGISTRY" --api-server "http://$CONTROL_PLANE:8080"
kubectl get serverlessservices
```
