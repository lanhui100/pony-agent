# 契约矩阵：斜杠命令（桌面 /plan /tasks /compact + TUI 收敛与修复）

状态：FROZEN（Lead 冻结，波次 wave-slash-1）
变更纪律：任何条目改动必须在对应行原地改写，并在文末登记单行流水；实现侧不得就地改契约。

## 0. 目标与非目标

目标
- 桌面端（Vue + Tauri）实现 `/plan`、`/tasks`、`/compact` 三个斜杠命令。
- TUI 移除 `/cost`；新增 `/compact`；修复现有斜杠命令缺陷（菜单行数硬编码、命令匹配不支持大小写/尾随参数、空态与 help 与命令表漂移）。
- `/compact` 必须是**真实上下文压缩**（复用 `agent/compression.rs`），不是"发一条总结提示词"。

非目标
- 不实现金额计价 / USD 成本核算（A16/C11 仍为独立欠账）。
- 不实现桌面端 `/clear`、`/help`、`/quit`、`/resume`、`/provider`、`/models`、`/thinking` 的斜杠形式（桌面已有等价 UI）。
- 不改变桌面既有"未知 `/xxx` 按普通消息发送"的现有行为（见 C-9）。

## 1. 契约条目

### C-1 手动压缩核心 API（Rust，core）

文件：`crates/pony-agent-core/src/agent/runtime/compaction_ops.rs`（新文件，由 `runtime/mod.rs` 声明 `mod compaction_ops;`）

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualCompactionOutcome {
    pub compacted: bool,
    pub reason: String,          // "compacted" | "history_too_short" | "below_threshold"
    pub compressed_count: usize,
    pub kept_count: usize,
    pub before_tokens: usize,
    pub after_tokens: usize,
}

