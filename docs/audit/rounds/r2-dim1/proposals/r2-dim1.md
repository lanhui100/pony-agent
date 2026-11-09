---
status: approved
approved_by: user
approved_at_unix: 1794144930
approved_scope: F1 / F3 / F4（写域按提案声明；F4 仅文档措辞，不改代码）
dimension: 1
dimension_name: latency.structure（时延结构）
round: r2-dim1
author_session: pa_01a1ba70-a314-7712-adf2-4bf675a96f99
baseline_commit: b4e48b4157a04322a7ad0a9caa779387a2a75cb7
corpus_hash: 879597eb3594eb365bdbe033f7fbcd8353380e359170267f8d3e621255b8300a
evidence: 18 sessions（replay/base-a2 9 + replay/candidate-clean 9，3 任务 × 3 会话），receipts 001–004
---

# r2-dim1 提案：时延结构（先修测量，再谈优化）

## 前置事实（实施前必须确认）

**r1-dim6 的三处修复目前只存在于工作区与游离提交 `6b77f939`，主线 HEAD（`b2d603d0`）不含 F2/F3。**
机械证据：`git show HEAD:crates/pony-agent-core/src/agent/otel/mod.rs | grep -c r1-dim6` = 0，
工作区 = 8；`git merge-base --is-ancestor 6b77f939 HEAD` = 否。

⇒ 本轮的 `impl_base_commit` 取**实施时的工作区状态**（含 r1 修复）。若先回退 r1 修复再实施本提案，
dim6 棘轮必然回归（A2/A3 直接红）。实施阶段第一步是确认工作区仍含 r1 修复，否则先落 r1 提交。

## 阈值与断言（批准即冻结）

| 断言 | 口径 | 基线实测 | 通过条件 |
| :-- | :-- | :-- | :-- |
| **L1** 根 span 完整性 | 每会话恰 1 个 `agent.turn` SPAN | 2/9 缺失（candidate 臂） | 缺失数 = 0 |
| **L2** 调用保真 | `GENERATION` span 数 == provider 成功调用数（stderr `usage:* raw={` 行数） | 80 调用 / 72 span，丢 8（10.0%） | 丢失数 = 0 |
| **L3** 首包可归因 | 无 `llm.call.ttft_ms` 的调用必须带 `llm.request.transport` | 18/18 无首包调用均无该键 | 不可归因数 = 0 |
| **L4** 长尾可归因 | 单会话未被任何子 span 覆盖的最大连续缺口 ≤ 5000ms | 4/18 会话超阈（最大 42 385ms） | 超阈会话数 = 0 |

断言命令（唯一裁决者，退出码 0=pass / 1=fail / 2=证据缺失 / 3=环境不可用）：

```bash
bash scripts/audit/assertions/dim1.sh \
  --arm-dir  docs/audit/rounds/r2-dim1/replay/<arm> \
  --arm-tag  <base|candidate> \
  --metrics  docs/audit/rounds/r2-dim1/metrics.dim1.json \
  --out-dir  docs/audit/rounds/r2-dim1/assert-<arm>
```

已实测红相：基线臂 `failed=3`（L2/L3/L4）、候选臂 `failed=3`；缺 metrics → `exit=2`；
臂目录/sessions.tsv 缺失 → `exit=2`（`scripts/audit/selftest.sh` 已锁这三条，29/29 通过）。

## 修复方案

### F1（blocker）退出路径缺前置 flush → 尾批 span 丢

**根因（已定位到常量与调用点）**

| 项 | 值 | 位置 |
| :-- | :-- | :-- |
| 关闭等待上限 | `SHUTDOWN_TIMEOUT_MS = 1_000` | `otel/mod.rs:3057` |
| 单次导出网络超时 | `EXPORT_TIMEOUT_MS = 3_000` | `otel/mod.rs:3049` |
| 批量调度周期 | `BATCH_SCHEDULED_DELAY_MS = 5_000` | `otel/mod.rs:3065` |
| CLI 退出路径 | 仅 `otel::shutdown()`，无 `force_flush` | `cli.rs:1130` |
| TUI 退出路径 | 同上（5 处） | `tui.rs:7067/10852/10975/10991/11006` |
| 桌面退出路径（对照） | **先 `force_flush(1000)` 再 `shutdown()`** | `src-tauri/src/lib.rs:1380` |

机制：尾批 span 尚未到 5s 调度周期；关闭窗口 1s < 导出超时 3s ⇒ 关闭必然在导出完成前超时，
代码走 `std::mem::forget(state)`（`otel/mod.rs:1007`）后进程退出，尾批丢失。

实测关联：stderr 含「provider 关闭超时或失败」的 3 个会话全部在 candidate 臂，其中 2 个正是
`turn_spans=0`；base 臂该 warn 0 次、根 span 9/9 齐全。

