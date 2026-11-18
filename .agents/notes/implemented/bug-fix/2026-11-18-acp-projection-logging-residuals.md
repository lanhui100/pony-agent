# Agent Note: ACP 委派投影/日志残余收口（B240）

Status: implemented

工单：B240。影响文件：`agent/projection.rs`、`agent/turn_event.rs`、`agent/turn_flow.rs`、
`agent/sqlite_session.rs`、`agent/otel/log.rs`、`acp/{protocol,delegate_events,result,session}.rs`、
`tests/b240_refold_retry_attribution.rs`（新）。

## Problem

wave-acp-delegate 的 L3-B / Gate-1 ADVISORY 残余：
1) `projection.rs` 从事件重建时恒把 `TurnToolActivity.parent_activity_id=None`、
   `ProviderCallCacheRecord.attempts=None`；`startup_repair.rs` 会把 refold 结果写回 trace 缓存
   ⇒ 重启修复后子活动归因与 C5.1 重试三键静默丢失；
2) `otel::log` 只对 message 脱敏，extras（params/error）明文入 stdout/落盘；
3) `acp/protocol.rs` 的 `ToolCallStatus` 缺 `#[serde(other)]`，未知状态整帧丢弃；
4) 子活动 id 直接使用线缆 `toolCallId`，未命名空间化；
5) `finalText` 上限是 32_000 **字符**，而下游工具结果落库上限是 32 KiB **字节**：32 000 个 CJK
   字符 ≈ 96 KB 会在下游被静默再切一刀，而 `finalTextTruncated` 仍为 false（谎报完整）。

## Decision

- 投影可重建：`ToolCall/ToolResult` 的 `parent_activity_id` 映射进 `TurnToolActivity`（activities /
  timeline / unmatched 合成活动三处）；`ProviderUsage` 事件新增 **additive** 三键
  `attempts / failed_attempt_ms / prelude_ms`（`serde(default)`，旧日志缺键 = 未知而非 0/1），
  写入侧透传 per-call 记录（回退单条仍 None，不伪造），旧数据回填同键，折叠侧映射回缓存记录。
- `otel::log` extras 与 message 走**同一条** `ponysentry::sanitize_json` 流水（数字/布尔保类型、
  字符串/嵌套结构递归脱敏）。
- `ToolCallStatus` 增 `#[serde(other)] Unknown`：未知状态不再整帧丢弃；映射侧按「进行中」处理
  （猜成 done/failed 会伪造未发生的结果），session 侧文案为 `unknown`。
- `finalText` 上限统一为 **UTF-8 字节**口径 `ACP_DELEGATE_FINAL_TEXT_MAX_BYTES = 32_000`
  （< 下游 32 KiB）；截断切口回退到最近字界，保证合法 UTF-8 且 `finalTextTruncated` 与实际保留
  内容一致（ASCII 下字节 == 字符，既有 32 000 断言不变）。
- **暂缓（明确理由）**：子活动 id 改 `{call_id}-child-N`。ACP 子活动 id 是外部 agent 的
  `tool_call_id`（跨 `tool_call_update` 复用、且被事件匹配键使用），改名需在 ACP 会话内建
  「外部 id → 命名空间 id」稳定映射并同时改写 Call/Result 双端；更硬的阻碍是前端契约：
  `src/lib/runtime/trace.ts` 与 `trace_timeline.rs` 都以 `id.contains("-child-")` 过滤顶层工具，
  改名会让 ACP 子活动从顶层列表消失且无嵌套渲染兜底 ⇒ 属跨前端写域契约变更，另立工单。
  既有 `workspace_batch` 复合子活动已是 `{call_id}-child-N`，口径本身已存在。

## Alternatives considered

- **A. 重试三键不事件化，只在 projection 里按「有 retry 事件」推断 attempts。** 落选：事件流没有
  每次 attempt 的结算键，推断会伪造计数；additive 三键才是可复原的真值。
- **B. finalText 改为 32 KiB 精确上限（32_768）。** 落选：下游工具结果上限同为 32 KiB，
  边界相等时序列化/拼接的额外字节仍可能触发下游截断；32_000 留出确定性余量。
- **C. 子活动 id 立即改命名空间化。** 落选：见上一节前端契约阻碍；先补可重建字段，id 命名空间化
  作为独立跨前端工单。

## Verification

- `--test acp_delegate_p0p2_contract` 15 passed、`--test acp_delegate_adversarial` 6 passed、
  `--lib projection` 37 passed、`--lib otel` 55 passed、`--test b240_refold_retry_attribution` 3 passed
  （事件 schema 带货 → 重启读取 → fold_all 保留归因/三键；旧日志 additive 折叠不伪造）；
- `check --all-targets` exit 0；`npm run rust:lint` PASS；`size:check` 0 new。