impl AgentRuntime {
    pub(crate) fn compact_session(
        &self,
        session_id: &str,
        force: bool,
    ) -> Result<ManualCompactionOutcome, String>;
}
```

语义（不可偏离）：
1. 先从 `SessionStore` 取历史快照后**释放锁**，再发起 LLM 摘要调用；**禁止**在持有 `sessions` 写锁期间做网络调用。
2. `force == false` 且 `should_compress(...) == false` ⇒ `Ok{compacted:false, reason:"below_threshold"}`，不改写历史。
3. `split_history` 得到 `to_compress` 为空 ⇒ `Ok{compacted:false, reason:"history_too_short"}`，不改写历史。
4. 摘要请求：`build_compression_system_prompt()` 作 system，`to_compress` 各条 content 作 user 消息，`temperature 0.3`，`max_output_tokens 2048`（与 `turn_prep.rs:56-64` 一致）。
5. LLM 失败 ⇒ `Err(String)`，**不得**静默吞掉、不得 panic、不得回落到"未压缩但报成功"。
6. 成功 ⇒ `apply_compression` + `replace_session_history(Some(session_id), result.history)`，返回真实计数与压缩前后 `estimate_history_tokens`。

### C-2 控制面命令（Rust，core）

文件：`crates/pony-agent-core/src/agent/control_plane/`（建议新增 `compaction_commands.rs` 并在 `mod.rs` 声明）

```rust
impl HostControlPlane {
    pub fn compact_session(
        &self,
        session_id: Option<&str>,
        force: bool,
    ) -> Result<ManualCompactionOutcome, String>;
}
```
- `session_id == None` ⇒ 回落 `DEFAULT_SESSION_ID`（与 `SessionStore` 既有默认一致）。
- 内部通过 `self.runtime.read()` 调用 C-1，锁毒化按既有 `unwrap_or_else(|e| e.into_inner())` 惯例恢复。

### C-3 Tauri 命令（桌面宿主）

文件：`src-tauri/src/lib.rs`

```rust
#[tauri::command]
fn compact_session(
    control_plane: State<'_, HostControlPlane>,
    session_id: Option<String>,
    force: Option<bool>,
) -> Result<ManualCompactionOutcome, String>;
```
- 必须注册进 `invoke_handler`。
- `force` 缺省按 `true` 处理（用户主动触发即强制）。
- 序列化字段为 camelCase（由 C-1 的 `serde(rename_all = "camelCase")` 保证）。

### C-4 TUI 命令表（唯一真值源）

`crates/pony-agent-core/src/bin/tui.rs::AVAILABLE_COMMANDS` 变更后**必须**为（顺序固定，共 10 条）：

```
/resume  /provider  /models  /thinking  /plan  /tasks  /agents  /clear  /compact  /help  /quit
```
- `/cost` 从命令表与分发分支**移除**。
- `/cost` 从 `bin/cli.rs` 移除（CLI 与 TUI 口径统一，`SlashCommand::Cost` 枚举、`parse` 分支、help 文案、`cli_acceptance.rs` 同步一并处理）。
- `/compact` 描述文案：`压缩当前会话上下文`。
- `/quit` 描述保持 `安全退出 TUI 界面`。

### C-5 TUI 命令解析（修复）

- 解析口径：取 `trim()` 后**首个空白分隔 token** 做命令比较，**大小写不敏感**；因此 `/PLAN`、`/plan `、`/compact now` 均命中对应命令。
- 未命中任何已知命令的 `/xxx`：保持现状——作为普通消息发送给模型（不新增拦截）。
- `/help` 输出必须遍历 `AVAILABLE_COMMANDS`（现状已满足），不得手写命令清单。

### C-6 TUI 菜单与空态容量（修复）

- 斜杠菜单行数上限**不得硬编码 8**：`rows = match_cmds.len().min(窗口可用行数)`，窗口不足时才截断（保留 `area.height` 派生上限，最小 3）。
- 空态引导 `ascii_logo()` 的 `take(4)` 允许保留 4 条，但必须与命令表同源（现状已用 `AVAILABLE_COMMANDS`，保持不变）。
- 断言不得退化为"放宽阈值"：审计脚本 `tools/tui-audit/checks.py` 的命令期望必须**从 `tui.rs` 的 `AVAILABLE_COMMANDS` 解析推导**，不得再写死命令字面量清单。

### C-7 TUI `/compact` 执行语义

- 必须在**后台线程**执行（复用 `spawn_model_discovery` 同款 `Sender<UiEvent>` 模式），**禁止**在事件循环内同步阻塞调用 LLM。
- 新增 `UiEvent` 变体承载成功/失败；成功时向转录区追加 System 消息，文案含 `compressed_count` / `kept_count`；失败追加 System 消息含错误摘要。
- `below_threshold` / `history_too_short` 属**成功但未压缩**，文案必须如实说明"历史过短，无需压缩"，不得谎报已压缩。

### C-8 桌面端斜杠命令（Vue）

新文件：`src/lib/chat/slash-commands.ts`（纯函数，可独立单测）

```ts
export type DesktopSlashCommand = "plan" | "tasks" | "compact";
export const DESKTOP_SLASH_COMMANDS: ReadonlyArray<{
  name: DesktopSlashCommand;
  usage: string;
  description: string;
}>;
/** 仅精确匹配已知命令（首 token、大小写不敏感、允许尾随空白/参数）；未知或非命令返回 null。 */
export function parseSlashCommand(input: string): DesktopSlashCommand | null;
```

行为契约：
- `parseSlashCommand("/plan")`、`"/PLAN"`、`"  /plan  "` ⇒ `"plan"`。
- `parseSlashCommand("/etc/hosts 是什么")` ⇒ `null`（按普通消息发送，保持既有行为）。
- `parseSlashCommand("/tasks2")` ⇒ `null`。
- 命中命令时，`HomeWorkspace` 的提交流程**不得**调用 `runtimeStore.submitTurn()`，并且必须清空草稿。

命令动作：
- `/plan` ⇒ 切换右侧栏"计划"面板；面板无数据（`planStore.plans` 与 `planStore.todos` 皆空且 `goalStore.hasGoal === false`）时改为提示"当前会话暂无计划"。
- `/tasks` ⇒ 切换右侧栏"任务看板"面板；无数据（`teamStore.tasks.length === 0`）时提示"当前暂无 Team 任务"。
- `/compact` ⇒ `invoke("compact_session", { sessionId, force: true })`；成功后展示归因文案（含 compressedCount / keptCount），失败展示错误文案。

面板状态必须**可被外部驱动**：`HomeSidebar` 现有组件内 `activePanel` ref 提升为共享 store（`src/stores/workspace-panels.ts`），由 `HomeSidebar` 与斜杠命令处理器共同消费。

### C-9 桌面端反馈通道

- 新增 `runtimeStore.sessionNotice: string | null` + `setSessionNotice(text)`；状态栏（`AgentRuntimeStatusBar` 消费的既有 getter）在无错误、无提交时以 `mode: "info"` 展示该文案。
- 提交新消息时清空 `sessionNotice`。
- 不得复用 `sessionError` 承载成功文案（语义错误）。

### C-10 可观测性声明

`docs/observability.yaml` 必须为新增的 Tauri 命令（对外接口）与手动压缩（LLM 调用）更新四维归属；N/A 必须写结构性理由。

## 2. 验收测试清单（Test Agent 冻结，实现侧只读）

Rust：`crates/pony-agent-core/tests/slash_commands_acceptance.rs`
- T1 命令表不含 `/cost`，含 `/compact` `/plan` `/tasks`，且 `/quit` 存在。
- T2 `matching_commands()` 在输入 `"/"` 时返回全部命令（数量等于 `AVAILABLE_COMMANDS.len()`），不再被 8 行上限截断。
- T3 大小写/尾随参数解析：构造 TuiState 后按 C-5 口径验证（若纯函数不可达，则抽 `match_command_token(input) -> Option<&'static str>` 纯函数并测试）。
- T4 `compact_session(force=true)` 在历史过短会话上返回 `compacted=false / reason="history_too_short"`，且历史未被改写。
- T5 `compact_session` 在历史足够长且 provider 不可用时返回 `Err`（不得返回假成功）。

