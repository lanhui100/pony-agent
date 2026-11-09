# r3-dim3 根因分析：失败语义（failure semantics）

- 轮次：`r3-dim3`｜维度：3（失败语义）
- 基线：`d9425a70ba3f36d8ab5ca0124c567093f3fd6788`（r2-dim1 attempt2 已合入）
- 取证回执：`receipts/001-collect-dim3.pa01a1ba70.json`
- 断言脚本：`scripts/audit/assertions/dim3.sh`（红相已实测，见文末「红相机器证据」）
- 会话总数：15（nominal 9 / provider-fault 3 / timeout-fault 3）

## 0. 判定口径

维度 3 要回答的是**「一轮失败之后，三侧还认不认得出这是失败」**。三侧指：

| 侧 | 真值来源 |
| :-- | :-- |
| trace | Langfuse 根 span 属性（`agent.turn.phase` / `agent.turn.outcome` / `agent.turn.fallback_reason`） |
| 封套 | headless `pony.headless.result.v1`（`status` / `exit_code` / `failure_reason`） |
| 事件流 | JSONL 终态事件（`turn:completed` / `turn:failed` / `headless:timeout`） |

任何一侧单独「不知道」可以接受；**三侧同时把失败说成成功**不可接受。

## 1. 汇总事实（metrics.dim3.json）

| 臂 | 会话 | trace 终态 | phase | outcome | 封套 status/exit | 事件终态 | provider 回退 |
| :-- | --: | --: | :-- | :-- | :-- | --: | --: |
| nominal-a0 | 9 | 9 | completed ×9 | `<absent>`×4 / degraded×5 | completed/0 ×9 | 9 | 0 |
| fault-provider-a0 | 3 | 3 | completed ×3 | `<absent>`×3 | completed/0 ×3 | 3 | **3** |
| fault-timeout-a0 | 3 | **0** | `<absent>`×3 | `<absent>`×3 | timeout/7 ×3 | **0**（仅 `headless:timeout`） | 0 |

## 2. F1（blocker）：provider 回退被三侧同时抹成「纯成功」

### 现象（3/3 可复现）

`fault-provider-a0` 用死地址网关（`http://127.0.0.1:9/v1`）制造 follow-up 调用失败，runtime 走本地兜底摘要后继续完成本轮。结果：

```
session  pa_01a1be05-485e-7f71-ae94-75ba66a049d0
trace    16a0b3c09daef1cd6228731dba6e9d6c
事件流   turn:completed（携带 fallbackReason: provider_followup_failed: …；providerMode=fallback）
trace    phase=completed，outcome=<absent>，无 fallback_reason
封套     status=completed，exit_code=0，failure_reason=null
```

也就是说：**唯一说明「provider 调用失败过」的地方是中间事件**（`turn:hop_complete` / `turn:output_end` /
`turn:phase_changed` / `turn:checkpoint_persisted` / `turn:completed` 的 `fallbackReason`），
而三侧**终态**全部报成功。下游按 trace/封套做统计时，这次真实失败不可见。

### 根因（定位到行）

| 环节 | 代码位置 | 事实 |
| :-- | :-- | :-- |
| 回退产生 | `crates/pony-agent-core/src/agent/provider/mod.rs:4776` | `local_tool_followup_fallback_response` 写 `provider_mode="fallback"`、`provider_source="provider_followup_local_fallback"`、`fallback_reason=Some("provider_followup_failed: …")` |
| 回退累积 | `crates/pony-agent-core/src/agent/runtime/turn_sync.rs:41/427/513/553` | `accumulated_fallback_reason` 一路 merge 到 `turn.completed` 事件，**仅此一个消费方** |
| 终态聚合 | `crates/pony-agent-core/src/agent/otel/mod.rs:2353-2402` | 根 span 的 `outcome` 只由三件事推导：`phase`、`take_active_turn_error()`、`tool_failures`。**provider 回退不参与** |
| 错误登记 | `crates/pony-agent-core/src/agent/otel/mod.rs:2971/3057` | `mark_active_turn_error` 只有两个调用点（`quota_blocked` / `tool_failed`）⇒ 回退从未登记为 turn 级错误 |
| 封套 | `crates/pony-agent-core/src/bin/cli.rs:753-772`（`ResultEnvelope` 装配） | 封套只取 `facts.failure_reason`，而 sink 的 `TurnFacts` 只在 `turn:failed/cancelled/suspended` 分支填它（`cli.rs:545-572`）⇒ `turn:completed` 携带的 `fallbackReason` 被丢弃 |

结论：**信号存在、链路断在两处**（otel 终态聚合、CLI 封套装配），不是「无信号可采」。

### 影响

- 失败率统计系统性偏低：provider 全挂时，9/9 会话可能仍报 100% 成功；
- 成本/质量归因错位：兜底摘要的 token 用量被算作正常调用；
- 与 r1-dim6 A3 同族但不同键：dim6 解决的是「同一轮两个结局」，本轮是「有失败却无结局」。

## 3. F2（major）：headless 超时在 trace 侧完全不可判定

### 现象（0/3）

`fault-timeout-a0`（`PONY_AUDIT_TIMEOUT_SECS=1`）三会话：

