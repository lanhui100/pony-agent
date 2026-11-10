# Agent Note: 压缩按 token 预算治理，且一次压缩 = 一条 append-only 日志命令

Status: implemented

- 日期：2026-11-10
- 交付面：`crates/pony-agent-core`（compression / runtime / session）
- 关联：ADR 0007（provider 前缀缓存命中为一等指标）、ADR 0099（会话日志为写路径真源）、
  `.agents/notes/implemented/bug-fix/2026-11-10-failed-turn-context-capture.md`（P0）

## Problem

三处独立缺陷同源于「会话上下文体积靠**丢历史**控制」：

1. **隐式滑窗（缓存杀手）**：`DEFAULT_HISTORY_LIMIT = 24` 在每轮提交后
   `history.drain(..keep_from)`（`store.rs` 两处）；`TRANSCRIPT_CONTEXT_MESSAGE_LIMIT = 24`
   在每次请求前再切一刀（`context.rs::recent_transcript_slice`）；`HistoryProjectionState`
   默认上限同样是 24（读路径第三刀）。⇒ 每轮前缀整体前移，provider 前缀缓存每轮全灭；
   单轮 90 个工具 hop 时窗口在**同一轮内**逐 hop 滑动。
2. **压缩对 native 会话无效**：`should_compress(&history)` 只估文本历史，而 native-tool-flow
   会话的 prompt 只取 `provider_native_transcript`（`context.rs` 中 transcript 非空即以它为准）
   ⇒ 真实增长不进计量、压缩触发不了；即便触发，`replace_session_history` 也只改 history。
3. **压缩不可重放**：压缩只走 `replace_session_history` → `save_to_backend()`（blob），
   而装载以**日志为真源**（`merge_log_truth_into_sessions`）⇒ 重启后压缩结果被日志里的旧历史
   覆盖，「压缩了但重启回到未压缩」。`TurnEvent::HistorySquash` 虽有投影实现，全仓无人发出。

附带缺陷：自动压缩（`turn_prep.rs`）在**持有 `sessions` 写锁**时同步调用
`provider.decide_with_tools`（网络 IO），违反契约 C-1 的锁纪律；且与手动 `/compact`
（`compaction_ops.rs`，有 deadline / 空摘要防护 / 并发改写检测）是两套实现。

## Decision

1. **计量口径唯一真源**（`agent/compression.rs`）：`ContextPressure { history_tokens,
   transcript_tokens, measured_input_tokens }`，比较值取
   `max(真实 input token, 两层累加估算)`；`compression_threshold_tokens` /
   `retain_tokens_for` 是从口径派生的唯一函数。`reserved_completion_tokens` 取
   provider 的 `max_output_tokens`（未声明才回落配置兜底），消除「硬编码 2048 vs 配置 4096」分叉。
2. **保留窗口按 token**（DSH 同构）：`retain_tokens = (cw - reserved) × retain_ratio`，
   `split_history_by_tokens` 从尾部累加，切点**必为 user 消息**；最后一轮自身超预算时整轮保留
   （宁可略超，也不把一轮切两半）。
3. **双写裁剪**：`compact_native_transcript(transcript, keep_user_turns)` 以 user 为切点裁剪
   transcript —— 一轮从 user 开始 ⇒ 工具调用与结果天然成对闭合，不会触发
   `sanitize_provider_native_transcript` 的整段清空。保留轮数取压缩后 history 的 user 条数，
   两层口径对齐。
4. **压缩事件化**：新增 `PersistCommand::CompactHistory { node, transcript, summary_message }`
   （`fold` 侧 = 节点 upsert + transcript **整份替换**），由新写入口
   `SessionStore::commit_compaction` 发出；`replace_session_history` 保留为「只换 history」的
   兼容壳并委托到同一路径。日志命令是 append-only 的「一次压缩」事实，重放后
   `apply_index_projection` 由可见节点快照 hydrate 出压缩后的历史。