TS：`tests/slash-commands.spec.ts`
- T6 `parseSlashCommand` 的命中/不命中矩阵（含 `/etc/hosts`、`/tasks2`、大小写、首尾空白）。
- T7 命中命令时 `submitTurn` 不被调用且草稿被清空。
- T8 `/compact` 触发 `invoke("compact_session", { sessionId, force: true })` 并写入 notice 文案。
- T9 未知 `/xxx` 仍走普通提交路径。

## 3. 门禁（Lead 执行，物理收据）

| 门禁 | 命令 | 阈值 |
|---|---|---|
| G1 前端单测 | `npx vitest run tests/slash-commands.spec.ts` 与 `npm run test:unit` | Exit 0 |
| G2 前端类型 | `npm run typecheck` | Exit 0 |
| G3 Rust 定向测试 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test slash_commands_acceptance` | Exit 0 |
| G4 Rust 编译 | `cargo check --manifest-path crates/pony-agent-core/Cargo.toml` 与 `cargo check --manifest-path src-tauri/Cargo.toml`（若本地缺 tauri 系统依赖则记录为 `[GATE DEGRADED]` 并说明） | Exit 0 |
| G5 审计脚本静态 | `python3 -c "import ast,sys;ast.parse(open('tools/tui-audit/checks.py').read())"` + 命令期望从 tui.rs 推导的单元自检 | Exit 0 |
| G6 回归 | 既有 `cargo test --test cli_acceptance --test tui_acceptance --test tui_render_layout`（若因 /cost 移除需同步更新，更新由 Test Agent 执行） | Exit 0 |

## 4. 写域（write_scopes）

- exec-rust：`crates/pony-agent-core/src/`、`src-tauri/src/`、`tools/tui-audit/`、`docs/observability.yaml`
- exec-web：`src/`
- test-agent：`crates/pony-agent-core/tests/`、`tests/`、`docs/release/evidence/slash-commands/`

## 变更流水

- 2026-11-01 wave-slash-1：初版冻结（Lead）。
- 2026-11-01 wave-slash-1 修订1（Lead 批准）：**C-3 签名**改为 `async fn compact_session(app: AppHandle, session_id: Option<String>, force: Option<bool>)` + `BlockingHelper::spawn` 内取 state —— 原 `State<'_, HostControlPlane>` 不可跨线程，且同步命令会阻塞 WebView IPC；命令名/参数名/`force` 缺省 true/返回形状不变，属契约兼容修订（与 `health_check`/`list_sessions` 既有惯例一致）。
- 2026-11-01 wave-slash-1 修订3（Lead 批准·审查 F2b）：手动压缩的摘要 LLM 调用增加墙钟上界 **60s**（`COMPACTION_TIMEOUT_SECS`），超时错误码 `compaction_deadline_exceeded`；真流式 provider 同时接 `ProviderStreamGuard`（同 deadline + 取消位），非流式路径由外层 `recv_timeout` 兜底。同时要求网络调用期间**不持有** `runtime` 读锁与 `sessions` 锁（先取 ProviderManager 与历史快照、克隆 `sessions_rwlock` Arc 再放锁），防止阻塞 `runtime.write()` 侧命令。
- 2026-11-01 wave-slash-1 修订4（Lead 批准·审查 F1/F3/F4）：(a) `docs/observability.yaml` 的 tracing 归属必须**如实声明**手动压缩当前不产生 span（不在 turn 作用域），并登记欠账项，不得声称复用 `llm.chat` 子 span；(b) 摘要为空串/纯空白 ⇒ `Err`，禁止以空摘要替换历史并报成功；(c) 抽取纯函数 `pub fn compact_outcome_message(&Result<ManualCompactionOutcome, String>) -> String` 作为 TUI/CLI 唯一文案出口，供验收测试断言"不谎报已压缩"。
- 2026-11-01 wave-slash-1 修订6（Lead 批准·批1 修复）：三项
  (a) **F-A 窄终端崩溃**：斜杠菜单宽度 `need_w.clamp(28, content.width.min(64))` 在 24–33 列必 panic（`content.width = area.width-6` ⇒ 18/22/27 < 28，`Ord::clamp` 断言 `min <= max` 失败，已用 rustc 物理复现）。改为恒满足 `min_w <= max_w` 且结果落在 `[1, max_w]`；既有 `menu_rect.y > chat_rect.y` 跳过守卫保留。
  (b) **F-B 命令吞正文回归**：新增 `pub fn parse_command_input(input: &str) -> Option<&'static str>` —— **整串**（trim 后）大小写不敏感等于命令表 token 才返回规范字面量；首 token 命中但存在额外 token ⇒ `None`（整串按普通消息发送）。**与 `match_command_token` 是两个独立函数、语义不同**：后者用于菜单补全/高亮（允许尾随参数，既有 T3 断言 `match_command_token("/compact now") == Some("/compact")` 不变）；前者用于提交拦截。分发处必须用 `parse_command_input`。
  (c) **F-C 多行粘贴**：进入终端 `EnableBracketedPaste`、恢复路径（含 panic catch 之后）`DisableBracketedPaste`；输入线程把 `Event::Paste` 转 `UiEvent::InputPaste(String)`；`pub fn normalize_paste(&str) -> String`（`\r\n`/`\r` → `\n`）；`TuiState::insert_paste(&mut self, &str)` 插入后光标在末尾且**绝不提交**（不置 `is_streaming`、不 push `messages`）；流式中沿用既有守卫。