```
s1  trace 52c67b0c…  root_attributes={}      （只有 tool 子 observation）
s2  trace <缺失>                              （Langfuse 按 sessionId 查不到任何 trace）
s3  trace 9daf7eb1…  root_attributes={}
事件流  turn:started … headless:timeout（宿主命名空间），无任何 turn:* 终态
封套    status=timeout / exit_code=7 / failure_reason=null
```

### 根因（结构性，不是接线漏了）

| 环节 | 代码位置 | 事实 |
| :-- | :-- | :-- |
| 宿主超时 | `crates/pony-agent-core/src/bin/cli.rs:603-614` | `host_timeout_marker` 只生成**事件流**标记（`kind=headless:timeout`），注释明确「不表示 turn 失败」 |
| 宿主发射 | `crates/pony-agent-core/src/bin/cli.rs:725-731` | 只在 `status == Timeout && output_format != Text` 时 `sink.emit`，**没有任何 OTel 出口** |
| 根 span 归属 | `crates/pony-agent-core/src/agent/otel/mod.rs:2348-2410`（`TurnTraceGuard::drop`） | 根 span 由 worker 线程的守卫 `drop` 收尾；worker 阻塞在 provider 读上**不可抢占** ⇒ 进程退出前根 span 永不结束 ⇒ 永不导出 |
| 采集期望 | `scripts/audit/collect_dim3.py:host_timeout_spans` | 采集器按 `headless.timeout` **observation** 取宿主终态，而产品从未产出该 span ⇒ 恒返回 `[]` |

`s2` 连 trace 都没有，是「根 span 未结束 + 子 span 未导出/未被 Langfuse 聚合」共同结果；r2-dim1 F1 的退出 flush 只保证「已入队 span 发完」，救不了「span 从未结束」。

### 影响

- 超时（宿主最该被观测的一种故障）在 trace 侧 0 命中：既无法告警，也无法统计超时率；
- 与封套/事件三侧无法对拍：trace 说「没有这一轮」，封套说「timeout」。

## 4. F3（minor）：超时封套 `failure_reason=null`

`status=timeout` 但 `failure_reason` 为 null（3/3）。`classify_failure` 只在 `Failed` 分支被调用
（`cli.rs:735-739`），`Timeout` 走 `headless::exit_code_for`，从不填原因。封套契约里
`failure_reason` 是「失败/挂起的 wire 原文」，超时属失败，却拿不到任何原因串 —— 机器消费方
必须自己把 `status=timeout` 二次翻译成原因。

## 5. 明确不列为缺陷（避免把 by-design 当 bug）

| 事实 | 判定 |
| :-- | :-- |
| nominal 9 会话中 4 个 `outcome=<absent>` | **by design**（`otel/mod.rs:3302`：无错误不输出该键）。维度 3 只需 `phase=completed` 即可定结局，不要求每轮都写 outcome |
| A3 三侧结局无矛盾（0 冲突） | 当前未触发：provider 回退臂三侧**一致地错**（都报成功），矛盾检测抓不到，靠 A1 抓 |
| A4 控制组不误报（0 违规） | 4 个 clean 会话未被误标降级，r1-dim6 F3 的 `degraded` 判定口径正确 |
| `turn:trace` / `turn:tool` 等过程事件形态 | 与维度 3 无关，不动 |

## 6. 红相机器证据（未修复代码上实测）

```
$ bash scripts/audit/assertions/dim3.sh \
    --arm-dir docs/audit/rounds/r3-dim3/replay/fault-provider-a0 \
    --expected-cwd /tmp/audit-r3-fault-provider-a0 \
    --metrics docs/audit/rounds/r3-dim3/metrics.dim3.json
   sessions=15 by_arm={'nominal-a0': 9, 'fault-provider-a0': 3, 'fault-timeout-a0': 3}
   A1 FAIL violations=3      # 三条回退会话：trace.outcome=None / 无 fallback_reason / 封套无 degraded_reason
   A2 FAIL violations=3      # 三条超时会话：封套 failure_reason=None / trace 无终态（s2 整条缺失）
   A3 PASS violations=0
   A4 PASS violations=0
dim3: FAIL（断言超阈）
exit=1
```

节点自测（`bash scripts/audit/selftest.sh`）：**pass=49 fail=0 exit=0**，含 dim3 绿相/三条红相/两条参数校验。

## 7. 未知与风险（须在实施中先证伪再全量重放）

1. **超时宿主 span 能否落进同一 trace**：宿主可按 `turn_id` 回查导出态 trace_id
   （`otel/mod.rs:3886 exported_trace_id_for_turn`），但要作为**子 span** 必须同时拿到根 span_id；
   当前只登记了 trace_id。方案是扩登记为 `(trace_id, span_id)` 并用 `Context` 作父启动 span。
   若 Langfuse 对「父 span 从未出现」的 observation 处理异常，需退化为「独立 host trace + 同
   `langfuse.session.id`」，届时 A2 的判据需按「按 session 扫全部 trace」扩展 —— 这一步必须先用
   单会话探针证伪/证实，不得直接全量重放。
2. **worker 仍活着**：超时后进程退出前 worker 可能继续产生 span。宿主 span 与 worker span 的
   顺序不确定，A2 只要求「至少一个终态可判定」，不要求唯一。
3. **封套新增键的兼容性**：`degraded_reason` 走 `skip_serializing_if = Option::is_none`，
   仅在降级时出现（不改变成功轮的 wire 字节）。若实施中发现有消费方按「键集固定」解析，
   回滚路径见提案。
