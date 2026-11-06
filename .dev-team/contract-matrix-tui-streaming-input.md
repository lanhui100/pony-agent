# 契约矩阵 · TUI 流式期间输入/面板/排队（tui-streaming-input）

> 真值源（Lead 冻结）。改判必须在此文件**原地改写**并登记单行变更流水。
> 文件：`crates/pony-agent-core/src/bin/tui.rs`（bin `pony`）。基线 commit：`adad39ad`（工作区含前序未提交增量，本轮不新增 git 提交，见 §0）。

## 0. 判据与偏差登记

- 缺陷真值：`tui.rs:9058` `if state.is_streaming && should_skip_while_streaming(&key) { continue; }`
  与 `tui.rs:611 should_skip_while_streaming` → 流式中除 Ctrl+U/J/W/A/E 外**全部按键被吞**；
  `tui.rs:9491` `InputPaste` 在 `is_streaming` 时直接 `continue`。
- 分级：**B 级**（状态机 + 事件循环 + 提交路径复用，涉及并发/spawn，命中 blast 关键词 `thread`/`spawn`）。
- 偏差登记（Lead 一票）：仓库工作区存在前序波次 40 个已暂存文件 + tui.rs 未暂存增量，
  本轮**不做任何 git 写操作**（不 add/commit/stash/reset），以避免污染他人未提交工作；
  物理收据以 `cargo check`/`cargo test` 原始退出码 + evidence 日志为准。

## 1. 契约条目（每条 = 一条可机械断言）