- 2026-11-01 wave-slash-1 修订7（Lead 批准·批2 修复）：
  (a) **F-D 终端 RAII**：新增 `TerminalGuard { restore: Option<Box<dyn FnMut()>> }`，`new(impl FnMut() + 'static)`、幂等 `restore(&mut self)`、`Drop` 调用且捕获闭包 panic 不传播；两处入口在 `enable_raw_mode()` 成功后立刻建守卫（早于 `Terminal::new()?`/`build_control_plane`），恢复闭包用独立 `io::stdout()` 句柄，顺序 PopKeyboardEnhancementFlags → DisableBracketedPaste → disable_raw_mode → LeaveAlternateScreen+DisableMouseCapture → cursor::Show，全 `let _ =`。
  (b) **F-E 输入泵**：`pub enum InputPumpAction { Continue, Stop }` + `pub fn classify_input_poll(std::io::Result<bool>) -> InputPumpAction`（Ok→Continue，Err→Stop）；`event::poll` 不再 `unwrap_or(false)`，Err 经 `UiEvent::InputPumpFailed { error }` 界面告警后 break，所有 `send` 失败即退出（消除 50ms 热旋）。
  (c) **F-F UB**：线程内 `unsafe env::remove_var("NO_COLOR")` 删除，仅保留两处入口单线程启动期（早于任何 `thread::spawn`）。
  (d) **F-G Esc 真中断**：`GoalRoundCancelToken`（cancel/reset/is_cancelled）+ `should_abort_goal_rounds(cancelled, consecutive, max)`；Esc 置位并 stop_turn，新提交换轮 reset，续轮每轮前检查且每轮回写 `current_turn_id`（`turn-goal-<round>-<uuid>`）。
  (e) 物理冒烟（pty）：SIGINT 优雅退出路径下 enter/leave 序列 1:1 对称（`?1049h/l`、`?2004h/l`、`?1000h/l`），`?25h` 出现 2 次，0 panic。
