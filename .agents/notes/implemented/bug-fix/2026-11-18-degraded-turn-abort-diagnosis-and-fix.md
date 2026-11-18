# Agent Note: degraded 轮（上游失败被兜底吞掉）的根因与七项修复

Status: implemented

影响文件：`agent/provider/mod.rs`、`agent/provider/fallback_body.rs`、`agent/config.rs`、
`agent/retry.rs`、`agent/turn_event.rs`、`agent/turn_flow.rs`、`agent/runtime/turn_control.rs`、
`src/stores/runtime.ts`、`src/lib/runtime/turn-retry.ts`、`src/components/chat/AgentRuntimeStatusBar.vue`、
`src/components/HomeWorkspace.vue`。

## Problem

现场（`pa_01a1e908` / `pa_01a1e928` 两条会话，同日全库 72 条同类）表现为「轮次毫无征兆地停在中途」：

- 每轮最后一个 `tool_followup` 请求连不上上游（`tokens.ponyjob.top`；`type=request` CDN 断连、
  `operation timed out`），随后 runtime 把失败合成为一段**本地兜底文案**；
- `provider_failure_message` 只认 `provider_mode == "mock"`，`"fallback"` 被判为**成功完成** ⇒
  轮次以 `completed + degraded` 收口，UI 上无法与正常结束区分；
- 同步链路用 `Client::builder().timeout(45s)`（含读完响应体的**总时限**），与流式路径
  （15s 建连 + 180s 读空闲）两套口径；每次 attempt 都吃满时限 ⇒ 把 180s 聚合预算吃光，
  末次 attempt 只剩 6.4s（必超时）甚至 0ms（`no request sent`，一次 HTTP 都没发）；
- 取消轮把 `error` 一律写死 `stopped_by_user`，实测那个 108s、0 次 provider 调用的
  `cancelled` 轮**无法归因**（人工停止？看门狗？）；
- 重试事实只有 `llm/retry`（退避前），没有「退避结束、attempt 真的发出」的落点。

## Decision

按最小闭合顺序落地七项：

1. **同步客户端超时口径对齐流式**：`build_http_client` 改 `connect_timeout(min(idle,15s))`
   + `read_timeout(idle)`，去掉 45s 总时限；墙钟护栏仍由跨 attempt 聚合 deadline 承担。
2. **预算算术修正**：新增 `UPSTREAM_MIN_USABLE_ATTEMPT_TIMEOUT_MS = 5s`；
   `effective_attempt_timeout_ms` 在「剩余 < `min(5s, request_timeout_ms)`」时返回 0
   （不发 HTTP，立即用最后一次真实错误收口），杜绝实测的 `remaining=0ms; no request sent`
   与「秒级必超时 attempt」。次数**不**按预算钳制（`sync_affordable_attempts` 只做观测出口）：
   默认 10 次是「快失败」（429/5xx/连接重置，数百毫秒级）的耐心预算，钳制会把 10 次压成 4 次，
   实测会打破 `retry_loop_reports_real_progress_to_observer` 等契约；慢路径仍由 180s 墙钟护栏收口。
   `upstream_policy_report` 增 `affordable_attempts`、`retry_mode`。
3. **兜底不再伪装成「正常完成」**：`completed_turn_end_reason` 由 `Degraded` 改为
   `Interrupted`（可重试/可续跑），`Degraded` 变体保留仅供旧日志读取（wire 兼容读不变）。
   刻意**不**把 `provider_mode="fallback"` 改成 `turn:failed`：实测那样会让本轮已完成的
   hook / capability / 工具事实在失败收口处提前 return（打破 7 个运行时用例），
   代价大于收益；终态 reason + UI 可重试提示足以让「说到一半停了」不再冒充正常结束。
4. **步骤级有界重试 + durable-before-wait**：新增 `TurnEvent::LlmRetryStarted`（`llm/retry_started`），
   在退避结束后、attempt 发出前落盘，补齐「重试是否真的发出」这一事实；新增
   `RetryMode { Normal, Always }`（dsh 两档），`Always` 只放宽次数/预算闸门，不放宽流安全与
   不可重试错误；`UpstreamPolicy.retry_mode` 容错反序列化（畸形值回落 `normal` 并 warn）。
5. **兜底归因换语义码**：`fallback_error_category` 收归 `retry::classify_error_class`，
   `type=request`（CDN 断连）不再笼统归「上游返回异常」；仅保留我们自产的解析失败前缀例外。
6. **turn_end 增 `stop_reason` + `failure_code`**：两者均为 additive（旧日志缺键 ⇒ `None`）；
   取消轮 `stop_reason` 取该轮 checkpoint 的真实值（`user`/`parent`/`hook`/`watchdog`/`legacy`），
   未记录仍退化为 `stopped_by_user`（既有哨兵句不变）；失败轮 `failure_code` 由
   `turn_flow::failure_code_of`（唯一出口 `classify_error_class`）产出。
7. **UI 显式可重试 + 一键续跑**：新增纯函数 `lib/runtime/turn-retry.ts`（可重试码集合 +
   标签/说明）；store 增 `failedTurnRetry` getter 与 `retryLastTurn()` action（等价输入「继续」）；
   终态状态行追加「上游失败，可重试」并按同一判定渲染「重试」按钮。

## Alternatives considered

- **只治标（把 `request_timeout_ms` 调到 120s 并启用 failover）**：能缓解当天现场，但
  「兜底伪装成功」与「0ms attempt」仍是活的缺陷；且 failover 对 followup 入口本就不参与换手。
- **让 `Always` 模式默认开启**：会牺牲「快速失败」契约（配错 base_url 时用户要等到天荒地老），
  故默认 `Normal`，`Always` 由用户在 `upstream_policy.retry_mode` 显式选择。
- **给 TurnEnd 加嵌套 `details` 结构**：需要改 51 处字面量且破坏既有 wire 读端；
  改为**平铺的 additive 可选字段**，旧日志/旧读端零改动。
- **把 `llm_retry_started` 做成观察者回调而非事件**：观察者是进程内 thread_local，
  重启即丢——而本次复盘卡住的正是「进程重启后无法判定末次 attempt 是否发出」。
- **在 UI 侧自建关键词表判可重试**：会与后端码表漂移；改为同一语义码集合的单点实现
  （后端 `classify_error_class` / 前端 `turn-retry.ts`），并在两端各配单测锁定。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core --all-targets`（exit 0）。
- `npx vue-tsc --noEmit`（exit 0）。
- `npx vitest run tests/turn-retry.test.ts`（6 passed）：覆盖实测两类死因文本
  （`operation timed out` / `type=request`）与不可重试类（配额/鉴权）及畸形输入红线。
- `cargo test -p pony-agent-core --test b219_degraded_turn_acceptance`：派生口径断言已随
  P0-3 改为 `Interrupted`（wire 兼容读仍读回 `Degraded`）。
- 兜底归因单测（`provider/fallback_body.rs`）新增 `type=request` ⇒「连接上游失败」与
  429 ⇒「上游限流」两条真值表断言。
