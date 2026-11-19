# Agent Note: tool_followup 断流不再中断回合——流式唯一通道 + 断流可重试 + 用户文案朴素化

Status: implemented

影响文件：`agent/provider/mod.rs`、`agent/provider/fallback_body.rs`、`agent/retry.rs`、
`agent/recovery.rs`、`agent/runtime/*`、`agent/session/*`、`bin/tui/status_dwell.rs`、
`bin/tui/errors.rs`、`bin/tui.rs`。

## Problem

B247 之后 `degraded` 已改为诚实的 `interrupted`，但**中断本身仍有增无减**：11-18 晚 20:13–22:58
全库 9 轮 `turn_end=interrupted`（`pa_01a1eeae` 4 轮中 2 轮、`pa_01a1ee53`、`pa_01a1ee8f`）。
逐条证据（session.jsonl / provider_usage）：

- 每个 `tool_followup` 步 `attempts=3..6` 全部失败，错误串
  `读取 provider 返回失败：error decoding response body；type=decode；elapsed≈40–80s`
  （另有 `unexpected eof`）；
- 失败串来自**同步 buffered 通道**（`latency_kind=buffered_response`），该通道由
  `build_http_client(idle=attempt_timeout)` 构建：请求在响应体整体到达前没有任何字节，
  于是「读空闲 45s」退化为「整次请求总时限 45s」；
- 流式失败后无条件 `followup_sync_fallback` → 同步再失败 → `local_tool_followup_fallback_response`
  合成「本地兜底摘要」并以 `interrupted` 收口；用户看到停住，只能手打「继续」。

对照 dsh 源码（同一上游模型，dsh 不出现该中断）：

- `packages/llm/llm-deepseek/src/adapter.ts` + `sse.ts`：只有 SSE 流；`idleWatchdog` 默认 **300s**，
  且 **SSE 心跳注释也算活动**（`onComment: activity`），长时间生成不会被墙钟砍；
- `packages/llm/llm-deepseek/src/transport.ts`：body 读失败/协议中断一律 `TRANSPORT`，
  属默认可重试码；
- `packages/llm/llm-retry/src/index.ts` + `packages/core/agent-loop/src/agent.ts`:512：失败先落
  `assistant/attempt`，再走 `agent/request-error` waterfall，策略返回 `{kind:'retry'}` 即
  **同一步重发**；回合不会因此结束；重试事实先持久化再 sleep；
- `retry-policy.ts`：`normal`(5 次) / `always`(无界)，退避 500ms→10s 抖动，**不存在每轮 180s 聚合预算**。

（前序记录：`2026-11-18-degraded-turn-abort-diagnosis-and-fix.md`（B247）已把 `retry_mode` 两档搬过来、
把 `degraded` 改为 `interrupted`，但没动「同步回退 + 兜底合成」这条链路，故只改了终态语义，未消除中断。
本条是其续篇，不取代 B247 的结论。）

## Decision

按 dsh 口径做最小闭合改造（契约见 `.dev-team/contract-matrix-b254.md`）：

1. **流式唯一通道（C1）**：`supports_streaming=true` 的 provider，`tool_followup` 只走流式；
   删除 `continue_with_tool_result_stream_bounded` 里的 stream→sync 回退与同步重试分支。
   已发出增量后的重试禁令保持不变（避免重复正文）。
2. **断流可重试（C2）**：`error decoding response body` / `unexpected eof` / `error reading a body` /
   body 读超时归入可重试传输错误；完整 SSE 帧 JSON 解析失败（malformed）仍不可重试
   ——与 dsh 的 `TRANSPORT`（可重试）与 `MALFORMED_RESPONSE`（不可重试）同构。
3. **去掉聚合预算终态权（C3）**：流式 followup 不再注入 `aggregate_sync_call_budget_ms`(180s)；
   每次流式 attempt 沿用 15s connect + 180s 读空闲（不长于 dsh 的 300s 空闲口径）。
4. **不得合成正文（C4）**：流式路径最终失败直接上抛原始错误；runtime 现有的中断检查点保留，
   但不再把失败包装成 assistant 正文。
5. **用户文案朴素化（C5，产品要求）**：面向用户的状态行/中断提示删除「兜底/SSE/decode/provider/
   上游/解析失败/聚合预算/同步」等技术词，改朴素短语或按需省略；`[中断轮次检查点]` 前缀
   逐字保留（projection 判定依赖）。
6. **兜底正文再简化（C5 追加，用户复审）**：正文压成一句话——`` `{tool}` 已执行完成，但模型
   没有回复（超时/网络中断/回复异常/请求过多/额度用尽/登录失效/请求无效/未完成）。发送「继续」
   接着做。``；删除摘要拼接、摘要消毒/截断工具及其单测（校验的正是已删行为），
   `fallback_body_text(summary)` 形参保留兼容。

## Alternatives considered

- **只调参（把 `request_timeout_ms` 提高到 120s / 开 `retry_mode=always`）**：同步通道的
  「无字节直到整体响应」使读空闲恒等于总时限，调参只是把必死点推后；且 always 仍会回退同步。
- **TUI 加「一键续跑」按钮**：用户明确否决——续跑按钮是让人承担机器本不该制造的失败。
- **已发增量后也自动重放（dsh 的 attempt 重建）**：需要在会话/渲染层丢弃已展示增量并重建，
  属独立课题（会改变会话事实与 UI 回放语义），本波不做。
- **保留同步兜底但把 45s 提高到 300s**：等于用 buffered 通道模拟流式行为，仍无增量进度、
  仍占用 180s 聚合预算，且与流式两套口径长期漂移；直接删除该回退更小。
- **在 UI 层维护禁用词黑名单过滤文案**：会把「事实」与「措辞」的家混在一处；改在文案源头
  （provider 正文 + TUI 状态行）一次性解决。

## Known limitations

- **上游忽略 `stream:true` 的网关**：若某 provider 被标记 `supports_streaming=true` 却对
  流式请求返回非 SSE 的整包 JSON，本改动后该 followup 会直接失败（dsh 同样只接受 SSE：
  `MALFORMED_RESPONSE` 不可重试）。纠偏路径是把该模型能力改成 `supports_streaming=false`
  （走同步通道并按新分类重试），本波不引入 content-type 嗅探式自动降级。
- **`ProviderPhase` 三阶段** 在流式路径已无生产发射点（保留类型供旧回放/非流式路径与
  TUI 文案映射），后续可单独做死代码收口（backlog 跟踪，不在本波夹带）。
- TUI 终态摘要 `errors.rs` 仍含「上游服务」等措辞：限流摘要前缀是严重级分类契约
  （`starts_with` 判定），改动需同批迁移分类器与测试，留作独立波次。

## Verification

- 冻结验收测试：`crates/pony-agent-core/tests/b254_stream_only_followup.rs`（回环 mock 计数：
  流式被截断 ⇒ 可重试且**零** `"stream":false` 请求；最终 Err 且无合成正文）、
  `crates/pony-agent-core/src/bin/tui/tests/b254_plain_copy.rs`（禁用词断言）。
- 重锚：`src/bin/tui/tests/running_status_visibility_tests.rs`、
  `tests/b221_fallback_body_acceptance.rs`、`tests/b219_degraded_turn_acceptance.rs`。
- 门禁：`cargo test -p pony-agent-core`（lane）、`npm run rust:lint`、`npm run adr:check`、
  `npm run backlog:gate`、observability `audit-diff.sh`。