- 2026-11-01 wave-slash-1 修订8（Lead 批准·批3 修复）：
  (a) **P-1 `/provider` 生效**：`select_current_provider` 必须把选择**持久化**（`ProviderRegistryStore::save_view` 写 `selected_provider_id`），且新增纯函数 `pick_active_provider_id(state_id: Option<&str>, registry_selected: Option<&str>, provider_ids: &[&str]) -> Option<String>`（优先级：state 命中注册表 → registry 选中命中 → 首个 → None）供 `resolve_active_provider_id` 复用。禁止再出现"界面报已切换、发送仍用旧 provider"。
  (b) **P-2 会话作用域复位**：新增 `TuiState::reset_session_scope(&mut self)`，复位 messages/status_text/is_streaming/turn_started_at/runtime_mode/turn_usage/total_input·output·cache_hit/last_context_tokens/context_window_tokens/plan_state/task_board_state/goal_state(+goal_banner_since)/pending_request/scroll_offset/max_scroll_offset/auto_scroll/selected_command_index/spinner_tick 与各选择器索引；`/clear`（另换 session_id）与 `/resume`（复位后再装载快照）共用。
  (c) **P-3 覆盖层 mode 栈**：新增 `mode_stack: Vec<InteractionMode>` 与 `begin_overlay/end_overlay`；审批/问答弹窗不得无条件覆写 mode，结束按栈回退；回到 `AddingProvider` 时必须清空 `input_buffer`（防 api_key 落入 composer 被当消息发出）。
  (d) **P-4 Ctrl+C 语义**：抽 `pub enum CtrlCAction { InterruptTurn, ClearDraft, Exit }` 与 `pub fn ctrl_c_action(is_streaming: bool, has_draft: bool) -> CtrlCAction`（streaming → InterruptTurn；有草稿 → ClearDraft；否则 Exit）；流式中 Ctrl+C 等价 Esc（置取消令牌 + stop_turn），**不得退出**。
  (e) **P-5 非流式提示**：新增 `status_notice: Option<(String, std::time::Instant)>` + `set_status_notice`，状态区在非流式期也渲染（TTL 4s，提交时清空）；抽 `pub fn should_render_status_notice(is_streaming: bool, age_ms: u64, ttl_ms: u64) -> bool` 供测试。
- 2026-11-02 wave-slash-1 修订9（Lead 批准·批4 修复）：
  (a) **F-H 滚动偏移 u32**：`TuiState::scroll_offset` / `max_scroll_offset` 由 `u16` 改 `u32`，消除 `max_offset as u16` 截断（超 65535 行即回绕跳顶）；渲染侧用 `pub fn clamp_render_scroll(offset: u32) -> u16`（饱和 cast）喂给 ratatui；所有 `scroll_up/down/to_bottom`、`calculate_scroll_offset`、滚动相关断言随之改型。
  (b) **F-I 每帧去重**：`ui()` 每帧最多调用一次 `workspace_label()`（不再每帧两次 `env::var`）、最多一次 `matching_commands()`（菜单与空态/补全共用同一结果）；禁止在渲染路径 `clone()` 工具正文（`RuntimeMode::ExecutingTool(String)` 改借用或只取所需字段）。
  (c) **F-J 消除复制式测试**：把 `ModelsDiscovered` 的生产映射逻辑抽成纯函数 `pub fn apply_models_discovered(view: &mut ProviderRegistryView, provider_id: &str, models: &[String]) -> Result<(), String>`（含 save_view），事件臂只做调用；`tests/tui_acceptance.rs:687-720` 改为断言该生产函数而非测试内重写。
  (d) **F-K 杀掉恒真审计断言**：`tools/tui-audit/checks.py` 的 `<cap>.no_panic` 目前只扫 tmux capture，而 TUI stderr 已被 dup2 进 `~/.pony-agent/tui-logs/tui-runtime.log` ⇒ 结构上不可能失败。改为**双源**：仍是 capture 无 panic **且** 该日志（若存在）中本次运行的 panic/backtrace 计数为 0；日志缺失时显式标记 skipped（不得静默算通过）。
