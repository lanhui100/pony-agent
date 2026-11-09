---
status: proposed
dimension: 3
dimension_name: failure.semantics（失败语义）
round: r3-dim3
attempt: 0
author_session: pa_01a1ba70-a314-7712-adf2-4bf675a96f99
created_unix: 1794191900
baseline_commit: d9425a70ba3f36d8ab5ca0124c567093f3fd6788
candidate_commit: ""
corpus_hash: pending-freeze-at-approval
assertion_hashes: pending-freeze-at-approval
evidence_receipts:
  - receipts/001-collect-dim3.pa01a1ba70.json
root_cause: root-cause.md
---

# r3-dim3 提案：失败语义（provider 回退 / headless 超时）

## 一句话

**三侧（trace / 封套 / 事件流）在「provider 回退」与「宿主超时」两种失败上，终态都在说成功或
干脆不说话；本提案把已有的失败信号接到终态上，并把超时终态从宿主侧落进 trace。**

## 冻结阈值（批准即冻结，事后放宽 = 新提案重审）

断言脚本：`scripts/audit/assertions/dim3.sh`（哈希随批准冻结，改动即 drift exit=4）

| 断言 | 口径 | 阈值 |
| :-- | :-- | :-- |
| **D1** 降级保真 | 事件流出现 `provider_followup_failed` 的会话：根 span `agent.turn.outcome=degraded` 且 `agent.turn.fallback_reason` 非空；封套 `degraded_reason` 以 `provider_followup_failed` 开头 | 违规会话数 = **0** |
| **D2** 超时终态可判定 | `result.status=timeout` 的会话：① 事件流有 `headless:timeout` 或 runtime 终态；② 封套 `failure_reason="timeout"`；③ trace 侧有终态 observation（根 span phase/outcome 或 `headless.timeout` span）；④ 不得整条 trace 缺失 | 违规会话数 = **0** |
| **D3** 三侧无矛盾 | `result.status ∈ {failed,timeout,cancelled,suspended}` ⇒ trace 不得 `phase=completed`、事件终态不得 `turn:completed`；`phase=completed` 不得配 `outcome=error` | 冲突会话数 = **0** |
| **D4** 控制组不误报 | 无 provider 回退且无工具失败的 `completed` 会话，不得出现 `outcome ∈ {degraded,error}` / `degraded_reason` / `failure_reason` | 误报会话数 = **0** |
| 证据完整性 | 封套或事件流未落盘 | **exit=2（UNKNOWN ≠ PASS）**，不折算 pass |

退出码：`0` 全绿｜`1` 断言超阈｜`2` 取证缺失｜`3` 环境不可用。

## 判定命令（批准后写入 frozen assertion_hashes）

```bash
# 单节点（候选臂）
bash scripts/audit/assertions/dim3.sh \
  --arm-dir docs/audit/rounds/r3-dim3/replay/nominal-a1 \
  --expected-cwd /tmp/audit-r3-nominal-a1 \
  --metrics docs/audit/rounds/r3-dim3/metrics.dim3.a1.json

# 唯一裁决者（含冻结/写域/棘轮）
bash scripts/audit/verdict.sh --round r3-dim3 --dim 3 \
  --arm-dir docs/audit/rounds/r3-dim3/replay/nominal-a1 \
  --expected-cwd /tmp/audit-r3-nominal-a1 \
  --metrics docs/audit/rounds/r3-dim3/metrics.dim3.a1.json
```

验证 corpus（三臂，共 15 会话，均为候选提交重放）：

| 臂 | 会话 | 采集 |
| :-- | --: | :-- |
| `nominal-a1` | 9 | 正常 9 任务会话（t01/t02/t03 × 3） |
| `fault-provider-a1` | 3 | `PONY_AUDIT_GATEWAY=http://127.0.0.1:9/v1` |
| `fault-timeout-a1` | 3 | `PONY_AUDIT_TIMEOUT_SECS=1` |

`metrics.dim3.a1.json` 由 `collect_dim3.py --arm nominal-a1=… --arm fault-provider-a1=… --arm fault-timeout-a1=…`
一次生成（同一份 metrics 同时服务 dim1/dim6 棘轮，字段为超集）。

## 修复方案

### F1（blocker）：provider 回退必须落成「降级完成」

**根因**：`provider_mode=fallback` + `fallback_reason` 只在 `turn_sync` 累积进事件；
otel 根 span 的 `outcome` 只由 `phase` / `take_active_turn_error()` / `tool_failures` 推导
（`otel/mod.rs:2353-2402`），`mark_active_turn_error` 仅有 `quota_blocked`、`tool_failed`
两个调用点（`otel/mod.rs:2971/3057`）⇒ 回退不进终态。

**改法（最小闭包，不新增契约类型）**：

1. `otel/mod.rs`
   - 新增常量 `ATTR_AGENT_TURN_FALLBACK_REASON = "agent.turn.fallback_reason"`；
   - 新增 `pub fn mark_active_turn_fallback(reason: &str)`：与 `mark_active_turn_error` 同构，
     落 `ActiveTurn.accumulated_fallback_reason`（**首条优先**，不覆盖更早的原因）；
   - `TurnTraceGuard::drop`：在取 `phase` 之前快照 fallback；`phase=completed` 且
     （`tool_failures > 0` 或 `fallback.is_some()`）⇒ `outcome=degraded`；
     有 fallback 时写 `ATTR_AGENT_TURN_FALLBACK_REASON`（复用既有 `preview_text` 截断/脱敏）。
   - 值域**不变**：仍是 `{error, degraded}`，`degraded` 只是把 provider 回退纳入既有语义。
2. `turn_sync.rs`：在累积点（`41/427/513/553` 之后、emit 终态之前）逐跳调用
   `mark_active_turn_fallback(&reason)`（门禁关闭时为空操作，不 panic）。
