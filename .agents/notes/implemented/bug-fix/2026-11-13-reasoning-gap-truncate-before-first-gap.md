# Agent Note: reasoning-gap 装载期降级从整段清空改为截断到首个缺口前

Status: implemented

## Problem

事故 `pa_01a1ce16` 的 full_replay 归因候选 (b)：`agent/session/store.rs` 的
`sanitize_provider_native_transcript` 在 `first_legacy_reasoning_gap` 为真时直接
`provider_native_transcript.clear()`（装载路径 `store.rs` 的 `snapshot_at` 自愈链路）。

后果不对称：历史里只要出现一条「缺 `reasoning_content` 的 assistant（或 tool 之后缺 reasoning）」，
整段原生转录被清空，续跑时模型看不到任何历史工具往返与结论，只能从 system + 最后一轮重新开始；
而缺口之外的消息本身完全合法，被一并丢弃属于过度降级。

既有基线 `session/tests/native_context.rs` 断言「真 reasoning gap 必须清空降级」，把过度降级固化成契约，
B114 因此只能靠「中断/失败轮闭合片段末条 assistant 必带 reasoning_content」缓解。
判据本身由 `.agents/notes/implemented/architecture/2026-11-11-multi-turn-tool-context-pruning-dsh.md`
确立（只有真 reasoning 缺口才降级）；本条收窄的是**降级动作**，不是判据。

## Decision

1. **降级粒度**：`sanitize_provider_native_transcript` 改为截断——保留首个缺口消息之前的完整前缀，
   丢弃缺口消息与其后全部消息（`provider_native_transcript.truncate(gap_index)`）。缺口消息本身不可用
   （缺 reasoning 的 assistant 会让上游 400），其后消息建立在它之上、无法安全续接；缺口之前的前缀与
   缺口无关，是可直接复用的合法上下文。
2. **尾部再自愈**：截断后重跑一次 `tool_pruner::repair_and_filter_tool_roundtrips`，保证「缺口紧跟
   tool 结果」时截断出的尾部仍是合法往返，不残留孤儿 tool 结果。
3. **判据单一真源**：判定从 `-> bool` 改为 `first_legacy_reasoning_gap(transcript) -> Option<usize>`
   返回首个缺口下标；删除只做 `.is_some()` 转发的 bool wrapper（消费者仅 sanitize 一处，全仓代码注释
   引用同步改名），遍历与豁免逻辑一行不改。
4. **基线迁移 + 补例**：`test_sanitize_transcript_clears_real_legacy_reasoning_gap` 迁移为
   `..._truncates_real_legacy_reasoning_gap`（断言保留 3 条前缀、缺口 assistant 与其 tool 结果一并消失）；
   新增 `test_sanitize_transcript_truncates_gap_after_tool_and_keeps_closed_roundtrip`（缺口落在
   「tool 之后的第一条 assistant」时，截断后末尾仍是已闭合往返）。
5. **模块归属（同波体积门禁收口）**：判据与降级实现从 `session/store.rs` 外移到
   `session/transcript_sanitize.rs`（纯搬迁，store 保留同名 `pub(crate) use` re-export，
   既有消费者路径不变）；对应测试从 `session/tests.rs` 迁入 `session/tests/native_context.rs`，
   使两个文件回到体积基线内（`size:check` 0 新违规）。

## Alternatives considered

- **A. 只丢缺 reasoning 的 assistant 文本、保留 tool 往返**（账本原收口建议）：落选。丢掉 assistant 正文后
  仍留下「只有 tool 往返、前面缺 reasoning 的孤岛」，reasoning 模型在 tool 之后缺思维链的事实没有消失，
  上游 400 风险未消；正文与 `tool_calls` 分离还会让转录语义更破碎。
- **B. 给缺口消息补 `reasoning_content` 事实注记**（与中断轮注记同构）：落选。中断轮注记成立的前提是
  「本轮确实没有产出推理」，属于可观测事实；历史缺口里的推理内容无人知晓，补注记等于为模型上下文伪造
  思维链，污染真值并诱导后续轮次把它当成真实推理。
- **C. 截断到首个缺口前**（采纳）：前缀合法、零伪造、丢弃范围最小。
- **现状 D. 整段 `clear()`**：落选。缺口外的合法上下文被一并丢弃，续跑上下文全丢（本事故归因）。

## Consequences

- 截断结果经 `snapshot_at` 的 `should_save` 落盘，即「丢弃尾部」不可逆；用户看不到被丢内容，
  语义可由压缩摘要/历史轮次补足。这是刻意取舍：宁可上下文短，也不给上游发送会被 400 拒的转录。
- 缺口越靠前保留越少；缺口就是首条 assistant 时等价于旧行为的清空（无可保留前缀）。
- 判据行为未变（同一遍历、同一中断注记豁免），仅返回形态改变；跨文件改动只有注释引用改名。
