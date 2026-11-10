# PA-175 TUI 双击 Esc 撤回（会话截断 + 代码还原）

## Basic Info

- ID: PA-175
- Status: Review
- Priority: P1
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-11-10
- Updated At: 2026-11-10
- ADR: `docs/decisions/0113-tui-esc-double-press-rewind.md`
- 实施记录: `.agents/notes/implemented/feature/2026-11-10-tui-double-esc-rewind.md`

## Background

Claude Code 的肌肉记忆是「连按两次 Esc → 撤回菜单」：把会话与它改过的文件退回到某条用户消息之前，
并把那条消息回填输入框。TUI 此前**没有回到过去的入口**（`/clear` 换新会话且不动文件、`/resume` 只切会议），
但底层两个部件都已存在却未接线：会话侧 `HistoryNode` + `checkout_history_node`；文件侧写路径本就会读到旧内容。

## Goal

1. 非流式**双击 Esc** 打开撤回面板（流式中双击仍是「停止本轮」，互不干扰）；`/rewind` 同入口。
2. 两段式：选用户轮次（新→旧）→ 选范围（`恢复代码与会话` / `仅恢复会话`）。
3. 会话回滚到该轮**之前**：只复用既有 checkout 真值路径，不复制截断逻辑。
4. 文件回滚：写工具落盘前抓 pre-image（`(session_id, turn_id)` 键），按「被撤销轮次里最早 pre-image
   取胜」还原，抓取时不存在 ⇒ 删除；超限/失败显式降级。
5. 撤回成功后输入框回填该轮用户消息（真值来自核心，不靠 TUI 内存下标）。

## Scope

- `crates/pony-agent-core/src/bin/tui.rs` + `tui/rewind.rs`（面板/判定/执行接线、`/rewind`、Esc 上膛）
- `crates/pony-agent-core/src/agent/control_plane/rewind_commands.rs`（`list_rewind_points` / `rewind_session`）
- `crates/pony-agent-core/src/agent/session/rewind_store.rs` / `rewind_files.rs`（活动链定位、空根补齐、pre-image 存储）
- `crates/pony-agent-core/src/agent/tools/mod.rs` + `tokens` 写调用点（`write_with_rewind_capture`，行数中性）
- `crates/pony-agent-core/src/bin/tui/queue.rs`（`/rewind` 归入会话变更类命令）

## Non-Goals

- 桌面端 UI 接线（控制面命令已是公共 API）
- 快照配额 / TTL 清理 / 跨会话审计
- `workspace_run_command` 内部脚本产生的文件改动回滚
- 撤回 → fork / 分支保留（既有 `fork_from_history_node` 独立提供）

## Acceptance Criteria

1. `list_rewind_points` 返回活动链上已提交轮次（新→旧）+ 用户消息原文 + 文件数 ✅
2. 撤回第 N 轮 ⇒ 会话历史 = 前 N-1 轮；回执 `user_message` = 第 N 轮用户消息 ✅
3. `ConversationAndFiles` ⇒ 文件还原到区间起点前版本、被撤销轮次新建文件被删除；失败/超限 `degraded` ✅
4. 撤回到**第一轮之前**走空 checkpoint 根路径，且撤完可继续对话 ✅
5. 已取消节点再撤 ⇒ 明确 Err；流式中 `/rewind` 被拒（不得撕裂在飞轮次）✅
6. 双击窗口/会话作用域/清膛、两段式按键裁决、菜单文案归约有单测 ✅

## Review Plan

- 定向门禁：`cargo check --lib/--bin pony`、`tui_rewind_acceptance`(2)、`--lib rewind`(5)、
  `--bin pony rewind`(13)、受影响既有 TUI 测试三套全绿
- 待补：独立 reviewer 对 D3（空根补齐对事件溯源语义的影响）与 D4（fail-open 抓取 + 显式降级回执）做对抗复核

## Current Progress

- 2026-11-10：实现 + 单测/验收测试 + ADR 0113 + 实施记录完成，进入 Review。
- 已知缺口（如实登记）：快照目录无配额清理；脚本内改文件不回滚；`bin/tui.rs` 行数棘轮早已超基线，
  本次净增约 40 行接线；桌面端无入口。
