# 契约矩阵：`/goal` 斜杠命令 + goal `clear` 动作（wave goal-1）

状态：FROZEN（Lead 冻结，wave goal-1）
上位契约：`docs/architecture/plan-goal-contract.md`（wave-1 C1–C6，不改写）；斜杠命令横向口径沿用
`.dev-team/contract-matrix-slash-commands.md`（C-8/C-9，不改写）。
变更纪律：实现侧不得就地改本契约；任何条目改动必须由 Lead 原地改写并在文末登记单行流水。

## 0. 背景与目标

现状：goal 域能力已在 Rust core 落地（`agent/goal.rs`：create/get/update + CAS revision + 原子 sidecar、
`agent/goal_lease.rs`、`active_goal_for_session` 真值源、PA-171 续轮停止原因），桌面有 `src/stores/goal.ts`
+ `GoalBar.vue`，TUI 有 goal banner 与自动续轮。**缺**的是「人类命令面」：用户必须依赖模型调用
`create_goal` / `update_goal` 工具才能建立或调整目标（对照 DSH `packages/goal/command-goal`）。
同时 `UpdateGoalAction` 没有 `clear`，已完成/已阻塞目标无法移除，只能被下一次 create 静默覆盖。

目标
- 新增 `/goal` 人类斜杠命令（桌面 + TUI），语义与 DSH `command-goal` 对齐并中文化：
  `show | create | edit | pause | resume | complete | clear`。
- 命令执行**不消耗模型轮次**，不把命令串当用户消息发送。
- core 新增 `clear` 能力（删除当前 session 目标记录并原子持久化）。
- 命令的语法/状态机/文案**单一真源在 Rust**（`agent::goal_command`），两端只做接线与渲染。

非目标
- 不实现 goal 的 append-only 事件重放 / `goal/change` fold 重构（DSH 的结构性差异，见 ADR「Alternatives」）。
- 不引入 `GoalErrorCode` 稳定错误码枚举（既有 `Result<_, String>` 口径不变）。
- 不改 `create_goal` / `update_goal` **模型工具**的既有语义与 schema（模型侧仍可静默覆盖）。
- 不改 goal sidecar 文件格式（`GoalData` 字段与 `snake_case` 序列化不变）。
- 不实现 `/goal` 的编辑器附件（DSH 的 attachments）——桌面/TUI 无此通道。

## 1. 契约条目

### C-1 语法与解析（core，纯函数）

文件：`crates/pony-agent-core/src/agent/goal_command.rs`（新文件，由 `agent/mod.rs` 以 `pub mod goal_command;` 声明）

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalCommandRequest {
    Show,
    Create { objective: String },
    Edit { objective: String },
    Pause,
    Resume,
    Complete,
    Clear,
}

/// `args` = `/goal` 之后的剩余文本（调用方负责剥离命令 token）。
/// Err 只表示**语法**错误（edit 缺目标描述），不读存储。
pub fn parse_goal_command(args: &str) -> Result<GoalCommandRequest, String>;

/// 帮助文案真值源（桌面 / TUI / 错误回执共用，禁止两端各写一份）。
pub const GOAL_COMMAND_USAGE: &str = "/goal [<目标>|edit <目标>|pause|resume|complete|clear]";
```

解析口径（全部大小写不敏感、按首个空白切 token）：
1. `args.trim()` 为空 ⇒ `Show`。
2. 首 token 为 `clear` / `pause` / `resume` / `complete` 且**无**额外 token ⇒ 对应请求；带额外 token ⇒ 按 `Create{objective: args.trim()}`（正文不得被吞）。
3. 首 token 为 `edit`：无剩余文本 ⇒ `Err("目标编辑需要提供新的目标描述")`；有剩余 ⇒ `Edit{objective: 剩余 trim}`。
4. 其余任意文本 ⇒ `Create{objective: args.trim()}`。
5. 空 objective 的 `Create` 不可达（第 1 条已覆盖），`Edit` 的空 objective 必 Err。

### C-2 执行与状态机（core，真值源）

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCommandOutcome {
    /// "show" | "create" | "edit" | "pause" | "resume" | "complete" | "clear"
    pub action: String,
    /// 已渲染的中文用户回执（多行；不含 CAS/revision 内部细节）。
    pub message: String,
    /// 变更后的目标快照；`clear` 与「无目标」的 show 为 None。
    pub goal: Option<GoalData>,
}

pub fn execute_goal_command(session_id: &str, args: &str) -> Result<GoalCommandOutcome, String>;
```

