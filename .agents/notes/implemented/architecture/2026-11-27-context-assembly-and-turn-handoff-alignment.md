# Agent Note: 上下文构建、跨轮 KV Cache 与轮次交接全链路对齐

Status: implemented

## Problem

在对照 `deepseek-harness` 对本仓上下文构建与轮次交接链路做全量审计后，发现虽然单程投影（[2026-11-13-context-assembly-single-pass-projection](../implemented/architecture/2026-11-13-context-assembly-single-pass-projection.md)）、原生压缩切分与溢出自救（[2026-11-24-native-compaction-split-and-overflow-rescue](../implemented/bug-fix/2026-11-24-native-compaction-split-and-overflow-rescue.md)）和原生工具回放（[2026-11-11-multi-turn-tool-context-pruning-dsh](../implemented/architecture/2026-11-11-multi-turn-tool-context-pruning-dsh.md)）已打下基础，仍存在八项核心错漏：

1. 压缩（`compaction_ops.rs`）与溢出兜底裁剪（`overflow_rescue.rs`）只将 `<session_summary>` 写入文本 `history`，`provider_native_transcript` 未注入摘要，导致 `native_tool_flow` 模型在压缩后丢失早期对话记忆。
2. 易变上下文（`VolatileNotes`）夹在历史与当前 `User_N` 之间且不落盘，次轮 `User_N` 与整轮工具往返前缀全部移位失效；且 `native_volatile` 以 `"role": "system"` 序列化，违反 OpenAI/DeepSeek/Anthropic 协议约束并污染 Anthropic `system` 缓存块。
3. 并行工具批落盘（`tool_recovery.rs`）按 `ToolTurnHopRecord` 逐跳生成独立 `assistant` 消息并克隆 `reasoning_content`，放大体积且在 Anthropic 协议下破坏 `1 assistant(N tool_use) -> 1 user(N tool_result)` 配对。
4. Ask 挂起不落盘此前已闭环 `hop_records`，恢复时若 `native_messages` 为空会丢弃 `request.input`、插在 `<system-reminder>` 之后且完结不进转录；Goal 轮 `history` 存 `display_message` 与转录存展开提示词不等，导致 `align_native_transcript_with_history` 尾部失配。
5. 单轮多跳 `ToolFollowup` 阶段缺少历史 hop 的 Soft-Trim 预剪枝；同时摘要请求零 KV-Cache 复用、缺少 Pruner 前置与体积收敛校验、`ContextPressure::layered_tokens` 重复叠加 `history + transcript`、CJK token 低估以及切小模型前未用原模型预压。
6. `workspace_hotloader` 会话基线与首轮请求头仅驻留内存，重启或 LRU 淘汰后前缀漂移。

## Decision

1. **原生压缩与自救双注入 `<session_summary>`**（`crates/pony-provider/src/compression.rs`、`crates/pony-agent-core/src/agent/runtime/{compaction_ops,overflow_rescue}.rs`）：
   - 新增 `compact_native_transcript_with_summary` 与 `is_session_summary_message`，压缩或 `transcript_trim` 兜底（覆盖多轮前缀丢弃及二次压缩保留）同步将 `<session_summary>` 作为首条 `user` 消息写入 `provider_native_transcript`。
2. **`Volatile` 改制为尾部 `user` 级 `<system-reminder>`**（`turn_prompt_content.rs`、`context.rs`、`crates/pony-provider/src/provider/mod.rs`）：
   - 易变提示统一以 `<system-reminder>...</system-reminder>` 包装，在 `input` 与 `native_messages` 中置于当前 `User_N` **之后**（`native_messages` 直接为 `role = "user"`；OpenAI/Anthropic 发送器对带该标记的 `Developer` 行同步投影为对话尾部 `user` 消息并排除出 Anthropic 顶层 `system` 缓存块），同时保证 `request.images` 仍绑定在真实 `User_N` 上。
3. **并行工具批聚合落盘**（`crates/pony-provider/src/provider/mod.rs`、`crates/pony-agent-core/src/agent/runtime/tool_recovery.rs`）：
   - 新增复数协议构造器 `provider_native_assistant_tool_calls_message_for_protocol` 与 `provider_native_tool_results_messages_for_protocol`；`tool_recovery.rs` 按同一 `assistant_message` 批次聚合连续 `ToolTurnHopRecord`，为 OpenAI 生成 `1 assistant + N tool`、为 Anthropic 生成 `1 assistant + 1 user([N tool_result])`。