3. `headless.rs`：`ResultEnvelope` 新增 `degraded_reason: Option<String>`，
   `#[serde(skip_serializing_if = "Option::is_none")]` —— **只**在降级轮出现，成功轮 wire 字节不变。
4. `cli.rs`：`CliTurnSink` 观测到 `payload.fallbackReason`（`turn:completed` 已携带，无需改 runtime
   事件契约）⇒ 写 `TurnFacts.degraded_reason` ⇒ 装配进封套。

**不变量单测**：① 回退 + completed ⇒ `outcome=degraded` 且 fallback 属性存在；
② 回退 + phase=failed ⇒ `outcome` 仍 `error`（失败优先）；③ 无回退无工具失败 ⇒
不写 `outcome` / `degraded_reason`（控制组）。

### F2（major）：headless 超时的终态落进 trace

**根因**：宿主超时只发事件流标记（`cli.rs:603-614/725-731`），无 OTel 出口；根 span 由不可抢占的
worker 守卫收尾，进程退出前永不结束 ⇒ 永不导出（trace 侧 0/3，其中 s2 整条缺失）。

**改法**：

1. `otel/mod.rs`
   - `TURN_EXPORTED_TRACE_IDS` 由 `turn_id → trace_id` 扩为 `turn_id → (trace_id, span_id)`
     （同一张表、同一容量上限，键类型不变，仅有界值扩宽）；
   - 新增 `pub fn record_host_timeout(turn_id: &str, session_id: &str) -> bool`：
     按 `turn_id` 回查根 `SpanContext`，以它为父启动并**立即结束** span `headless.timeout`，
     属性：`agent.turn.id` / `langfuse.session.id` / `agent.turn.phase=timeout` /
     `agent.turn.outcome=timeout` / `agent.turn.failed_reason=timeout`。
     回查失败 ⇒ 返回 `false`（**绝不新建孤儿 trace 冒充**），由调用方 warn。
2. `cli.rs`：`status == TurnStatus::Timeout` 时无条件调用 `record_host_timeout`（与输出格式无关：
   trace 不该因 text 模式而缺失）；返回 false 时 `eprintln!` 一行 warn（fail-closed 可见）。

**必须的前置证伪（实施第一步，不通过则回退方案 B）**：单会话探针（`PONY_AUDIT_TIMEOUT_SECS=1`，
1 会话）验证 `headless.timeout` observation 能被 `GET /api/public/observations?traceId=…` 取到且
`phase/outcome` 可读。若 Langfuse 对「父 span 从未出现」的 observation 处理异常 ⇒ 退化为
**独立 host trace + 同 `langfuse.session.id`**，并把 `collect_dim3.py` 的 trace 定位从
`limit=1` 改为「按 session 扫全部 trace 取终态」。**该退化不改变任何阈值**（D2 仍要求终态可判定），
属实现自由度；一旦发生，须在回执中如实记录并重算 metrics。

### F3（minor）：超时封套必须带原因

`cli.rs` 装配封套时：`status == Timeout ⇒ failure_reason = Some("timeout")`（`Failed` 路径不触发，
沿用 `classify_failure` 原文）。语义仍符合封套契约「失败/挂起的 wire 原文」。

## 写域（批准即冻结；越界 = verdict exit=1）

```
crates/pony-agent-core/src/agent/otel/mod.rs
crates/pony-agent-core/src/agent/runtime/turn_sync.rs
crates/pony-agent-core/src/agent/headless.rs
crates/pony-agent-core/src/bin/cli.rs
crates/pony-agent-core/tests/
docs/observability.yaml
```

审计自身产物（`docs/audit/**`、`scripts/audit/**`）走白名单，不占写域。

**明确不做**：不改 `provider/mod.rs`（回退产生点语义正确）、不改 `src-tauri`（本维度不涉桌面端
退出路径）、不动 `dim1/dim6` 断言与既有属性值域。

## 文档与归属

- `docs/observability.yaml` 新增一段「归属登记（r3-dim3）」：四维归属不变（tracing applicable、
  metrics/logs 维持结构性 N-A —— 只改既有 OTLP 出口的属性与 span 形态，不新增对外接口/网络 IO/
  后台任务/LLM 调用）；按 ADR 0068 登记为 additive：
  - 新键 `agent.turn.fallback_reason`（仅回退轮出现）；
  - `agent.turn.outcome` 的 `degraded` 覆盖面扩大（provider 回退轮，**值域不变**）；
  - 新 span `headless.timeout`（宿主命名空间，属同一 trace）；
  - 封套新键 `degraded_reason`（仅降级轮出现）。
- 收口点（机械可查）：`otel/mod.rs` 单测三条不变量 + `headless.rs` 序列化测试 +
  `scripts/audit/assertions/dim3.sh`（冻结哈希）。

## 回滚路径

1. `git revert <本轮提交>` 即回到 `d9425a70`：所有改动均为 additive（新属性键、新 span、
   条件出现的封套键），回滚不改任何既有值域，旧消费方零感知。
2. 若仅 F2 需要回滚：删除 `record_host_timeout` 的调用点即可（span 不再产生，D2 回红），
   不影响 F1/F3。
3. 状态机回滚：`state.py transition --to propose`（attempt 保持），阈值不变、不新增冻结项。

## 验收失败处理

- 若 D1/D2 仍红：`attempt < 3` 时按本轮证据继续修，超限则 `escalated`；
- 若出现「全部会话降级」（D4 红）⇒ 视为作弊式实现，直接判 FAIL 并回滚 F1；
- 若 `headless.timeout` 探针证伪 ⇒ 按方案 B 调整采集口径后重跑，**不得**放宽 D2。