语义（不可偏离）：
1. `session_id` 先 `trim()`；空 ⇒ `Err("会话 ID 不能为空")`。
2. `Show` 恒 `Ok`：无目标 ⇒ `message = "当前会话未设置目标。\n" + GOAL_COMMAND_USAGE`，`goal=None`；有目标 ⇒
   多行：`目标状态: <中文相位>` / `目标: <objective>` / `轮次: <rounds_started>/<max_goal_rounds>` /
   `激活: <armed|disarmed>` / 阻塞原因（仅 blocked）/ `可执行命令: <按相位>`;
   `goal=Some(当前快照)`。
3. `Create`：读取当前目标；若存在且 `phase != "completed"` ⇒
   `Err("当前已存在{相位中文}目标；请用 /goal edit <目标> 修改，或先 /goal clear 清除")`；
   否则调用既有 `create_goal`（completed 或不存在均可覆盖），`action="create"`，`message` 前缀 `目标已创建`。
4. `Edit`：无目标 ⇒ `Err(missing_goal("edit"))`；当前 `phase == "completed"` ⇒ 走 `create_goal` 新建
   （`action="create"`，文案 `目标已创建`，与 DSH 对齐）；否则用当前 `id`/`revision` 调既有 `update_goal`
   （`UpdateGoalAction::Edit`，只带 objective），`action="edit"`，文案 `目标已更新`。
5. `Pause` / `Resume` / `Complete`：无目标 ⇒ `Err(missing_goal(<动词>))`；
   否则用**当前** `id`+`revision` 调 `update_goal`，`action` 同名，文案 `目标已暂停|已恢复|已完成`。
6. `Clear`：无目标 ⇒ `Ok{action:"clear", message:"当前没有可清除的目标。", goal:None}`；
   有目标 ⇒ 调 C-3 `clear_goal(session_id)`，`message="目标已清除。"`，`goal=None`。
7. 任何 `persist` 失败必须原样向上传播 `Err`，**不得**返回「成功但未落盘」。
8. 命令只碰 goal store，**不得**触碰会话历史 / 不得发起 turn。
9. `missing_goal(<action>)` 文案：`当前未设置目标；/goal {action} 需要一个目标。\n{GOAL_COMMAND_USAGE}`。

### C-3 core `clear` 能力

文件：`crates/pony-agent-core/src/agent/goal.rs`（追加，不改既有函数）

```rust
/// 删除该 session 的目标记录并原子持久化；返回是否真的删掉了记录。
pub fn clear_goal(session_id: &str) -> Result<bool, String>;
```

语义：session 空 ⇒ `Err`；无记录 ⇒ `Ok(false)`（幂等，不落盘）；有记录 ⇒ 内存删除 + `persist_goal_store`，
失败时把原记录放回并 `Err`。**不**新增 `UpdateGoalAction::Clear`（避免枚举扩散到模型工具 schema 与
`src/types/goal.ts`）。

### C-4 Tauri 命令（桌面接入面）

文件：`src-tauri/src/lib.rs`

```rust
#[tauri::command]
fn goal_command(session_id: String, args: String) -> Result<Value, String>;
```

实现：`pony_agent_core::agent::goal_command::execute_goal_command(&session_id, &args)`，
`serde_json::to_value(outcome)`；必须注册进 `invoke_handler!(...)`（`goal_update` 之后）。
参数/返回一律 `camelCase`（`rename_all` 已覆盖）；前端 invoke 名为 `goal_command`。

### C-5 桌面斜杠命令

文件：`src/lib/chat/slash-commands.ts`（纯函数 + 纯元数据，禁止引入 pinia/Tauri/DOM）

1. `DesktopSlashCommand` 联合类型增加 `"goal"`。
2. `DESKTOP_SLASH_COMMANDS` 增加
   `{ name: "goal", usage: "/goal", description: "设置或查看当前会话目标" }`。
