# W3 验证收据（task-10，green 阶段）

- 角色：独立 Test Agent（只写测试，不改生产代码/其它测试，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- 变更范围：**仅新增** `crates/pony-agent-core/tests/w3_team_visibility_test.rs`
  - `git status --porcelain` 唯一相关条目：`?? crates/pony-agent-core/tests/w3_team_visibility_test.rs`
- 未执行 git commit；不联网；不依赖全局运行时/引擎/OnceLock。

## 1. 命令与结果（物理凭证）

```
$ cargo test -q -p pony-agent-core --test w3_team_visibility_test
running 13 tests
.............
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

退出码：**0**

13 = 本文件新增 **10** 个 W3 用例 + 3 个由 `#[path = "../src/bin/tui.rs"] mod tui;` 一并编译进来的既有
`tui::wave43_unit_tests::{test_ask_question_from_pending_mapping, test_pending_ui_request_from_value,
test_permission_request_from_pending_mapping}`（非本次编写，未修改）。

## 2. 用例清单（行号 → 契约）

| 契约 | 测试函数 | 行号 | 关键断言 | 结果 |
| --- | --- | --- | --- | --- |
| 1 | `w3_agents_command_is_registered_and_case_insensitive` | `:78` | `match_command_token("/agents")==Some("/agents")`；`parse_command_input("/agents")==Some`；`/AGENTS`、`"  /Agents  "`、`"\t/AgEnTs\n"` 大小写/空白不敏感 | PASS |
| 1 补充 | `w3_agents_command_follows_first_token_vs_whole_string_rules` | `:105` | 首 token 口径 `/agents worker-1` ⇒ Some；分发口径同串 ⇒ None；`agents`、`/agent` ⇒ None | PASS |
| 2 | `w3_team_status_maps_to_tui_task_status` | `:128` | pending→Pending、in_progress→InProgress、completed→Completed、blocked→Blocked；`COMPLETED`/`Blocked` 大小写不敏感 | PASS |
| 2 补充 | `w3_unknown_team_status_degrades_to_pending` | `:149` | `definitely-unknown-status`、``、`"  "`、`weird_status_xyz` ⇒ 一律 Pending | PASS |
| 3 | `w3_team_tasks_map_fields_verbatim` | `:164` | id/subject/owner/blocked_by（顺序）/write_scopes（顺序）透传；无 owner ⇒ None；输入顺序保持 | PASS |
| 3 补充 | `w3_empty_team_task_list_maps_to_empty_board` | `:215` | 空列表 ⇒ 空看板 | PASS |
| 4 | `w3_roster_with_zero_entries_has_no_selection` | `:228` | 空表：selected=0、`selected_entry()==None`、next/prev 不越界 | PASS |
| 4 | `w3_roster_with_single_entry_stays_at_zero` | `:248` | 单条：next/prev 均停在 0（不 wrap） | PASS |
| 4 | `w3_roster_with_many_entries_clamps_without_wrapping` | `:268` | 3 条：0→1→2，末条 next 夹紧；2→1→0，首条 prev 夹紧；每步 `selected_entry().target` 对齐 | PASS |
| 5 | `w3_available_commands_place_agents_right_after_tasks` | `:327` | `/tasks` 仍在表中；`/agents` 紧随其后（`index(/agents)==index(/tasks)+1`）；两者各只出现一次 | PASS |

## 3. 实现口径说明（为什么 C2 走间接断言）

`tui_task_status_from_team_status` 在 `src/bin/tui.rs:500` 是**私有** `fn`（无 `pub`）。本任务红线
「不改生产代码」⇒ 不能加 `pub` 也不能在 `tui.rs` 内追加 `#[cfg(test)]` 子模块（那是另一个写域）。
因此用例通过**唯一公开投影** `tui_tasks_from_team_tasks`（`tui.rs:512`，内部逐条调用该私有映射）
做等价断言：映射结果直接体现在 `TuiTask::status` 上，覆盖 pending/in_progress/completed/blocked
与未知降级 Pending 全部分支。

`AgentRosterState` / `AgentRosterEntry` / `TuiTask` / `TuiTaskStatus` 经 `pub use protocol::*` 公开，
用例直接访问真实类型，未复制/伪造结构。

## 4. 未覆盖 / 边界

1. 未覆盖 `/agents` 的真实数据装配（`agent_roster_state_from_runtime`：`list_agents` + `inbox_pending_len`
   + `is_driving` + `teammate_session_id`）——那需要全局 roster/引擎装配，违反本任务「不依赖全局态」的
   确定性约束，属集成/端到端范围。
2. 未覆盖 `/agents` 面板的渲染与键盘交互（↑↓/Enter 进入子会话/r 返回/Esc 关闭）——属 TUI 交互层用例。
3. 未覆盖 `sync_tasks_from_store` 的错误分支（`team_task_list()` 失败 → `status_text`）——同上，需全局存储。
4. 未新增/修改任何生产代码与既有测试；`tui.rs` 及其子模块完全只读。
