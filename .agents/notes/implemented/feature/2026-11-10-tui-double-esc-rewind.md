# Agent Note: TUI 双击 Esc 撤回（ADR 0113 实施记录）

Status: implemented

- 日期：2026-11-10
- 决策真源：[`docs/decisions/0113-tui-esc-double-press-rewind.md`](../../../docs/decisions/0113-tui-esc-double-press-rewind.md)
- 交付面：`pony`（TUI 二进制）+ `pony-agent-core` 公共控制面

## Context

TUI 此前只有「Esc = 停止本轮 / 清空草稿」，没有把对话退回某个用户轮次的能力，也无法连带撤销
该轮之后由工具写入的文件。Claude Code 的心智是「双击 Esc 打开撤回菜单」：非流式双击 Esc
（或 `/rewind`）打开两段式面板，选轮次 + 选范围后截断会话、可选还原文件，最后把撤回点的
用户消息回填输入框，便于改写后重发。背景与备选方案见 ADR 0113。

## Decision

改动面按层：

1. **交互层**（`crates/pony-agent-core/src/bin/tui.rs` + `tui/rewind.rs`）
   - `InteractionMode::Rewind` + `TuiState::rewind_state`；
   - 非流式 Esc 上膛（`RewindEscArm`，会话作用域 + 700ms），第二次开面板；
     流式双击仍只停止本轮（两条判定互斥）；
   - 两段式面板：选轮次（新→旧，右侧标可回滚文件数）→ 选范围（恢复代码与会话 / 仅恢复会话）；
   - `/rewind` 命令同入口，并归入 `is_session_mutating_command`（流式期拒绝）；
   - 成功后 `reset_session_scope` + `hydrate_state_from_session`（与 `/resume` 同口径）+ 回填用户消息。
2. **控制面**（`agent/control_plane/rewind_commands.rs`）
   - `list_rewind_points`：活动链上的已提交轮次（新→旧）+ 用户消息原文 + 抓到的文件数；
   - `rewind_session`：先文件还原、后 `checkout_history_node(TranscriptOnly)`；回执带
     `files_restored/deleted/skipped/failures + file_restore_degraded`。
3. **会话层**（`agent/session/rewind_store.rs`、`rewind_files.rs`）
   - `rewind_activity_chain` / `rewind_turns` / `rewind_plan` / `rewind_user_message_for_node`；
   - `ensure_rewind_root_checkpoint`：撤回到第一轮之前所需的空 checkpoint 根（幂等）；
   - `RewindFileStore`：`(session, turn)` 键的 pre-image 抓取与还原，超限/失败显式降级。
4. **写路径**（`agent/tools/mod.rs`、`agent/tools/workspace.rs`）
   - 单点 `write_with_rewind_capture`：先抓 pre-image 再 `fs::write`；两个写调用点**行数中性**替换
     （`store.rs` / `tools/workspace.rs` 都在文件行数棘轮里，禁止净增）。

## 机器回执

- `cargo check -p pony-agent-core --lib` / `--bin pony`：通过。
- `cargo test -p pony-agent-core --test tui_rewind_acceptance`：3 passed
  （R1/R2 会话截断 + R4 撤回到空会话并可继续对话；R3 文件 pre-image 还原与新建文件删除；
  R5 空会话 checkpoint 根与截断跨存储重载存活）。
- `cargo test -p pony-agent-core --test tui_rewind_capture_seam`：3 passed（真实
  `workspace_write_file` 落盘前的抓取接缝：幂等 / 最早 pre-image 取胜 + 新建删除 / 缺 turn 事实不抓 /
  抓取失败 fail-open 不阻断写入）。
- `cargo test -p pony-agent-core --lib rewind`：5 passed（`rewind_files` 语义：幂等、最早 pre-image 取胜、
  新建删除、超限降级、未知会话 no-op）。
- `cargo test -p pony-agent-core --bin pony rewind`：13 passed（双击上膛窗口/会话作用域/清膛、
  两段式按键裁决、范围映射、菜单文案归约）。
- `cargo test -p pony-agent-core --test tui_streaming_input_acceptance --test tui_process_fold_acceptance
  --test tui_overlay_cancel_acceptance`：全绿（流式双击停止的既有语义未被改动）。
