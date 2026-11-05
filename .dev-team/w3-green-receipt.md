# W3 GREEN 收据（task-9）

- 角色：Executor（实现；不改 W1/W2 冻结测试，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- 变更范围（3 个文件 / +491 / −11，无测试改动、无 git 操作）：
  - `crates/pony-agent-core/src/agent/orchestration.rs`（+42 / −1）
  - `crates/pony-agent-core/src/bin/tui.rs`（+404 / −10）
  - `crates/pony-agent-core/src/bin/tui/protocol.rs`（+56 / −0）
- 无 session schema 变更、无子会话创建逻辑变更（未触碰 session/store.rs）。

## 1. 门禁（物理凭证）

```
$ cargo check -p pony-agent-core
    Finished `dev` profile [unoptimized] target(s) in 0.97s
```

退出码：**0**（无 warning）

```
$ cargo build -p pony-agent-core --bin pony
    Finished `dev` profile [unoptimized] target(s) in 5.66s
```

退出码：**0**（无 warning）

补充：`cargo check -p pony-agent-core --all-targets` 亦退出码 **0**（TUI 测试把 `src/bin/tui.rs` 以 `#[path]` 引入，故该检查覆盖其编译面）。
按红线未跑全量测试。

## 2. A) 队友会话可见性（orchestration.rs）

| 项 | file:line |
| --- | --- |
| 进程内表 `TEAMMATE_SESSIONS: Mutex<Option<HashMap<String, String>>>` | `orchestration.rs:382` |
| 写入 `set_teammate_session(target, session_id)`（忽略 `lead` 与空 id） | `orchestration.rs:388` |
| 读取 `pub fn teammate_session_id(target) -> Option<String>`（`lead` 恒 `None`） | `orchestration.rs:405` |
| 唯一写入点：`drive_teammate_turn_in` 的 `Ok(outcome)` 分支（失败路径不写） | `orchestration.rs:1145` |

语义：只有**成功**的队友 turn 才登记会话 id；失败/超时（孤儿会话）不会被 `/agents` 当成可进入会话。

## 3. B) TUI `/agents` 花名册（tui.rs / tui/protocol.rs）

| 项 | file:line |
| --- | --- |
| `InteractionMode::AgentRoster` | `tui.rs:805` |
| 命令表 `("/agents", "查看 Team 角色与状态")` | `tui.rs:815` |
| `/agents` 分发（toggle） | `tui.rs:8106` |
| `AgentRosterEntry{target,name,role,status,pending_inbox,driving,session_id}` | `tui/protocol.rs:395` |
| `AgentRosterState{entries,selected}` + `selected_entry/next/prev` | `tui/protocol.rs:407-` |
| 状态字段（`agent_roster_state` / `return_session_id`）与 Default | `tui.rs:1594` / `:1596` / `:1666-1667` |
| 作用域复位时清空（`/clear`,`/resume` 共用） | `tui.rs:1799` 附近 |
| 真实快照投影 `agent_roster_state_from_runtime()`：`list_agents` + `inbox_pending_len` + `is_driving` + `teammate_session_id` | `tui.rs:569` |
| 打开/关闭/刷新/选中封装 | `tui.rs:2313`（refresh）/`:2318`（toggle）/`:2328`（close）/`:2336`（selected_agent_target） |
| 渲染注册 | `tui.rs:5412` |
| 渲染面板 `render_agent_roster_panel`（name/role/status/待处理/会话尾号/驱动态 ●，空态“当前没有 teammate”） | `tui.rs:6074` |
| 按键处理块（↑↓ 选择 · Enter 进入 · r 返回 · Esc 关闭） | `tui.rs:7803` |
| 进入实现 `enter_teammate_session` | `tui.rs:6769` |
| 返回实现 `return_to_main_session` | `tui.rs:6805` |
| 可加载判定 `session_is_loadable`（`control_plane.list_sessions()` 成员判定） | `tui.rs:6755` |

按键语义：
- `Enter`：`teammate_session_id(target)` → 无 → 提示“该 teammate 尚无独立会话（未成功运行）”；
  有 → `session_is_loadable` 否 → 提示“该 teammate 会话暂不可由宿主会话存储加载（<id>），无法进入”；
  是 → 先保存 `return_session_id = 当前主会话` → 切 `session_id` → `reset_session_scope()` → 回写
  `return_session_id` → `hydrate_state_from_session(...)` → `mode = Normal`。
- `r`：读 `return_session_id` → 主会话可加载则切回并 `hydrate`（成功后清空返回目标）；否则提示
  “没有可返回的主会话（当前不在队友会话）” / “主会话 <id> 已不可加载”。
