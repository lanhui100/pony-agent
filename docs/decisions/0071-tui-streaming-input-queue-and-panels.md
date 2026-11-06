# ADR 0071：TUI 流式期可输入、面板可开、用户消息排队

Status: implemented

## 背景

用户实测 `pony` TUI 时报告：**Agent 运行（流式生成）期间输入框完全无法使用**——打不出字、按 Enter 无反应、`/agents` `/models` `/thinking` 打不开、`Ctrl+T` 任务板与 `Ctrl+R` 模型面板也无效。表现上像是「TUI 死掉了」，只能等本轮结束。

根因（机器事实）：主循环里

```rust
if state.is_streaming && should_skip_while_streaming(&key) { continue; }
```

`should_skip_while_streaming` 只对 `Ctrl+U/J/W/A/E` 放行，**其余按键（可打印字符、编辑键、Enter、全部面板快捷键与斜杠命令入口）在流式期被静默丢弃**；`UiEvent::InputPaste` 分支另有一处 `is_streaming ⇒ continue` 的同源早退。

该守卫的初衷是对的：**同一 session 不允许并发 turn**。但「不许提交第二轮」不等于「不许输入」，把「会话控制」与「输入框编辑」混成一个开关，才产生了这个缺陷。

## 决策

1. **按键裁决收敛为单一面**：`streaming_key_disposition(&KeyEvent, composer_has_text) -> StreamingKeyDisposition`，返回 `Forward | QueueSubmit | Reject`。
   - 编辑/光标/面板类键 ⇒ `Forward`（流式期照常走原处理链）；
   - 流式期 Enter 且输入非空 ⇒ `QueueSubmit`；空输入 ⇒ `Reject`；
   - 面板/模态分支、双击 Esc 停止、`Ctrl+C` 三态、F2/F3 **全部先于**该裁决生效（模态内按键不被抢）。
   - 保留 `should_skip_while_streaming` 符号（不删公开 API），但主循环不再调用。