4. **Ask 挂起/恢复与 Goal 对齐修复**（`turn_prep.rs`、`turn_flow.rs`、`turn_sync.rs`、`turn_stream.rs`、`context_window.rs`）：
   - Ask 挂起时持久化挂起前已闭环的工具 hops；恢复时若 `native_messages` 为空则先从 `request.input` 物化完整上下文（避免丢失系统/历史），并定位在真实 `User_N` 之前插入问答对，完结时前置于落盘原生转录；`align_native_transcript_with_history` 在 user 文案锚定为空时按末条非检查点 `assistant` 正文回退锚定，兼容 Goal `display_message`。
5. **高水位轮内 Soft-Trim、Prefix-Fork 压缩与单源压力计价**（`crates/pony-protocol/src/{estimate,tool_pruner}.rs`、`compression.rs`、`compaction_ops.rs`、`turn_prep.rs`、`turn_sync.rs`、`turn_stream.rs`）：
   - `prune_native_messages_in_place` 扩展支持 Anthropic `tool_result` 块，并新增 `prune_older_tool_results_in_place`；`turn_sync`/`turn_stream` 仅在 `accumulated_messages` 超过模型压缩高水位时修剪更早 hop（阈值内零篡改保缓存；客户端严守既定 128 MiB 网关量级闸门、禁止任何 512 KiB 类小体积拦截）。
   - `estimate_text_tokens` 校准非 ASCII/中文权重（`~1.5 token/字`）；`ContextPressure::layered_tokens` 改为 `history_tokens.max(transcript_tokens)` 消除 2x 重复计价；`build_compaction_request` 先跑 `ToolResultPruner` 再复用稳定系统前缀与真实原生转录作 Prefix-Fork 摘要，并校验 `summary_tokens < pre_tokens`（大负载前提下）与切小窗口模型前优先用上一较大窗口模型预压。
6. **Hotloader 会话基线与首轮请求头落盘**（`crates/pony-tools/src/workspace_hotloader.rs`、`turn_prep.rs`）：
   - `WorkspaceHotloader` 将首轮冻结的宪法/技能基线与 `SessionRequestHeaderSnapshot` 原子持久化至隔离数据目录，重启或内存 LRU 淘汰后自动回填，避免头部前缀漂移。

## Alternatives considered

- **通过 `client` 侧 512 KiB 字节拦截防止超长上下文**：拒绝。与既有决策及 `1M` 模型正常合法载荷冲突，详见 [2026-11-26-request-body-gate-align-gateway-limit](../implemented/bug-fix/2026-11-26-request-body-gate-align-gateway-limit.md)（待合并分支）；必须使用协议级 token 剪枝、压缩与 128 MiB 传输兜底协同治理。
- **在每跳无条件修剪历史 `accumulated_messages`**：拒绝。无条件改写早先 hop 会主动打碎后续 hop 的 KV Cache 前缀命中，仅在超出压缩高水位时触发才能兼顾缓存命中与防爆仓。
- **将 `Volatile` 内容直接拼接进 `User_N` 字符串本体**：拒绝。会污染持久化用户消息文本并导致 UI 展示/标题派生/`align_native_transcript_with_history` 匹配混入易变环境噪声；置于 `User_N` 之后且不落盘是最清洁的分界。
- **用新增 `SessionState` 字段持久化 hotloader 基线**：拒绝。`SessionState` 在存储/测试中有十余处字面量构造且会直接膨胀会话快照 blob；独立按 `(workspace, session)` 哈希落盘与 `WorkspaceHotloader` 单一职责更内聚。

## Consequences

- 改动文件：`crates/pony-protocol/src/{estimate,tool_pruner}.rs`、`crates/pony-provider/src/{compression,provider/mod}.rs`、`crates/pony-tools/src/workspace_hotloader.rs`、`crates/pony-agent-core/src/agent/{turn_prompt_content,context,context_window,turn_flow}.rs`、`crates/pony-agent-core/src/agent/runtime/{compaction_ops,overflow_rescue,tool_exec,tool_recovery,turn_prep,turn_sync,turn_stream}.rs` 及 `crates/pony-agent-core/tests/runtime_suite/context_cache_red_contract.rs`。
- `compression::tests::test_estimate_text_tokens` 与 `test_context_pressure_prefers_larger_side` 及 `context::tests::test_layered_context_message_topology_places_history_before_volatile_context` 按新契约同步更新断言。
- 靠 review 边界：Anthropic/OpenAI 实网真实 `cacheReadTokens > 0` 命中依赖真实 API key，本地通过消息拓扑与跨轮前缀字节级全等断言锁定不变量。
