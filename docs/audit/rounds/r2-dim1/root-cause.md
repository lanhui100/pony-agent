# r2-dim1 根因分析：维度 1 时延结构

轮次：`r2-dim1`｜维度：1（时延结构）｜阶段：audit｜会话：`pa_01a1ba70`
证据：18 会话（`replay/base-a2` 9 + `replay/candidate-clean` 9，各 3 任务 × 3 会话）
产物：`metrics.dim1.json`（逐调用时延）、`span-fidelity.json`（调用保真）、`latency-gaps.json`（缺口归因）

> 口径纪律：所有结论都可由「重跑命令 → 退出码/数字」复现；凡 Langfuse 聚合行为差异一律标注，
> 不把 infra 行为算作本仓缺陷。

## 一、先修测量，再谈优化（本维度的第一结论）

时延结构审计的前提是**分母可测**。当前 18 会话里存在三类测量空洞，任何「ttft 占比 / 生成占比 /
调度开销占比」在此之上都不成立：

| 空洞 | 实测 | 后果 |
| :-- | :-- | :-- |
| 根 span 缺失 | candidate 臂 2/9 会话无 `agent.turn` | 墙钟分母不存在，整轮不可审计 |
| 调用 span 丢失 | 80 次 provider 调用只落 72 个 `GENERATION` span（丢 8，10.0%） | 逐调用分解少算，缺口被推给「未归属」 |
| 首包不可归因 | 18 次调用无 `llm.call.ttft_ms`，且无任何键说明是流式还是批量 | 「排队等待」与「解码生成」不可分离 |

真值对拍方式（不靠自证）：`provider 调用数` 取自 replay 产物 `<tag>.stderr` 的
`[pony-provider] usage:<proto> raw={` 行（每次成功调用恰好一行），`span 数` 取自 Langfuse
`observations?traceId=` 的 `type=GENERATION` 条数。两个来源互不依赖。

## 二、根因 F1（blocker）：关闭预算与导出超时不匹配 → 尾批 span 丢

**证据链**

1. 常量关系（`crates/pony-agent-core/src/agent/otel/mod.rs`）：
   `SHUTDOWN_TIMEOUT_MS = 1_000` < `EXPORT_TIMEOUT_MS = 3_000`，而
   `BATCH_SCHEDULED_DELAY_MS = 5_000`。
2. 进程退出路径（`src/bin/cli.rs:1130` 的 `ObservabilityBootstrap::drop`）只调 `otel::shutdown()`，
   **没有前置 `force_flush`**。
3. 实测关联（18 会话逐一核对）：stderr 含
   `otel::shutdown：provider 关闭超时或失败，导出 runtime 保留至进程终止` 的会话 = 3 个，
   全部在 candidate 臂；其中 2 个正是 `turn_spans=0` 的会话（`s1-t01`、`s2-t01`），
   base 臂该 warn 出现 0 次、根 span 9/9 齐全。

**机制**：尾批 span 在最后 5s 内产生时尚未到批量调度周期；关闭窗口只有 1000ms，而单次导出
网络超时是 3000ms ⇒ `shutdown_with_timeout` 必然在导出完成前返回 Err，代码走
`std::mem::forget(state)` 保留 runtime，进程随即退出，尾批丢失。

**反证（已做）**：`BATCH_MAX_QUEUE_SIZE=2048` 远大于实测单轮 span 数（≤15），队列满丢 span 不成立；
采样率未开启（`GatingSampler` 只在关闭态 Drop）。故丢失不是采样/容量造成。

## 三、根因 F2（major）：provider 重试与流式回退不产 span → 未归属黑洞

**证据**：4/18 会话存在「无任何子 span 覆盖」的连续空白 > 5s，最大 42.4s。

| 会话 | 缺口 | 缺口两侧 | stderr 同期记录 |
| :-- | --: | :-- | :-- |
| base `s3-t03` | 42 385ms @14.96s→57.34s | `TOOL:List` → `llm.chat` | `followup_stream:attempt=1/5 failed ... elapsed=30786ms; parsed_bytes=10515` + `525` + `followup:stream-fallback` |
| base `s2-t01` | 10 582ms @0.0s→10.58s | turn 起点 → 首个 `llm.chat` | 无失败记录（首个请求排队） |
| cand `s1-t03` | 23 440ms @1.05s→24.49s | `TOOL:Read` → `llm.chat` | 无失败记录（首个请求排队） |
| cand `s2-t03` | 10 865ms @0.0s→10.87s | turn 起点 → 首个 `llm.chat` | 无失败记录 |