2. **Enter 在流式期 = 入队**（`TuiState::queued_user_messages`，上限 `MAX_QUEUED_MESSAGES = 20`，FIFO，拒绝空串/命令串/别名，满队列拒绝且**保留正文**）。命令串不排队：面板/只读类（`/agents /tasks /plan /models /thinking /provider /help /update`）照常执行；会话变更类（`/clear /compact /resume`）拒绝并给出可见反馈（消息区 System 消息，不走流式期被抑制的 `status_notice`）。
3. **终态后单飞派发**：`take_next_queued_for_dispatch(state, is_streaming, ui_idle, turn_in_flight, dispatch_pending)` 是**唯一取件点**（单飞闩为**时间窗**语义：`QUEUE_DISPATCH_LATCH_MS = 2000` + `queue_dispatch_latch_active`；提交成功、提交失败、以及「放弃回灌」三条路径各自清闩；**禁止**用「任意按键清闩」——那会在派发与自动提交之间开一个二次派发窗口。放弃回灌时消息用 `requeue_front` 回队首并清空输入框，绝不静默丢正文），闸门 = 非流式 ∧ UI 空闲（Normal 且无待作答/审批）∧ 队列非空 ∧ 无在飞轮次线程 ∧ 未处在派发中。取件后写回输入框（`composer_buffer_for_dispatch`：正文 + 尾部空格，绕开「行尾 `\` ⇒ 换行」与「`/hel` 前缀 ⇒ 补全劫持」），再向同一 `tx` 回灌一条合成 Enter，复用既有唯一提交路径——**不内联复用提交逻辑，不同步重入**。
4. **in-flight 判据用「轮次线程存活」而非「有无 goal」**：`TurnLoopInFlightGuard`（RAII，Drop 复位）覆盖首轮 + goal 续轮 + panic 展开；否则「armed 但未跑」的会话会让队列永不派发。
5. **panic 兜底**：轮次线程体整体 `catch_unwind`，panic 时补发 `UiEvent::StreamFailed`，杜绝 `is_streaming` 永真把输入永久锁死。
6. **会话切换清队列**：`reset_session_scope` 清空排队，防止旧会话消息被派发进新会话。
7. **可观测**：裁决（`composer_queue_submit` / `composer_queue_rejected` / `composer_key_rejected`）与派发（`composer_queue_dispatch`）各一条结构化日志，复用既有 `otel::log` JSON 通道（含 session/turn）。

## 备选方案与取舍

- **流式期直接开第二个 turn**：最省事，但 `start_turn_stream` 无 session 级在飞守卫，两轮会按完成序互插历史、trace 序号可撞号 ⇒ **否决**。
- **把提交逻辑抽成函数供排队路径调用**：可行但改动面更大、易与 goal 续轮状态耦合；「写回 + 回灌合成 Enter」让提交仍只有一条路径，改动更小且可审计 ⇒ 采纳。
- **只放开输入、Enter 仍禁用**：用户能打字但发不出去，仍要在本轮结束后手动重发 ⇒ 不满足「插入/排队新消息」诉求。

## 已知局限（显式登记，非缺陷粉饰）

1. `TurnSuspended` 只置 `is_streaming=false`，仓内无「挂起 run」全域标记；若挂起瞬间恰好无 pending ask，闸门可能判空闲而派发。彻底闭环需新增 `suspended_run` 状态标记（登记 backlog，本轮未做）。
2. **（已缓解）** 合成 Enter 曾与真实按键同通道，理论上可被 Ask/审批模态当作「作答/放行」消费。现改为独立 `UiEvent::QueuedAutoSubmit`：消费点再校验「非模态 ∧ 无 pending request ∧ 无 ask ∧ 无 permission」才回灌 `InputKey(Enter)`，否则不派发、正文留在输入框。残余窗口已从「任意模态态」收窄到「校验后到下一事件之间」，且要求此刻无在飞轮次（无 turn 即无新权限请求来源）。
3. 流式行的「已排队 N 条」以 C7 指示器呈现；逐字回执（含「本轮结束后自动发送」）在活跃流式行不保证逐字出现。
4. **孤儿载荷收割窗口内的草稿**：当闩（2s）过期且仍有合成事件在飞时，闸门会先把「在飞载荷」回队（正文不丢），该路径无条件清空输入框——若用户恰好在这个 ≥2s 停顿窗口里键入草稿，草稿会被吞掉。彻底闭合需引入「草稿 / 派发快照」双槽字段（登记 backlog，未做）。
5. 队列日志的 `trace_id` 在事件循环线程恒为 `null`（turn 登记发生在轮次线程），事件名/session/turn 字段仍可用于关联。

## 复审追加决策（同日）

- **PlanMode 不再是「吞键输入态」**：`mode == PlanMode` 时只有 `Esc`/`Up`/`Down` 由面板消费（`plan_mode_consumes_key`），其余按键落回通用处理链——否则计划面板一开，输入框整段失效（与本次缺陷同症状）。
- **流式 Enter 优先级**：`streaming_enter_pre_action` 在入队判定之前先仲裁「行尾 `\` = 换行」「命令前缀 = 接受补全」，避免把 `/he` 这类前缀当聊天消息排队。
- **反馈必须有与运行态无关的渲染点**：`queue_indicator_line(queued, notice, is_streaming, executing_tool)` 两个布尔不参与判定，接在每帧渲染的指标行；`TuiState::queue_notice` 承载 C2/C4/C6 的短时提示（流式期不再往消息区插 System 行，避免打断流式气泡的 `messages.last()` 不变量）。
- **派发/裁决日志补字段**：`queue_depth_before` / `queue_depth_after` / `key_class`（不记正文）/ `composer_has_text`。

## 验收（机器收据）

- `cargo check --bin pony` ⇒ EXIT 0（0 warning）
- `tui_streaming_input_acceptance`(21) / `tui_streaming_input_adversarial`(15) / `tui_six_fixes_acceptance`(18) ⇒ EXIT 0
- 既有 TUI 回归五套 ⇒ 18/18/18/18/12 passed
- 变异敏感度：6/6 轮全红，11 个冻结用例全部至少被 1 个变异杀死
- 最终轮：`tui_streaming_input_acceptance`(30) / `tui_streaming_input_adversarial`(15) / `tui_six_fixes_acceptance`(18) ⇒ EXIT 0；回归五套 18/18/18/18/12；脚本生成/迁移的用例 A1–A24 全绿，独立测试者复跑一致（`tui.rs` sha256 `a024ee72…`）
- **PTY 实机**：本地假 SSE 流 9s 窗口内，流式中输入 `hello` 与「生成中 2秒」同帧并存；流式中 `Ctrl+T` 打开「Team 共享任务看板」，Esc 关板后仍在流式态
- 收据目录：`.dev-team/evidence/`（`impl-executor.log`、`red-phase-*.log`、`mutation-sensitivity.log`、`pty-streaming-smoke.log`）