- `Esc`：关闭面板（`mode` 回 `Normal`，不清返回目标）；`lead` 行 Enter 明确提示“Lead 即宿主主会话，无需进入”。

数据真实性：成员/待处理/驱动态/会话 id **全部**来自真实运行时（进程内名册/信箱/驱动集合/成功 turn 结果），无假数据；`lead` 排首位，其余按 target 字典序稳定排序。

## 4. C) `/tasks` 接真实 Team 看板

| 项 | file:line |
| --- | --- |
| 状态映射 `tui_task_status_from_team_status`（pending/completed/blocked/in_progress + 历史别名，未知→Pending） | `tui.rs:500` |
| 映射 `tui_tasks_from_team_tasks`（status/owner/blocked_by/write_scopes 全量透传） | `tui.rs:512` |
| `sync_tasks_from_store` 改用 `team_task_list()` | `tui.rs:530` |
| `/tasks` 与 Ctrl+T 仍走同一路径（未改调用点） | `tui.rs` /tasks 分发 + Ctrl+T |
| 面板行新增 `⛔阻塞数` / `写域N` 渲染 | `render_task_board_stub` 行渲染处 |
| 保留 todo 能力 | `tui_tasks_from_todos` 保留并标 `#[allow(dead_code)]`（todo 模块与 `todo_write`/`todo_list` 工具零改动） |

空列表仍走「当前没有任务」空态（`task_board_state = None`），行为与既有一致。

## 5. Enter / r 的实际可达性（如实）

- **`/agents` 面板本身完全可达**：命令表 → `parse_command_input` → `toggle_agent_roster`；渲染在 `ui()` 主面板层。
  打开即读到真实名册（同进程内 `spawn_teammate` 之后可见；`lead` 常驻）。
- **`r`（返回主会话）可达**：只要曾成功进入过队友会话（`return_session_id=Some`），且主会话可加载。
- **`Enter`（进入队友子会话）在当前架构下通常走提示分支**：`HostTurnEngine` 用**独立建栈**
  跑队友 turn（`subagent_runtime.rs` 内 `AgentRuntimeBuilder::new()...build()` ⇒ 独立
  `SessionStore::new()` 实例），其会话写入磁盘 SQLite 但**不会进入宿主 control plane 的内存会话表**，
  因此 `session_is_loadable` 判否 → 显示“暂不可由宿主会话存储加载”。
  该判定是**契约要求**（“若 SessionStore 能加载才进入，否则提示”），不是静默失败：
  - 若宿主会话表里确实存在该 id（例如后续 wave 让队友复用宿主 SessionStore，或宿主重启后按 id
    恢复），`hydrate_state_from_session` 路径与 `/resume` 完全同源，可直接进入；
  - 本波次**未**新增“从磁盘按 id 装载会话”的读取通道（会绕过宿主 control plane 的权威快照）。
- 本次未在真实终端交互验证（无 TTY）：以 `cargo build` + `--all-targets` 编译 + 源码路径审阅为准；
  TUI 行为测试未新增（不改测试红线）。

## 6. 已知冲突 / 未实现 / 风险

1. **冻结测试将被 `/agents` 打破（需 Lead 裁决）**：`tests/slash_commands_acceptance.rs:52` 的
   `CONTRACT_COMMANDS`（11 条）与 `t1_tui_command_table_drops_cost_and_adds_compact` 断言
   “`AVAILABLE_COMMANDS` 必须与契约 C-4 冻结顺序/条目完全一致”。新增 `/agents` 后该断言必红
   （命令表 12 条）。本任务禁止改测试，故**未动该测试**；需测试方在后续 wave 更新 C-4 冻结表，或撤销 `/agents` 入表。
2. `/tasks` 数据源切换后，会话级 todo 不再出现在 `/tasks`（`tui_tasks_from_todos` 保留但未接线到面板）；
   todo 工具/存储未删除。
3. Enter 的“进入子会话”依赖宿主会话表，当前架构下实际几乎只能走提示分支（见 §5）；
   根治需要队友复用宿主 `SessionStore` 或新增只读装载通道（均超出本任务“不改 schema/最小可用”边界）。
4. `/agents` 快照在打开时读取一次（面板内不会自动刷新）；如需最新状态需重开面板（未加 `R` 手动刷新键，避免与既有 `r` 语义冲突）。
5. 未跑全量测试；未做 TUI 终端手工验收（无 TTY）。`cargo check --all-targets` 已证明所有测试目标可编译。
