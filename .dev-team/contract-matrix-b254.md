# B254 契约矩阵（Lead 冻结，2026-11-19）

任务：消除 TUI tool_followup 的「中途中断」，借鉴 dsh 的流式/重试语义。

## 根因（现场证据）

- 现场会话 `pa_01a1eeae`（4 轮 2 中断）、`pa_01a1ee53`、`pa_01a1ee8f`：每个 `tool_followup` 步
  `attempts=3..6` 全失败，错误 `读取 provider 返回失败：error decoding response body；type=decode；
  elapsed≈40–80s`，随后 `turn_end.reason=interrupted`。
- 代码路径：`provider/mod.rs::continue_with_tool_result_stream_bounded` 流式失败 →
  `followup_sync_fallback`（同步客户端 `build_http_client` 的 read idle = attempt 45s，无字节直到
  整体响应 ⇒ 等价总时限）→ 同步再失败 → `local_tool_followup_fallback_response` 合成「本地兜底摘要」。
- 对照 dsh：只有 SSE 流（`idleWatchdog` 默认 300s，心跳注释计活动）；无 180s 每轮预算；
  失败走 `agent/request-error` → 同一步重试（`continue`），回合不结束。

## 冻结契约

| 编号 | 契约 | 判定面（测试） |
|---|---|---|
| C1 | `supports_streaming=true` 的 provider，`tool_followup` **只走流式**；删除 stream→sync 回退与同步重试分支。流式按既有 policy 重试；**已发出增量后不得重试**（保持不重复文本）。 | `tests/b254_stream_only_followup.rs`：mock 对端收到的 `"stream":false` 请求数 = 0 |
| C2 | body 断流/读失败（`error decoding response body`、`unexpected eof`、`error reading a body`、body 读超时）⇒ 可重试（UpstreamNetwork 语义）；完整 SSE 帧 JSON 解析失败（malformed）保持**不可重试**。 | 同上：截断 body 场景同 URL 请求数 ≥2；`retry.rs` 行内断言同步 |
| C3 | 流式 followup **不注入** `aggregate_sync_call_budget_ms`（180s）deadline；不得因聚合预算耗尽产生终态。每次流式 attempt 用 15s connect + 180s 读空闲。 | C1 的无同步请求即反证；代码审阅 `with_aggregate_sync_deadline` 调用点 |
| C4 | 流式路径最终失败**不得**返回 `local_tool_followup_fallback_response` 合成正文；原始错误上抛（runtime 现有中断检查点逻辑保留）。`ProviderPhase::{StreamFallback,SyncAttempt,LocalFallback}` 不再由流式路径发出。 | `tests/b254_stream_only_followup.rs`：最终为 Err，且不存在含「兜底」的 Ok 正文 |
| C5 | 面向用户的状态行/中断提示**不得**含技术词：兜底、SSE、decode、provider、上游、解析失败、聚合预算、同步。用朴素短语或按需省略。`[中断轮次检查点]` 前缀逐字保留（projection 依赖）。 | `src/bin/tui/tests/b254_plain_copy.rs` + 重锚 `running_status_visibility_tests.rs` |

## 非目标（Lead 裁决，用户确认）

- 不做「一键续跑」按钮/自动续跑；中断后仍由用户决定是否继续。
- 不做已发增量后的自动重放（会重复正文）。
- 不改换手（failover）语义、不改 Anthropic 臂与非流式 provider 的同步通道。

## 写域

- exec-core：`agent/provider/`、`agent/retry.rs`、`agent/recovery.rs`、`agent/runtime/`、`agent/session/`
- exec-tui：`bin/tui/status_dwell.rs`、`bin/tui/errors.rs`、`bin/tui.rs`
- test-agent：`tests/b254_*`、`src/bin/tui/tests/b254_*`、重锚 3 个既有测试文件
- protocol：`.dev-team/nfr-baseline.json`


## C5 冻结文案（用户可见，逐字）

- `ProviderPhase::StreamFallback` → `连接不稳定，重试中…`
- `ProviderPhase::SyncAttempt { attempt }` → `连接不稳定，重试中（第 {attempt} 次）…`
- `ProviderPhase::LocalFallback` → `本轮未能完成，请稍后重试。`
- 中断状态行保持 `上一轮已中断，已保留已执行的步骤记录。` / `上一轮因错误终止，已保留已执行的步骤记录。`（已朴素，不改）