5. **滑窗降级为紧急上限**：`HISTORY_EMERGENCY_LIMIT = 400`（写路径 / 投影侧 / 常量导出共用同一
   常量），`TRANSCRIPT_CONTEXT_MESSAGE_LIMIT = 400`；trace 保留量 `DEFAULT_HISTORY_LIMIT = 24`
   保持不变（观测数据，不参与上下文）。provider 未声明窗口时 `input_budget_tokens` 回落到
   保守预算 `32_000`，避免「窗口未知 ⇒ 完全不裁」变成无上界。
6. **单一压缩入口**：`turn_prep` 的自动压缩改为「短读判定 → 释放锁 → 调
   `compact_session_with_sessions`」，与手动 `/compact` 共用同一实现（含 deadline、空摘要
   中止、并发追加保留、失败不谎报）。

## Alternatives considered

- **A. 只把三个 24 调大（如 200）**：治标不治本——仍是「按条数丢」，只是滑窗更慢；且 token
  体积无上界，长轮次照样溢出。
- **B. 只做压缩双写、不做事件化**：压缩能生效，但重启丢压缩（日志为真源），用户会看到
  「重启后上下文又变长/又变成未压缩的历史」，且不可审计。
- **C. 复用 `AppendProviderTranscript` 追加语义表达压缩**：表达不了「基线前移」——重放会把
  被摘要覆盖的轮次重新接回去，上下文体积反弹。故必须用**替换**语义的命令。
- **D. 发 `TurnEvent::HistorySquash` 事件代替新命令**：`history_from_turn_events`
  （PA-174 契约冻结，只投影 user/assistant 消息）不消费 squash，事件只影响
  `projection::HistoryProjectionState` 一条视图；且 `FlushEvents` 需要 `turn_id`，压缩不是轮次。
  命令层是唯一能同时覆盖「可见节点快照 + transcript 替换」的载体。
- **E. 保留 `keep_recent_turns` 轮数切分、另加 token 上限**：两套切分口径必然分叉
  （哪个先触发就按哪个切），保留窗口体积不可预测。token 预算是唯一可比的价格。
- **F. 未声明窗口时按 128k 兜底**：会让小窗口模型直接 400（溢出是硬失败）；
  按最小常见窗口 32k 兜底属保守方向，代价只是多裁一点。

## 本次未做（明确遗留）

1. **上游溢出兜底（`FailureKind::RequiresRequestMutation` 的消费者）**：上游报
   `context_window_exceeded` 时「强制压一次 + 重试 1 次」尚未接线。当前靠 token 预算裁剪
   兜住体积，溢出仍会终止该轮。接线点在流式/同步错误分类之后，属独立变更（要新增
   「压到 retain=0 再重试一次」的回路与对应失败语义）。
2. **`history_from_turn_events`（PA-174，契约冻结）不消费 `history/squash`**：压缩重放由
   `CompactHistory` 命令（节点快照 + transcript 替换）承载，事件投影侧不受影响；若将来要让
   「事件流单一真源」也覆盖压缩，需扩该函数并处理存量日志。
3. **cache 命中门禁未加**：`provider_call_records` 已带 `cache_hit_input_tokens`，但「连续两请求
   第二次起命中 > 0」的端到端断言尚未落地（需要可回放的 mock provider 计时序列）。

## 验证回执

- `cargo test --lib`：**1452 passed / 0 failed**
- `cargo test --test slash_commands_acceptance --test compression_acceptance --test tui_session_resilience_l3b`：135 / 3 / 4 全绿
- `cargo test --no-run`：全部测试目标编译通过
- `npm run decisions:check`：通过（117 篇 ADR）
- `npm run size:check`：`context.rs`（3065→3030）与 `store.rs`（5711→5699）已回到基线内；
  仍有仓库既有违规（改动前即超基线），本次未整体刷新基线以免掩盖他人漂移