**方案**：CLI/TUI 的 `ObservabilityBootstrap::drop` 在 `shutdown()` 前增加
`otel::force_flush(EXIT_FLUSH_BUDGET_MS)`（1000ms），与 `src-tauri/src/lib.rs:1373-1390` **同构**；
失败只降级 warn，不阻断退出。总预算 1000 + 1000 = 2000ms ≤ NFR `exporter_flush_timeout_ms=3000`。

- 写域：`crates/pony-agent-core/src/bin/cli.rs`、`crates/pony-agent-core/src/bin/tui.rs`
- 回滚：删除两处 `force_flush` 调用，行为回到现状
- 不新增契约：`force_flush` 与 `EXIT_FLUSH_BUDGET_MS` 语义已在桌面端固化

### F3（major）`llm.request.mode` 不区分流式/缓冲传输 → 无首包不可归因

**根因**：`LlmRequestMode` 只有 `initial / tool_followup / tool_recovery`
（`turn_flow.rs:2915-2919`），stream 与 buffered 都写 `tool_followup`；
而 buffered 路径本就无首包（`record_call_timing(Some(elapsed), None)`，
`turn_flow.rs:3199 / 3271 / 3364`）⇒ 与「流式测量丢包」同形。

实测：18/18 会话的 `ttft_missing_calls` **恒等于** buffered HTTP 调用数
（`request:openai followup-sync body_preview` + `followup-sync-batch`）。

**方案**：新增属性 `llm.request.transport`，值域 `stream | buffered`，由 6 个 llm span 调用点
各自传入（与 `record_request_shape` 同一出口，三键同生同灭的既有纪律不变）：

- `otel/mod.rs`：新增 `ATTR_LLM_REQUEST_TRANSPORT` 常量；`llm_request_shape_attributes` 增一参；
  `LlmSpanGuard::record_request_shape` 增 `transport` 参数
- `turn_flow.rs`：6 处 `start_llm_span_with_provider` 调用点按实际传输方式传值
  （`provider_decision_stream*` / `provider_followup_stream*` = `stream`；
  `provider_decision` / `provider_followup`（sync、sync-batch） = `buffered`）
- `docs/observability.yaml`：登记新键与值域
- 测试：`crates/pony-agent-core/tests/otel_request_transport_test.rs`（新增，断言两种路径各自的键值）

- 回滚：删除该键（L3 随之退化为不可判定，不会产生假绿）
- 风险：`llm_request_shape_attributes` 是公开函数，签名变更影响 1 处生产调用 + 单测；
  不做兼容重载，避免两套口径并存

### F4（minor）`agent.turn.calls` 文档语义与实现不符

实现写的是「参与速度计算的合格调用数」（`turn_flow.rs:2672-2702` 的 `truth.calls`，
要求 `dur > 0 && out > 0`，优先 `decode_calls`）；文档（`observability.yaml` wave-54 段）
写「已完成调用数」。实测 base `s1-t02`：5 次调用 / 5 个 span，`agent.turn.calls=3`。

**方案**：只改文档措辞为「参与速度计算的合格调用数（`dur>0 且 out>0`；优先 stream_decode 集合）」，
并在 L2 断言旁注明「真实调用数以 L2 的 provider 计数为准」。不改代码、不改值域。

- 写域：`docs/observability.yaml`
- 回滚：恢复原措辞

## 不属本维度（记录，转后续维度）

- **F2 不是独立缺陷**：18/18 对拍显示「span 丢失」与「>5s 缺口」完全同集（6/6 与 12/12），
  每个大缺口恰好对应 1 个丢失的 `llm.chat` span；F1 修好后 L4 应自动转绿。故不出 F2 独立方案。
- **重试策略**：`NonRetryable` 把 SSE 解码失败判为不可重试并立即回退同步请求（单 span 达 42s），
  是否应重试属失败语义 → 维度 3。
- **价格轨全空**：`pony.cost.priced=false`、trace `totalCost=0` → 维度 5。
- **arm 产物缺 `arm.json`**：r1 的 6 个臂目录无 `arm.json`（provenance 未机器固化），
  本轮沿用其会话数据但需在 verify 阶段用 `scripts/audit/replay.sh` 重放带 provenance 的新臂。

## 写域冻结与防作弊

- 批准时冻结 `corpus_hash` 与本提案四条断言口径；事后放宽须新提案重审。
- 实施只允许触碰：`cli.rs`、`tui.rs`、`otel/mod.rs`、`turn_flow.rs`、
  `docs/observability.yaml`、新增 `tests/otel_request_transport_test.rs`。
- 验证：scratch worktree 双臂（baseline = 实施前工作区，candidate = 实施后）各 ≥3 会话重放，
  跑 `assertions/dim1.sh` 退出码 + `ratchet.sh` 回归 dim6。
