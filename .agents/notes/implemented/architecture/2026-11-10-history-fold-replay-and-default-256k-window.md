# Agent Note: 会话历史的折叠口径归一（squash 可重放 + 失败轮检查点）与默认窗口 256K

Status: implemented

- 日期：2026-11-10
- 交付面：`crates/pony-agent-core`（projection / session / compression / config / bin）
- 关联：ADR 0007（provider 前缀缓存命中为一等指标）、ADR 0099（会话日志为写路径真源）、
  `.agents/notes/implemented/bug-fix/2026-11-10-failed-turn-context-capture.md`（P0 失败轮落历史）、
  `.agents/notes/implemented/architecture/2026-11-10-compaction-token-budget-and-log-command.md`（P1 压缩）

## Problem

P0/P1 修好了「失败轮写不进上下文」「压缩只写 blob」，但读路径有两处折叠口径与写路径分叉，
加上默认窗口取值失真，共三处缺陷：

1. **压缩结果对折叠不可见**：`session/history.rs` 的 `message_from_turn_event` 只认
   `user/message` 与 `assistant/message`，`history/squash` 被忽略；而 `TurnEvent::HistorySquash`
   全仓**无人发出**（P1 只落了 `CompactHistory` 日志命令与节点快照）。⇒ 索引投影一丢
   （自愈路径「投影空 → 按事件重建历史」），折叠就把被压缩的旧轮次**整段复活**，压缩等于白做，
   上下文被顶回预算上限。
2. **失败轮折叠成悬空 user**：流式失败的事件流是 `user/message` + `assistant/chunk` +
   `turn/end{error}`，**没有** `assistant/message`。重建路径只折叠后者 ⇒ 重启后该轮只剩一条
   user（模型看不到自己做过什么、卡在哪），`pa_01a1c4d0` 的事故形态换一种形状复现。
3. **默认上下文窗口 1M 使压缩失效**：`config::DEFAULT_CONTEXT_WINDOW_TOKENS = 1_000_000`，
   压缩水位 = `min(0.8×窗口, 窗口−输出预留−headroom)` ≈ 800K，而主流模型真实窗口是
   128K~256K ⇒ 压缩**永远晚于上游溢出**（硬失败，直接终止本轮且丢本轮上下文）。
   TUI 侧另有一份 `1_000_000` 硬编码副本，界面占用率与压缩预算各按一套窗口算。

## Decision

1. **折叠口径只有一份**：删除 `session/history.rs` 自带的 `message_from_turn_event`，重建入口改为
   复用 `projection::HistoryProjectionState`（`history_from_branch_events` 直接
   `fold_all_with_branches::<HistoryProjectionState, _>`；`history_from_turn_events` 逐事件 apply）。
   签名从 `&[TurnEvent]` 改为 `&[(u64, TurnEvent)]`——squash 的边界判据是 seq，丢 seq 就丢边界。
2. **摘要注入与运行时同构**：squash 注入 role=`user`（对齐 `wrap_summary_to_message`）、文本
   **原样**（事件里存的是已包裹文本，不再二次包裹）、位置**插到头部**（对齐 `apply_compression`
   的 `[摘要] + 保留窗口`）；同时清空当前 turn 的过程累积，过程状态不得跨压缩边界复活。
3. **压缩落 `history/squash` 事件**：`SessionStore::commit_compaction` 在写完 `CompactHistory`
   后追加一条 `history/squash`，边界由 `compression::derive_squash_base_seq` 推导——
   折叠结果按 (role, content) 与保留窗口做**后缀对齐**，取最长匹配前一条的 seq。
   退化口径「宁可多压缩，绝不复活」：对不上 ⇒ 只留摘要；折叠为空 ⇒ 0；全部保留 ⇒ 边界压到
   折叠结果之前（用**最小** seq，避免被置顶的摘要（seq 最大）把整段历史压掉）。
4. **失败轮在折叠侧补 assistant**：`HistoryProjectionState` 累积每轮的 `assistant/chunk` 文本与
   `tool/call` 工具名，`turn/end` 非 Completed 且该轮无 `assistant/message` 时，用
   `session::interrupted_turn_checkpoint_text`（与运行时**同一个**纯函数）合成中断检查点，
   `status = Error`。工具步骤行数上限 20 行 + 「其余 N 步已省略」——一轮 90+ hop 全列会让
   检查点本身变成上下文炸弹。