| ID | 断言（行为真值） | 断言载体 |
| :-- | :-- | :-- |
| **C1** | `is_streaming == true` 时，以下按键**必须**修改/移动输入框并生效：可打印 `Char(c)`、`Backspace`、`Delete`、`Left`、`Right`、`Home`、`End`、`Tab`（命令补全）、`Up`/`Down`（补全选择或滚动）、`Ctrl+U/J/W/A/E`。 | `streaming_key_disposition(&key, _) == Forward`（A1） |
| **C2** | `is_streaming == true` 且输入框 trim 后非空、且不是命令串 ⇒ `Enter` = **入队**（推入 `state.queued_user_messages`），清空输入框，状态提示「已排队 N 条 · 本轮结束后自动发送」。**严禁**在流式中直接 spawn 第二个并发 turn。 | `streaming_key_disposition(Enter, true) == QueueSubmit`（A2） |
| **C3** | `is_streaming == true` 时，面板/只读类命令 `Enter` 分发后**必须**照常执行：`/agents` `/tasks` `/plan` `/models` `/thinking` `/provider` `/help` `/update`；快捷键 `Ctrl+T`（任务板）、`Ctrl+R`（模型面板）必须生效。 | `is_panel_command(cmd) == true`；按键分支不再被流式守卫拦截 |
| **C4** | `is_streaming == true` 时，会话变更类命令 `/clear` `/compact` **`/resume`**（会换 session，中途切换会撕裂当前 turn）**必须拒绝**（不执行、清空输入框、给出「本轮结束后可用」提示）。`/quit` `/exit` 仍允许退出（既非面板亦非会话变更，走原退出分支）。 | `is_session_mutating_command(cmd) == true` |
| **C5** | 任一轮**终态**（`StreamCompleted` / `StreamFailed` / `StopTerminal`）后：若队列非空，**自动派发队首 1 条**（等价用户按 Enter 的完整提交路径：API key 校验 → `begin_turn_usage` → `add_user_message` → spawn turn thread），队列长度 −1。**单飞**：一次终态最多派发 1 条；派发采用「回灌合成 Enter 事件」或等价机制，不得同步重入。 | `drain_next_queued` 语义 + 事件回灌幂等 |
| **C6** | 队列容量上限 `MAX_QUEUED_MESSAGES = 20`；满则**拒绝**入队并提示，绝不静默丢弃；空串/纯空白**不入队**；命令串**不入队**（走 C3/C4 分发）。 | `push_queued_message` 返回值 |
| **C7** | 排队数量 ≥1 时用户**可见**：状态区/输入区显示「排队 N」。 | `queued_status_label(n)` |
| **C8** | 静态（`is_streaming == false`）行为**完全不变**：输入、Enter 提交、Esc 清空输入、`Ctrl+T`/`Ctrl+R`、斜杠命令整串分发口径、双击 Esc 停止、goal 续轮循环均不得回归。 | 既有 `tui_*` 测试集全绿 |
| **C9** | `InputPaste` 在流式中**必须**插入输入缓冲（与 C1 同口径）。 | 主循环 paste 分支去掉流式短路 |
| **C10** | 自动派发**闸门**（防并发 turn 撕裂会话历史）：仅当 `!is_streaming && mode == Normal && queue 非空 && 无在飞轮次线程 && !dispatch_pending` 时，循环一次最多派发 1 条；派发后置「闩」（**口径已由 C24 取代**：时间窗 + 提交成功即清 + 放弃回队；**禁止**用「任意 InputKey 清闩」）。**「无在飞轮次线程」= 轮次线程存活标志为 false**（不是「无 armed goal」）：续轮不置 `is_streaming=true`，只看 goal 存在会让「armed 但未跑」的会话排队永不自动派发。 | `should_drain_queued(is_streaming, ui_idle, queued, turn_in_flight, dispatch_pending) -> bool` |
| **C11** | 轮次线程 panic 兜底：`std::thread::spawn` 的轮次体用 `catch_unwind` 包住，panic 时补发 `UiEvent::StreamFailed`，杜绝 `is_streaming` 永真导致输入永久失效（现网无 catch_unwind，见侦察 :9426）。 | 代码 + `StreamFailed` 收据 |
| **C12** | **反馈必须可见**（L3-B B1）：流式期 C2「已排队 N 条」/ C4「本轮结束后可用」/ C6「队列已满」的反馈**不得**只走 `set_status_notice`（其渲染点被 `should_render_status_notice` 的 `!is_streaming` 抑制）。C4/C6 必须 `messages.push(ChatMessage{role:System})`；C2 允许 `status_text` + C7 指示器。 | 代码 + A11 |
| **C13** | **会话切换清队列**（L3-A#1 / L3-P#3）：`reset_session_scope`（`/clear`、`/resume`、hydrate 路径）必须清空 `queued_user_messages`，禁止旧会话排队消息被派发进新会话。 | `reset_session_scope` + A12 |
| **C14** | **派发正文净化**（L3-A#3/#4）：drain 写回输入框时，必须消除「行尾 `\` ⇒ 换行」与「命令前缀（如 `/hel`）⇒ 补全劫持」两条自救路径。**载体裁定（lead, 2026-11-06）**：净化由纯函数 `composer_buffer_for_dispatch(&str) -> String` 承载（返回「正文 + 一个尾部空格」），主循环 `set_input(&composer_buffer_for_dispatch(&msg))`；`take_next_queued_for_dispatch` 只负责取件，不写回。提交路径 `trim()`，正文内容不变。 | A13（断言 `composer_buffer_for_dispatch` + 真实 `TuiState` 语义） |
| **C15** | **单飞可测**（L3-B B2）：把「闸门判定 + 取件」收成一个生产符号 `take_next_queued_for_dispatch(...) -> Option<QueuedUserMessage>`，主循环**只能**经它派发；A7 必须直接断言该符号（禁止只测测试内自建闩）。 | 新符号 + A7 重写 |
| **C16** | **结构化日志**（L3-P Q2）：按键裁决（`QueueSubmit`/`Reject`）与派发各打一条结构化日志（沿用运行日志通道，含 turn/session 标识），不得零埋点。 | 代码 + ADVISORY 转 PASS |
| **C17** | **挂起期禁止派发**（L3-A#2）：`ui_idle` 必须同时要求「无待作答（awaiting answer）」——`TurnSuspended` 会置 `is_streaming=false` 而 run 未收口，此时派发会造成 run 交叠。 | 闸门第 2 参语义 + A14 |
| **C18** | 细节收口（ADVISORY 转正，低成本）：① `push_queued_message` 拒绝集补别名 `/exit` `/effort`；② `/help` 增加「流式期可输入 · Enter 排队」说明；③ 流式期不得同时在 `status_word` 与 `metrics_line` 重复渲染「排队 N 条」。 | A15 + 代码 |
| **C19** | **PlanMode 不得吞键**（计划门禁 B3）：`mode == PlanMode` 时仅 `Esc`/`Up`/`Down` 由面板消费，其余按键必须落回通用处理链（可输入/可提交/可排队）；派发闸门把 `PlanMode` 视为空闲（其余模态仍排除）。 | `plan_mode_consumes_key` + A16 |
| **C20** | **流式 Enter 优先级**（B5）：`Forward` 判定必须先于 `QueueSubmit`——① 输入以 `\` 结尾 ⇒ 换行；② `/` 前缀且存在补全候选且非完整命令 ⇒ 接受补全。禁止把命令前缀当聊天消息入队。 | A17 |
| **C21** | **合成事件去标记化**（B4 缓解）：派发改为投递独立 `UiEvent::QueuedAutoSubmit`；消费点再校验「非模态 ∧ 无 pending request ∧ 无 ask ∧ 无 permission」才回灌 `InputKey(Enter)`，否则不派发、正文留在输入框、清 `dispatch_pending`。禁止让无标记的合成 Enter 进入模态分支（防代用户审批/作答）。 | `should_reinject_auto_submit` + A18 |
| **C22** | **反馈通道可测**（防假绿 A11）：`queue_feedback(kind) -> (QueueFeedbackChannel, String)`，`SessionMutating`/`QueueFull` ⇒ `SystemMessage`，`Enqueued` ⇒ `StatusText`；主循环按 channel 决定 `messages.push` 还是 `status_text`。 | A11/A19 |
| **C23** | **排队/反馈必须有与运行态无关的渲染点**（L3-A 复审 BLOCKING）：`queued>0` 或存在 `queue_notice` 时，任何 `is_streaming` × `executing_tool` 组合都必须有可渲染行；C4/C6/C2 反馈写 `TuiState::queue_notice`（流式期不 push System，避免打断 `messages.last()` 气泡不变量），`clear_messages` 与 `reset_session_scope` 一并清空队列与 notice。 | `queue_indicator_line` + A20/A21 |
| **C24** | **单飞闩改为时间窗 + 成功即清 + 放弃回队**（L3-A round3 BLOCKING）：`queue_dispatch_pending: bool` 改为 `queue_dispatch_deadline: Option<Instant>`（活性 = `now < deadline`，窗口 2s）。① 任何 `InputKey` **不再**无条件清闩（旧行为会给「派发后、合成 Enter 被消费前」开单飞缺口）；② `state.is_streaming` 变 true（提交成功）时清闩；③ `QueuedAutoSubmit` 校验不通过时：把消息**回队队首** + 清空输入框 + 清闩，待下一次空闲重试（不得只留稿造成整个队列停摆）。 | A22 |
| **C25** | **派发/裁决日志字段补全**（NFR `logging.surfaces`）：`composer_queue_dispatch` 补 `queue_depth_before` / `queue_depth_after`；`composer_key_rejected` 补 `key_class`；入队事件补 `composer_has_text`。 | A23 |
| **C26** | **派发令牌与载荷绑定**（L3-A round4 BLOCKING）：派发时把消息快照存入局部 `queue_dispatch_payload`，`QueuedAutoSubmit` **消费点**先比对载荷与当前输入框（`queued_dispatch_diverged`）——分歧则 `requeue_front` 回队原 payload、清闩、保留用户当前输入、`continue`；且 QAS 分支**末尾必须 `continue`**（禁止同轮回落闸门造成 1 终态 2 派发）。Esc/Ctrl+U 撤销路径回队**权威 payload**（非 `input_buffer`）。 | `queued_dispatch_diverged` + A25/A26 |
| **C27** | **孤儿载荷收割**（L3-A round5 BLOCKING）：`queue_dispatch_payload`（单槽）与时间窗闩生命周期不同步——闩过期后闸门可能把「仍有合成事件在飞」当空闲并覆盖 payload，导致上一条在 `remove(0)` 后既未提交也未回队。闸门在取件**之前**必须收割：`orphan_dispatch_payload(payload, latch_active)` 为 `Some` ⇒ 回队 + 清输入框 + 清 payload + `continue`（此路径闩必已过期，无需再置 `deadline=None`）。**已知取舍**：收割时无条件 `clear_input()` 会连带清掉该窗口内用户刚键入的草稿（派发正文不丢，草稿会丢）——登记为已知残余，修复需引入「草稿/派发快照」双槽字段。 | `orphan_dispatch_payload` + A27 |

## 2. 契约冻结的空桩符号（Protocol Agent 唯一写主，Test Agent 只读引用）

> 裁决修订（lead, 2026-11-06）：采用 Protocol Agent 的单函数收敛口径 `streaming_key_disposition`
> 作为 **C1–C4 唯一裁决面**（少一个 API 面 = 少一处漂移），并保留命令分类与队列原语。原
> `streaming_input_allowed` / `composer_enter_intent` **作废**（不再实现、不再断言）。

```rust
pub const MAX_QUEUED_MESSAGES: usize = 20;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingKeyDisposition { Forward, QueueSubmit, Reject }
pub type QueuedUserMessage = String;
/// C1–C4 唯一裁决面：键 × 是否有输入 → 透传 / 入队提交 / 拒绝
pub fn streaming_key_disposition(key: &crossterm::event::KeyEvent, composer_has_text: bool) -> StreamingKeyDisposition;
pub fn is_panel_command(command: &str) -> bool;
pub fn is_session_mutating_command(command: &str) -> bool;
pub fn push_queued_message(queue: &mut Vec<String>, text: &str) -> bool;
pub fn drain_next_queued(state: &mut TuiState) -> Option<QueuedUserMessage>;
pub fn queued_status_label(count: usize) -> Option<String>;
pub fn should_drain_queued(is_streaming: bool, ui_idle: bool, queued: usize, turn_in_flight: bool, dispatch_pending: bool) -> bool;
/// C15：生产唯一取件点（主循环只经它派发；A7 直接断言本符号）
pub fn take_next_queued_for_dispatch(state: &mut TuiState, is_streaming: bool, ui_idle: bool, turn_in_flight: bool, dispatch_pending: bool) -> Option<QueuedUserMessage>;
/// C14：派发写回净化（正文 + 尾部空格；主循环必须经它写回输入框）
pub fn composer_buffer_for_dispatch(message: &str) -> String;
pub fn queued_count(&self) -> usize;   // TuiState 只读访问器
// TuiState 新增字段：pub queued_user_messages: Vec<String>
```

- 既有 `should_skip_while_streaming` / `ComposerControlAction` **保留**（不删公共符号），但主循环改由 `streaming_key_disposition` 裁决。
- 既有测试 `crates/pony-agent-core/tests/tui_six_fixes_acceptance.rs:116-125` 断言「流式中 `Char('x')` 被跳过」——该断言属**旧契约**，本轮由 Test Agent 按 C1 迁移改写（Lead 已裁决，属契约演进，非测试放水）。

## 3. NFR（`.dev-team/nfr-baseline-tui-streaming-input.json` 为准）

- `queue_drain_single_flight`：一次终态最多 1 条派发；不得批量连发造成 turn 并发。
- `lock_scope_bounded`：持 `current_turn_id`/`inflight_turns` 锁期间**不得**调用 `control_plane`。
- `no_busy_wait_ms`：不新增忙等；沿用 `rx.recv_timeout` 16/120ms 轮询。
- `max_queue_len = 20`；`logging.format = json` 且含 trace_id（沿用现有 TUI 运行日志通道）。

## 4. 验收测试清单（Test Agent 冻结，Executor 只读）

| 用例 | 类型 | 断言 |
| :-- | :-- | :-- |
| A1 | L2-T | 流式中 Char/Backspace/Delete/Left/Right/Home/End/Tab 全部生效（`streaming_input_allowed`） |
| A2 | L2-T | 流式中 `composer_enter_intent(true, "hi") == Queue`；空串 → `Reject`；命令串 → `Reject` |
| A3 | L2-T | 面板命令/会话变更命令分类（C3/C4 全表逐项） |
| A4 | L2-T | `push_queued_message` 空串/空白/命令串拒绝；达 20 上限拒绝；`drain_next_queued` FIFO 且一次性 |
| A5 | L2-T | `queued_status_label(0) == None`，`(1)`/`(3)` 文案含数量 |
| A6 | L2-T | 静态口径无回归：`composer_enter_intent(false, "hi") == Send`；既有 `should_skip_while_streaming` 旧断言按 C1 迁移 |
| A7 | L2-AT | 终态连续（Completed → StopTerminal → Failed）不得双发：模拟队列 3 条 + 连续终态，派发计数 ≤ 终态数且每次 ≤1 |
| A8 | L2-AT | 队列已达上限时高并发重复 push 不 panic、不越界、不变负；drain 后 count 单调 |
| A9 | L2-T | `should_drain_queued` 六组真值矩阵（5 拒 1 允）+ `queued==1` 边界（C10） |
| A10 | L2-AT | 满队列 40 次 push 恒 false；容量恢复后未满坏载荷恒 false（C6） |
| A11 | L2-T | `queue_feedback` 通道：`SessionMutating`/`QueueFull` ⇒ `SystemMessage`，`Enqueued` ⇒ `StatusText`；满队列文案含上限（C12/C22） |
| A12 | L2-T | `reset_session_scope` 清空排队（C13） |
| A13 | L2-T | `composer_buffer_for_dispatch`：尾部 `\`/`/hel` 前缀不再触发换行或补全劫持，正文 trim 后不变（C14） |
| A14 | L2-T | `take_next_queued_for_dispatch(ui_idle=false)` ⇒ None（C15/C17） |
| A15 | L2-T | `push_queued_message` 拒绝 `/exit` `/effort` 别名；上限语义不变（C18） |
| A16 | L2-T | `plan_mode_consumes_key`：Esc/Up/Down ⇒ true，其余键 ⇒ false（C19） |
| A17 | L2-T | 流式期 Enter 优先级：行尾 `\` ⇒ 换行、命令前缀 ⇒ 补全，均优先于入队（C20） |
| A18 | L2-T | `should_reinject_auto_submit`：仅四条件全 false 才回灌合成 Enter（C21） |
| A19 | L2-T | `queue_feedback(QueueFull)` 文案与 `MAX_QUEUED_MESSAGES` 同源（C22） |
| A20 | L2-T | `queue_indicator_line`：`queued=2` 在 `is_streaming`×`executing_tool` 四组合全 `Some`；`0/None` ⇒ None；notice 与计数并存（C23） |
| A21 | L2-T | `clear_messages` 清空排队（C13 第二入口） |
| A22 | L2-T | 派发闩时间窗：`should_reinject_auto_submit=false` 时消息回队队首且输入框清空（不得停摆）；闩过期后可重试（C24） |
| A23 | L2-T | 派发日志字段完备性：`tui_queue_log` 调用的 extras 必含 `queue_depth_before`/`queue_depth_after`（可用纯函数 `queue_dispatch_fields(before, after)` 承载并断言）（C25） |
| A25 | L2-T | `queued_dispatch_diverged` 真值矩阵（None / trim 相等 / 抹空 / 追加） |
| A26 | L2-AT | trace-B：Ctrl+W 抹空后载荷回队、顺序不变、无丢无重复、不停摆（C26） |
| A27 | L2-T | `orphan_dispatch_payload` 真值矩阵（闩过期+有载荷 ⇒ 收割；闩活跃 ⇒ 不收割；空白不收割；trim）（C27） |

## 5. 变更流水

- 2026-11-06 lead：初版冻结（C1–C9 + 空桩符号 + A1–A8）。
- 2026-11-06 lead：裁决修订——采纳 `streaming_key_disposition` 单裁决面；`streaming_input_allowed`/`composer_enter_intent`/`ComposerSubmitIntent` 作废（§2 已移除，§1 C1/C2 载体改判）。
- 2026-11-06 lead：C10 判据由 `goal_active` 改为「轮次线程存活」（`turn_in_flight`）；新增 C11（panic 兜底）。
- 2026-11-06 lead：**L3 审查回合**新增 C12–C18（反馈可见 / 清队列 / 写回净化 / 单飞可测 / 日志 / 挂起期 / 细节），C14 载体裁定为 `composer_buffer_for_dispatch`，C15 新增 `take_next_queued_for_dispatch`，用例扩到 A9–A15。
- 2026-11-06 lead：**计划门禁回合**新增 C19–C22（PlanMode 吞键 / Enter 优先级 / 合成事件去标记 / 反馈通道可测），用例扩到 A16–A19。
- 2026-11-06 lead：**L3-A 复审回合**新增 C23（运行态无关的排队/反馈渲染点 + queue_notice 口径 + clear_messages 清队列），用例扩到 A20–A21。
- 2026-11-06 lead：**L3-A 终审回合**新增 C24（单飞闩时间窗/成功即清/放弃回队）与 C25（日志字段补全），用例扩到 A22–A23。
- 2026-11-06 lead：**L3-A round4 回合**新增 C26（派发令牌与载荷绑定 + QAS 分支强制 continue），用例扩到 A25–A26。
- 2026-11-06 lead：**L3-A round5 回合**新增 C27（孤儿载荷收割，闭合「闩过期 ⇒ payload 被覆盖 ⇒ 静默丢正文」），用例扩到 A27。C5 与 C24 的张力（同一终态可能触发 2 次派发尝试但仅 1 次交付）登记为已知语义，不再迭代。