**关键相关性（18/18 全命中，可复跑）**：把「span 丢失数」与「是否存在 >5s 无覆盖缺口」逐会话对拍：

- `span_loss > 0` 的 6 个会话中，**6/6** 要么存在 >5s 缺口（4 例），要么根 span 直接缺失导致不可测（2 例）；
- `span_loss == 0` 的 12 个会话中，**12/12** 最大缺口均 < 1.4s。

即：**长尾缺口不是「调度开销」也不是「上下文装配」，而是丢失的 span 本身**。
每个大缺口恰好对应 1 个丢失的 `llm.chat` span（`loss=1`），其 `startTime→endTime` 正是那段空白。

| 会话 | span 丢失 | 最大缺口 | 缺口两侧 |
| :-- | --: | --: | :-- |
| base `s2-t01` | 1 | 10 582ms | turn 起点 → 首个 `llm.chat` |
| base `s3-t03` | 1 | 42 385ms | `TOOL:List` → `llm.chat` |
| cand `s1-t03` | 1 | 23 440ms | `TOOL:Read` → `llm.chat` |
| cand `s2-t03` | 1 | 10 865ms | turn 起点 → 首个 `llm.chat` |

**为什么这些 span 特别长、又特别容易丢**：`base s3-t03` 的 stderr 给出机制 ——
`attempt=1/5 failed ... elapsed=30786ms`（SSE 解码失败）+ `525` + `followup:stream-fallback`，
即一次 `llm.chat` 内部串了「失败流式尝试 + 回退同步请求」，单个 span 长达 42s。
span 越长，越容易在进程退出时仍在队列中未导出（见 F1），于是「最该被看见的长尾」最先消失。

**结论**：F2 不是独立的「重试无 span」缺陷，而是 F1 的放大器 ——
`llm.chat` 覆盖了重试与回退（时长正确），但正因为长，它最易被 F1 丢掉。
修复 F1 后这 4 个缺口应当自动消失，因此 **F2 不单独出修复方案，只作为 F1 的验证对象**（L4 断言即为其门禁）。

## 四、根因 F3（major）：`llm.request.mode` 无法区分流式与缓冲传输

**证据（18/18 会话完全相等）**：每会话 `ttft_missing_calls` 恒等于该会话的 buffered HTTP 调用数
（`request:openai followup-sync body_preview` + `followup-sync-batch`），无一例外。

| 会话 | span 数 | 无 ttft | buffered HTTP |
| :-- | --: | --: | --: |
| base `s1-t01` | 6 | 1 | 1 |
| base `s2-t02` | 6 | 3 | 3 |
| cand `s2-t01` | 4 | 2 | 2 |
| …（其余 15 会话同样相等） | | | |

**缺陷**：`LlmRequestMode` 只有 `initial / tool_followup / tool_recovery` 三值
（`turn_flow.rs:2915`），**stream 与 buffered 都写 `tool_followup`**。于是「批量路径本就没有首包」
（`record_call_timing(Some(elapsed), None)`，`turn_flow.rs:3199/3271/3364`）与
「流式路径测量丢包」在下游完全同形 —— 这正是首包口径无法做门禁的原因。

## 五、根因 F4（minor）：`agent.turn.calls` 语义漂移

文档（`docs/observability.yaml` wave-54 段）写「已完成调用数」，代码实际写的是
**参与速度计算的合格调用数**（`resolve_turn_speed` 返回的 `truth.calls`，
要求 `dur > 0 && out > 0`，且优先取 `decode_calls`）。

实测：base `s1-t02` 有 5 次 provider 调用、5 个 `llm.chat` span，但 `agent.turn.calls=3`；
base `s2-t02` 6 次调用写 3。按文档理解会得出「3 次调用产生 6 个 span」的错误结论。

## 六、不属本维度（记录，转后续维度）

- **价格轨全空**：`pony.cost.priced=false`、trace `totalCost=0` → 维度 5。
- **重试策略本身**：`NonRetryable` 分类把 SSE 解码失败判为不可重试并立即回退同步请求，
  是否应重试属 provider 策略问题，本维只要求「耗时可见」→ 维度 3（失败语义）。
- **Langfuse trace 级 usage 恒空**：infra 侧聚合行为，非本仓缺陷 → 维度 6 已记录。

## 七、修复顺序与依赖

F1 是其他一切的前置：**先让 span 不丢**，否则 F2/F3 的验证同样拿不到完整证据。
F2 与 F3 无相互依赖，可同批实施。F4 只改文档 + 断言口径，不动代码。