5. **空消息一律不入历史**：`user/message` 空文本、`assistant/message` 空文本且无推理内容都跳过
   （空历史项污染下一轮 carry，下游按空文本判无效；此口径用 P0/PA-174 既有约定）。
6. **默认上下文窗口 = 256K**：`config::DEFAULT_CONTEXT_WINDOW_TOKENS = 262_144`（`pub`），
   TUI / CLI 统一引用该常量，删除各自的 `1_000_000` 副本。水位由此约 180K~205K，
   压缩先于溢出发生。

## Alternatives considered

- **A. 只在重建路径里「记得压缩过」**：比如装载时若发现有 `CompactHistory` 命令就跳过事件重建。
  这会丢掉重建的全部收益（投影空时仍然返回空壳），且把「有没有压缩」变成第三个真源。
- **B. 让 `TurnEvent::HistorySquash` 继续无人发出，只改折叠不落事件**：折叠侧没有边界信息，
  等于什么都没改——第一处缺陷的根因是「事件不存在」，不是「折叠不认」。
- **C. 用会话水位（`event_watermark`）当 squash 边界**：会把保留窗口一起压掉，折叠后只剩摘要，
  重启即丢最近若干轮；用 turn_id 反推也不行——`append_turn` 落历史时不写 `turn_id`。
- **D. 失败轮在运行时补发一条 `assistant/message` 事件**：能让两层天然一致，但会改动
  `turn_flow` 的终态事件面（TUI/桌面按 `assistant/message` 渲染，可能双写正文），且对**存量**
  会话无效（老日志里只有 chunk）。折叠侧合成对存量与新数据同时生效，故取折叠侧。
- **E. 默认窗口维持 1M，只在 UI 上提示**：压缩仍不会触发，问题不变；改小窗口的风险是
  「比真实窗口还小 ⇒ 提前压缩」，代价只是多一次摘要调用，远小于溢出硬失败。

## Verification

- `cargo test -p pony-agent-core --lib`：**1458 passed / 0 failed**。
- 新增门禁：
  - `history_from_turn_events_honours_history_squash`（squash 后旧轮次不得复活、摘要 user 角色置顶）；
  - `history_from_turn_events_synthesises_checkpoint_for_failed_turn`（失败轮必须有 assistant +
    Error 状态 + `[中断轮次检查点]` + 工具名）；
  - `commit_compaction_appends_replayable_history_squash_event`（端到端：压缩落 squash 事件、
    base_seq = 2、重放折叠结果与进程内一致）；
  - `test_derive_squash_base_seq_follows_retained_suffix` / `test_default_context_window_is_256k_and_triggers_before_overflow`。
- `pa174_history_wiring_and_followup_contract` 13、`compression_acceptance` 3、
  `slash_commands_acceptance` 135、`tui_session_resilience_l3b` 4 全绿；`cargo test --no-run` 全目标通过。
- 复跑说明（诚实回执）：收尾复跑时 `src/bin/tui.rs` / `headless.rs` 存在**他人未提交的在途改动**
  （`PermissionMode::from_str` 缺失、早前一处多余 `}`），`bin "pony"` 目标因此编译失败，集成套件
  复跑被该错误阻塞。阻塞点与本批改动无关（本批对 `tui.rs`/`cli.rs` 只把窗口常量改为引用
  `DEFAULT_CONTEXT_WINDOW_TOKENS`）。`cargo test --lib` 不受影响，最终复跑仍为 1458 passed / 0 failed。

## 遗留（未在本批处理）

- 上游溢出兜底（`FailureKind::RequiresRequestMutation` 无消费者）；
- `cacheReadTokens` 端到端门禁（连续两请求第二次起 `cache_hit_input_tokens > 0`）；
- 折叠与运行时的检查点**逐字**等价（工具描述在事件层只有工具名，运行时用的是
  `TurnToolActivity.description`；语义等价已保证，字节不保证）。