3. 新增导出：
   ```ts
   /** `/goal` 之后的参数文本（未命中 `/goal` ⇒ 空串）；首 token 大小写不敏感。 */
   export function goalCommandArgs(input: string): string;
   ```
   口径：`trim()` 后必须以 `/` 开头、首 token 等于 `goal`（大小写不敏感）；返回首个空白之后的**原文**（不做二次 trim 以外的改写）；否则 `""`。

文件：`src/components/HomeWorkspace.vue`
4. 分发：`parseSlashCommand(draftMessage) === "goal"` ⇒ 调 `safeInvoke<GoalCommandOutcome>("goal_command", { sessionId, args: goalCommandArgs(draftMessage) })`；
   成功后把 `outcome.message` 作为**本地系统消息**追加（与 `/compact` 回执同一渲染通道），随即
   `useGoalStore().fetchGoal(sessionId)` 刷新目标条；失败把错误文案以同一通道如实呈现（不得谎报成功）。
5. 提交 `/goal` 后清空草稿，**不得**把命令串作为用户消息发给模型。

### C-6 TUI 斜杠命令

文件：`crates/pony-agent-core/src/bin/tui/goal_command.rs`（新文件）+ `bin/tui.rs`（仅接线）+ `bin/tui/queue.rs`

1. `AVAILABLE_COMMANDS` 在 `/plan` 之后插入 `("/goal", "设置或查看当前会话目标")`（`/help` 自动列出）。
2. **新的分发口径**：`/goal` 是唯一「允许带参数且仍算命令」的斜杠命令。判定纯函数落在
   `tui::goal_command::goal_command_args(input: &str) -> Option<String>`：
   `match_command_token(input) == Some("/goal")` ⇒ 返回首 token 之后的剩余文本（可为空串），否则 `None`。
   其余命令继续走 `parse_command_input` 整串匹配（`/clear 这个需求怎么拆` 仍按普通消息发送）。
3. 命中后：`execute_goal_command(&state.session_id, &args)`，把 `Ok(msg.message)` 以
   `MessageRole::System` 推入 `state.messages`，并 `state.set_goal(TuiGoal::from(goal))`（`Some` 时）；
   `Err(e)` ⇒ 同样以 System 消息呈现错误文案并同时 `state.set_status_notice`，**不得** panic / 不得静默。
4. 流式期：`/goal` 归「本地控制类」⇒ `is_panel_command` 之外**新增**判定
   `is_local_control_command`（`/goal` 唯一成员）并纳入流式 `Forward` 白名单；
   **不得**归入 `is_session_mutating_command`（它不拆在飞轮次）。
5. `unknown_slash_command` 必须把 `/goal ...`（含带参形态）判为已知。

### C-7 不变式与回归保护

1. 未知命令（含 `/goa`、`/goalx`）在两端仍按普通消息发送 / 提示未知命令 —— 既有行为零变化。
2. `/goal` 命中后不产生任何模型请求；不改变会话历史长度。
3. goal 续轮、`active_goal_for_session`、CAS revision 语义零变化。
4. `clear` 之后 `active_goal_for_session(session)` 必须返回 `None`（续轮立即停）。

## 2. 验收（机械收口）

- `cargo check -p pony-agent-core --all-targets` 0 error（含 TUI bin target）。
- `cargo test -p pony-agent-core --lib` 与 goal/goal_command/TUI 相关既有测试全绿。
- `npm test`（含新增 `slash-commands` 单测）全绿。
- `npm run typecheck` 0 error。
- `npm run size:check` 通过（`tui.rs` 只允许少量接线增量）。
- `npm run adr:check` 通过（本波 ADR 已就位）。
- `bash .meta/gates/backlog-gate.sh` 通过。

## 3. 流水

- 2026-11-12 Lead：创建本契约（wave goal-1），冻结 C-1..C-7。
- 2026-11-12 Lead：`tests/slash-commands.spec.ts` 的「恰好三条命令」断言原地改写为四条（新增 `goal`），
  理由：C-5 扩列 `DESKTOP_SLASH_COMMANDS`，旧断言（C-8）与本契约冲突，按上位契约以本波次为准。