- `node scripts/check-decisions.mjs`：通过（0113 已入索引）。
- `node scripts/check-file-size.mjs --baseline scripts/file-size-baseline.json`：新增违规数为**存量基线**
  以外的一批（`bin/tui.rs` 等在本改动前已超基线，见「已知缺口」）。

## 对抗复核（D3 / D4，2026-11-10）

复核口径：只信机器回执，不复述实现意图。两条疑点各自补了**可失败的**验收用例，未发现实现缺陷，
但暴露出两个原先没有证据的空白。

- **D3（空根补齐对事件溯源语义的影响）= 通过，补证据**。疑点：`ensure_rewind_root_checkpoint`
  在内存里合成一个**无事件承载**的空 `Checkpoint` 根，并改写现有无父节点的 `parent_node_id`；
  若这份改写不落盘，重启后空根消失 ⇒ 新轮 `keep_node_id` 指向不存在的节点，撤回到空会话直接 Err，
  截断状态也可能复活。证据：`tui_rewind_acceptance::r5_rewind_to_empty_survives_store_reload`
  在同一落盘文件上新建会话存储（等价重启），断言（a）会话仍为空且已撤销轮次不被复活；（b）空根
  **保留同一 node_id**；（c）重启后新轮 `parent_node_id` 正是该空根，二次撤回到空会话成功。
  落盘路径成立的结构性原因：`session_is_persistable` 显式保留 `history_nodes` 非空的会话
  （截断后仍持久化），整包快照走 `commit_store`。
- **D4（fail-open 抓取 + 显式降级回执）= 通过，补证据**。疑点：抓取键 `(session_id, turn_id)`
  取自 `ToolExecutionContext.dispatch_context`，与撤回侧读的节点 `turn_id` 若不是同一份事实，
  还原会**静默空转**（`files_restored` 为空但 `degraded=false`）。证据：
  `tui_rewind_capture_seam`（新文件）走真实 `ToolRouter::execute_with_context` →
  `workspace_write_file`：同轮同路径只抓一次、跨轮撤回复原到最早 pre-image 且删除区间内新建文件、
  缺 `turn_id` 时写入照做但不产生快照目录、快照根不可写时写入仍成功（fail-open）。
- 复核后仍存在的**已知缺口**（不是缺陷，是范围）：抓取失败在撤回侧**不可区分于「该轮没改文件」**——
  二者都表现为 0 文件可还原，`degraded` 只在「有快照但用不了」时置位。要消歧需在抓取失败时落一条
  `capture_failed` 标记并计入降级回执，属后续增强（与快照 TTL 清理同批）。

## 已知缺口 / 后续

1. **快照目录无配额与清理**：撤回后旧轮次的快照成为孤儿（不还原、不删除）。后续可加「会话删除时
   `clear_session` + 定期 TTL 清理」。
2. **脚本内部改文件不回滚**：`workspace_run_command` 里执行的 `sed`/`git checkout` 等改动不在 pre-image
   范围（只覆盖 `workspace_write_file` / `workspace_edit_file`）。
3. **`bin/tui.rs` 文件行数棘轮早已超基线**（本次净增约 40 行：模式变体、状态字段、两处接线、渲染入口）。
   面板本体、执行逻辑、纯判定都在新文件 `tui/rewind.rs`，后续再拆应改为把接线也搬进子模块导出函数。
4. **桌面端未接线**：`rewind_session` 是公共控制面命令，桌面 UI 目前没有入口（本次范围仅 TUI）。

## Alternatives considered

- 见 ADR 0113「Alternatives considered」六条（git 回滚 / 复用 W32 快照管理器 / 全工作区快照 /
  只做会话 / fork 语义 / 继续堆进 `store.rs` 与 `control_plane/mod.rs`）。此处不重复。
- **验收测试用 SQLite 还是文件后端**：先用 `SessionStore::new()`（SQLite 索引路径），发现沙箱下
  `turn_events` 空导致节点折叠为空视图、且 ask-aware 路径会给两个节点写同一 `turn_id`；改为
  `FileSessionBackend`（无事件表 ⇒ legacy 节点快照真值），行为可复现且不依赖沙箱装载细节。
  SQLite + 事件的 checkout 路径由既有 `control_plane` 内测覆盖，不在此文件重复。