- 2026-11-02 wave-slash-1 修订10（Lead 批准·批5 修复）：
  (a) **F-L 网关可见**：`/cost` 移除后 gateway 端点在全 UI 无出口。新增 `pub fn gateway_display(url: &str) -> String`（仅显示 `scheme://host[:port]/path`，**必须剥离 userinfo/token/query 中的凭据**，超长按 48 字符 ellipsize），在 header 右侧（模型信息之后）或 `/help` 尾部展示；不得把 API key 或 token 渲染出来。
  (b) **F-M Ask 交互**：①无 options 的 ask 不再伪造"确认/取消"两项，改为"输入回答后 Enter 提交"，`pub fn ask_submit_verdict(has_options: bool, selected: Option<usize>, typed: &str) -> AskSubmitVerdict`（`Reject{reason}` / `Submit`）——无 options 且 typed 为空白 ⇒ Reject；有 options 但未选中且 typed 空白 ⇒ Reject；②多问场景逐个推进：`pub fn next_question_index(current: usize, total: usize) -> NextQuestion`（`Advance(usize)` / `Finish`），`protocol.rs` 的 `current_question_index` 真正推进，答完全部才 `answer_ask`（现状只答第一问就关闭）。
  (c) **F-N `/plan` 诚实化**：plan 模式不接线后端只读语义，界面必须明示。新增 `pub fn plan_mode_banner(active: bool) -> Option<&'static str>`（active ⇒ 一句"计划模式 · 仅界面提示，不限制工具执行"），在状态区展示；`AVAILABLE_COMMANDS` 中 `/plan` 的描述改为如实措辞（不含"只读/不执行"暗示）。
- 2026-11-02 wave-slash-1 修订11（Lead 批准·批6 虚拟化）：
  (a) **P-9 按消息粒度的布局缓存**：`ui()` 不再每帧为全部消息重建 `Line`。TuiState 增 `render_cache`：`cache_width: u16`、`cache_msg_count: usize`、`cache_lines: Vec<Vec<Line<'static>>>`（索引对齐 messages）。每帧规则：宽度变化 / 消息数变化 / 显式 `invalidate_render_cache()` ⇒ 全量重建；否则 `messages[0..n-1]` 直接复用缓存，**仅最后一条重算**（流式内容只影响末条）。`/clear`、`/resume`、`reset_session_scope`、`Event::Resize` 必须调用 `invalidate_render_cache()`。
  (b) **P-10 视口虚拟化**：组装喂给 `Paragraph` 的行时**只取可见窗口**（由 `scroll` 偏移与 `chat_h` 推出 `[start, start+chat_h)`），`Paragraph` 的 scroll 置 0（已切片）；`max_scroll_offset` 语义不变（= 总行数 − 可见高度，u32）。滚动/贴底/阻尼步长语义零变更。
  (c) **P-11 全量文本惰性化**：`rendered_lines_text` 不再每帧重建，改为按需（鼠标选区/复制时）由缓存拼接（`ensure_full_text`），保证 `extract_selected_text` 行为不变。
  (d) **可测性接口**：`pub fn message_lines_for(message: &ChatMessage, width: usize) -> Vec<Line<'static>>`（单条消息布局，缓存与参考路径共用）与 `pub fn render_cache_stats(&self) -> RenderCacheStats { rebuilt_messages, reused_messages, laid_out_lines, total_lines }`。
  (e) **红线**：渲染结果必须与虚拟化前**逐格一致**（同 fixture/同宽度/同滚动位）；任何既有断言不得放宽；若发现语义冲突立即停下回报，不得为过测试改行为。
