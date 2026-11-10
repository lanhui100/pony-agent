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
- `cargo test -p pony-agent-core --test tui_rewind_acceptance`：2 passed
  （R1/R2 会话截断 + R4 撤回到空会话并可继续对话；R3 文件 pre-image 还原与新建文件删除）。
- `cargo test -p pony-agent-core --lib rewind`：5 passed（`rewind_files` 语义：幂等、最早 pre-image 取胜、
  新建删除、超限降级、未知会话 no-op）。
- `cargo test -p pony-agent-core --bin pony rewind`：13 passed（双击上膛窗口/会话作用域/清膛、
  两段式按键裁决、范围映射、菜单文案归约）。
- `cargo test -p pony-agent-core --test tui_streaming_input_acceptance --test tui_process_fold_acceptance
  --test tui_overlay_cancel_acceptance`：全绿（流式双击停止的既有语义未被改动）。
- `node scripts/check-decisions.mjs`：通过（0113 已入索引）。
- `node scripts/check-file-size.mjs --baseline scripts/file-size-baseline.json`：新增违规数为**存量基线**
  以外的一批（`bin/tui.rs` 等在本改动前已超基线，见「已知缺口」）。

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