- 2026-11-02 wave-slash-1 修订12（Lead 批准·批7 tmux 场景扩展）：
  (a) **S-PASTE** `paste-multiline` 场景：用 `tmux set-buffer` + `paste-buffer -p -t <pane>`（**bracketed paste**）注入 3 行含缩进/中文的文本；断言：①三行都进入 composer 且顺序正确；②**未自动提交**（无新增 assistant/tool 行、无 `Thinking`,③无 `\r`/`^M` 残留；④缩进与中文宽字符不被吞。
  (b) **S-RESIZE** `resize-invalidate` 场景：120×34 建立会话 → `tmux resize-pane -x 80 -y 24` → 再回 120×34；断言：①缩小时长行重排（行数/折行位置变化）；②恢复原始尺寸后画面与首帧**逐行一致**（证明 `invalidate_render_cache` 语义正确）；③全程 `no_panic` 双源干净、无残留半帧。
  (c) **S-ASK** `ask-panel` 场景（若 mock 可驱动）：驱动一次 Ask 交互，断言面板出现（问题文本/选项/自定义输入提示）、提交后转录出现回答且**面板关闭**、且 5s 内**不重开**（覆盖"旧 version 重开"风险的可见面）。若 mock 无法驱动 ask，则如实记为 `documented-gap` 并在 report 里注明所需 seam，**不得伪造场景**。
  (d) **S-CONTRAST**：新增 ANSI 实测对比度断言（parse `main.ansi` 的 SGR → 计算 WCAG 比值），对主题次要色（FAINT/GHOST/ACCENT_DIM on CANVAS/SURFACE）断言 **≥3.0**（当前实测 3.15/3.90/4.11/4.22 均应通过），并把实测值写进 `assertions.json` evidence；同时把 <4.5 的项在 report.md 里列为"AA 未达标（记录项）"，不阻断。
- 2026-11-02 wave-slash-1 修订13（Lead 批准·批8 Ask 闭环审计）：
  (a) **AUDIT-ASK 引导（仅 debug 构建）**：新增 `pub fn audit_ask_bootstrap_enabled() -> bool`（`cfg!(debug_assertions) && env PONY_AUDIT_ASK_BOOTSTRAP 存在`）；TUI 启动时若为真，则用**生产 API**（`control_plane.ask_dispatcher.dispatch_governed`，descriptor `builtin:ask`、`DispatchContext.session_id = state.session_id`）创建一条**真实** `PendingControlRequest`（kind=Interaction），使既有 500ms 轮询自然拉起 Ask 面板——**不得**新增合成 pending 队列或绕过 CAS 的后门；release（无 debug_assertions）编译期即不存在该路径。env 内容：`{"prompt":"…","options":[{"label":"…","description":"…"}]}`，解析失败按无 env 处理（fail-closed）。
  (b) **新审计场景 `ask-loop`**（真终端 tmux）：`PONY_AUDIT_ASK_BOOTSTRAP` 注入 → 面板出现（prompt/选项/esc 取消提示）→ Down 换焦点 → Enter 提交 → **面板关闭** → **5s 内不重开** → 状态区**无** CAS/版本/提交失败类文案（负向断言）。原 `ask-panel`（`--demo-ask-user`）保留为纯 UI 形态场景，`ask.documented_gap` 从 `ask-panel` 移除并改由 `ask-loop` 的真实断言覆盖。
  (c) **Rust 级闭环证据**（不依赖终端）：`dispatcher.dispatch_governed(builtin:ask)` → `list_pending_asks()` 恰 1 条且 kind=Interaction → `answer_ask(authorization)` 成功 → 再 `list_pending_asks()` 为空（CAS 消费生效）。
  (d) **fail-closed 证据**：无 env 时 `audit_ask_bootstrap_enabled() == false` 且不产生 pending ask；并静态断言引导代码位于 `cfg(debug_assertions)` 守卫内（release 不编译）。
