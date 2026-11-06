# 契约矩阵 · TUI 思考消息缺失 + 消息区底部与输入框间距（thinking-bottom-gap）

> 真值源（Lead 冻结）。改判必须在此文件**原地改写**并登记单行变更流水。
> 文件：`crates/pony-agent-core/src/bin/tui.rs`（bin `pony`）。分级：**A 级**（单模块 UI 修复，无并发/网络/IO）。
> 验收测试（唯一）：`crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs`（红相冻结后只读）。

## 0. 判据与偏差登记

- **缺陷 1（思考消息缺失）真值**：`tui.rs:4438` `"turn:delta"` 分支只读 `payload.text`；
  `TurnStreamEvent.reasoning_content`（`runtime/types.rs:165`）在 TUI 侧**全链路丢弃**——
  无 `UiEvent` 变体、无 `MessageRole`、无渲染分支。上游确实在发：`runtime/turn_stream.rs:913`
  `emit_stream_event_with_step(..., Some(reasoning_content), ...)`。故用户看到「消息列表里没有思考消息」。
- **缺陷 2（底部间距）真值**：`ui()` 中 `let chat_h = bottom_top.saturating_sub(chat_top)`（`tui.rs:6479`）
  ⇒ `chat_rect` 下缘 = Composer 色带上缘，滚到底时最后一行（正文或流式状态行）**零间距**贴住输入带。
- 偏差登记（Lead 一票）：仓库工作区存在**他人前序未提交改动**（`tests/tui_terminal_restore.rs`、
  `docs/decisions/README.md`、`docs/decisions/0075-*.md`）。本轮 git 写操作仅允许
  **定向路径**暂存（`git add -- <本轮两文件>`），严禁 `git add -A/.`、`git stash`、`reset --hard`、`clean`。
- 观测性动线：**不适用**（纯终端渲染 + 内存事件分派；无对外接口/网络 IO/后台任务/LLM 调用新增）。
  依据：`docs/observability.yaml` 的 metrics/logs 结构性 N-A 理由已覆盖该交付形态，本轮不新增收口点。

## 1. 契约条目（每条 = 一条可机械断言）

| ID | 断言（行为真值） | 断言载体 |
| :-- | :-- | :-- |
| **C1** | `turn:delta` 载荷的 `reasoning_content` **必须**进入 UI：新增 `UiEvent::StreamReasoning(String)`，事件循环调用 `TuiState::apply_stream_reasoning`。取载口径唯一化到 `stream_delta_ui_payload(&TurnStreamEvent) -> DeltaUiPayload { text, reasoning }`；`TuiTurnSink` 只允许经它取载（禁止在 sink 里二次解析字段）。 | A1/A2（`stream_delta_ui_payload`）+ 代码审阅：sink 分支只调用该符号 |
| **C2** | 载荷双通道各自独立：`text` 与 `reasoning` 可同时非空；两者都为空 ⇒ `DeltaUiPayload::default()`（`None/None`）。空载**不得**产生任何 UI 事件。 | A1/A2 |
| **C3** | `text` 通道沿用现有 DSML 净化 `sanitize_model_protocol_text`：净化后为空**串** ⇒ `text = None`（不投递空正文事件）。**修订 R3（2026-11-06，L3-A BLOCKING）**：判据必须是 `cleaned.is_empty()`，**不得**用 `trim()`——纯空白正文增量（如 `"\n\n"`）是段落/换行载体，丢弃即回归（HEAD 口径 `!visible.is_empty()`）。`reasoning` 通道**原样**透传（不做 DSML 净化；思考内容不是工具协议通道），仅 `trim().is_empty()` 时置 `None`。 | A2/A13 |
| **C4** | 思考入库口径：`append_reasoning_chunk`——末条消息 role == `Thinking` 时**原地续写**；否则**新开**一条 `Thinking` 消息；`chunk.trim()` 为空 ⇒ 不改变 `messages`。正文/工具/系统消息**不得**被思考增量污染（`append_assistant_chunk` 只在末条为 `Assistant` 时续写，保持既有不变量）。 | A3/A4 |
| **C4a** | **思考按轮分块**（修订 R4，2026-11-06，L3-A ADVISORY 转修）：宿主续轮（Ask 恢复 / 审批恢复 / goal 续轮）不经过 `add_user_message`，故 `turn:started` 的公共复位点 `TuiState::begin_turn_usage()` 必须同时关闭当前思考块（`reasoning_turn_closed = true`）⇒ 下一轮首个思考增量**新起** `Thinking` 消息，禁止续写进上一轮块；用户手动提交轮不受影响（`add_user_message` 先于 turn 开始）。 | A14 |
| **C5** | 运行态：收到思考增量时，若当前不是 `RuntimeMode::ExecutingTool(_)` ⇒ `runtime_mode = Thinking`（工具执行中不得抢状态）。`is_streaming`、`scroll_offset`、`auto_scroll` 不被改写。 | A5 |
| **C6** | 渲染：`MessageRole::Thinking` 必须在消息列表**可见**，且视觉从属于正文——首行前缀 `✻`（`theme::ACCENT_DIM`），正文 `theme::FAINT` + `Modifier::ITALIC`，续行用 RAIL（2 列）缩进，按 `width - RAIL` 折行（复用 `push_prefixed`）。**间距（修订 R1，2026-11-06）**：思考块与**轮内**相邻消息（Assistant / ToolCall / ToolResult / System）之间恰好 1 行呼吸——前驱侧不得为 0（粘连），后驱侧不得为 2；与 `User` 的边界沿用既有轮间距（User 块自带内边距，前后各 1 行 ⇒ 视觉 2 行），不得由思考块额外叠加。 | A6/A7/A12（TestBackend 真实渲染断言） |
| **C7** | 正文接续：`Thinking` 之后到达的正文仍**新起** `Assistant` 消息（不得追加进思考块）；渲染上思考块与随后正文之间 1 行呼吸。 | A4/A7 |
| **C8** | 底部间距：消息区下缘与 Composer 带之间**恒定保留 `CHAT_BOTTOM_GAP` 行空白**（CANVAS 底色、无任何字形）。**修订 R5（2026-11-06 用户实测）**：`CHAT_BOTTOM_GAP = 2`（1 行偏挤，用户要求 2 行）；自动贴底（`auto_scroll=true`）与手动滚到底（`auto_scroll=false` + max offset）**两种情形都成立**；这 2 行空白即最后一条内容行之后的前两行。 | A8/A9 |
| **C9** | 间距不得靠"内容不够长"碰巧成立：消息总量必须溢出视口（`total_lines > visible`），贴底后最后一条消息仍完整可见（`state.scroll_offset == state.max_scroll_offset`），且间距行空白。 | A8 |
| **C10** | 流式状态行（`is_streaming` 时的「思考中/生成中 · 秒数」行）是内容区最后一行，其下同样保留 `CHAT_BOTTOM_GAP` 行空白（用户诉求的「状态消息与输入框保持距离」）。 | A9 |
| **C11** | 极小终端（24×10 边界）下 `saturating` 语义：不得 panic、不得把间距行挤进 Composer；空输入时 `last_chat_rect` 下缘 ≤ `prompt_row - (1 + CHAT_BOTTOM_GAP)` = `prompt_row - 3`。**修订 R2（2026-11-06）**：当 Composer 多行输入把消息区可用高度压到 ≤1 行时，间距行让位（best-effort，`chat_h.max(1)`），此为已登记边界，不得 panic/越界绘制。 | A10 |
| **C12** | 回归不变：既有 TUI 全量测试（含 `tui_render_layout` 的 composer 几何断言、指标行、发丝线）保持全绿；`ChatMessage`/`MessageRole` 新增变体不得改变既有 5 类角色渲染。 | 全量 `cargo test --test 'tui_*'` 绿 + A11 |

## 2. 契约冻结的空桩符号（Lead 唯一写主，Executor 只填写实现，Test Agent 只读引用）

```rust
// MessageRole 新增变体（思考块）
pub enum MessageRole { User, Assistant, ToolCall, ToolResult, System, Thinking }

// 新增 UI 事件（reasoning 增量）
pub enum UiEvent { /* … */ StreamReasoning(String) }

// 思考取载唯一面：把 turn:delta 载荷拆成「正文 / 思考」两条独立通道
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeltaUiPayload {
    pub text: Option<String>,
    pub reasoning: Option<String>,
}
pub fn stream_delta_ui_payload(payload: &TurnStreamEvent) -> DeltaUiPayload;

impl TuiState {
    /// 思考增量入库（末条 Thinking 续写，否则新开）+ 运行态推进（非工具执行 ⇒ Thinking）。
    pub fn apply_stream_reasoning(&mut self, chunk: &str);
}

// 消息区下缘与 Composer 带的固定呼吸行数
pub const CHAT_BOTTOM_GAP: u16 = 1;
```

## 3. 依赖与写域

| 任务 | owner | write_scopes | blocked_by |
| :-- | :-- | :-- | :-- |
| S1 空桩冻结（Lead） | lead | `crates/pony-agent-core/src/bin/tui.rs` | — |
| S2 验收测试红相（Test Agent） | test-agent | `crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs` | S1 |
| S3 实现（Executor） | executor | `crates/pony-agent-core/src/bin/tui.rs` | S2（红相冻结后） |

## 4. 变更流水

- 2026-11-06 Lead 初版冻结（A 级 / 2 缺陷 / C1–C12）。
- 2026-11-06 22:43 红相冻结：`cargo test --test tui_thinking_bottom_gap` exit 101（15 passed / 10 failed），
  收据 `.dev-team/red-phase-thinking-bottom-gap.md`。
- 2026-11-06 23:05 绿相：同一命令 exit 0（27 passed，含 tui.rs 内既有单测）；实现偏离已登记
  （C7 呼吸落思考块尾部；`scroll_total` 剔尾部连续空白行）——收据 `.dev-team/green-phase-thinking-bottom-gap.md`。
- 2026-11-06 23:17 Lead 独立复核：验收 exit 0（30 passed）；全量 TUI（排除 `tui_terminal_restore`）
  仅 3 项红：`o1`（HOME 进程级竞态，单跑复绿）、`c9`/`j1`（PTY 启动期挂起，回退本轮改动后症状逐字相同 ⇒ 外部并行会话在飞状态）。
- 2026-11-06 23:20 文档：ADR 0076 + `docs/decisions/README.md` 索引 + `.agents/notes` 决策记录
  （`verify-note.sh` exit 0）。
- 2026-11-06 23:55 L3-A 审查（`.dev-team/review-l3-thinking-gap-a.json`）判 FAIL：BLOCKING = `text` 通道误用 `trim` 吞掉纯空白增量（段落回归）；ADVISORY = 状态行前呼吸 no-op（既有死码，登记）、续轮无思考块边界（→ 已转修 C4a）、reasoning 零净化（C3 冻结口径，登记）、R4 判**不可达**（`turn:delta` 全在 provider 回调内）。契约修订 C3/C4a，新增 A13/A14。
- 2026-11-06 23:40 L3-B 审查（`.dev-team/review-l3-thinking-gap-b.json`）判 FAIL：1 BLOCKING（C6 前驱侧呼吸为死代码：
  `push_blank` 在空块上 no-op ⇒ Assistant/ToolResult→Thinking 0 行）+ 4 ADVISORY。Lead 裁决：C6 原地修订为
  「轮内 1 行 / 跨轮沿用 2 行」，新增 A12 锁死前驱侧间距；F4（`show_banner` 未补 `CHAT_BOTTOM_GAP`，height=31 品牌画消失）
  一并修复；F2（`scroll_total` 剔尾随空白）与 F3（极小终端让位）登记为**已裁决的契约修订**；F5（Thinking 打散
  「末条即运行中 ToolCall」判据）登记为待观测残余（当前 runtime 顺序不可达）。

- 2026-11-06 23:45 Lead 终局复核（独立于执行者收据）：
  - 验收 `cargo test --test tui_thinking_bottom_gap` **exit 0 / 33 passed**（A1–A14）；`cargo check --bin pony` exit 0。
  - 间距探针（临时 test，跑完已删）：User→Thinking=1、Assistant→Thinking=1、ToolResult→Thinking=1、
    Thinking→Assistant=1、Thinking→User=2（跨轮既定节奏）、User→Tool→Thinking→Assistant 三段均 1。
  - 全量 TUI（no-fail-fast，排除 `tui_terminal_restore`）3 个红目标，全部经隔离复跑/回退实验判定为外部：
    ① `tui_acceptance::test_resolve_active_provider_id_reads_default_registry`（单跑绿）与
    ② `tui_dialog_wave39::a4_metrics_row_left_shows_workspace_label`（单跑绿）——同因：并行会话新增的
    `tui.rs::wave65_approval_unit_tests::isolate_process_data_dir()`（tui.rs:12102）用 `set_var("HOME")`
    改进程级环境，`#[path] mod tui` 使该用例在每个测试二进制内运行 ⇒ HOME 竞态（既有测试基建债）；
    ③ `tui_overlay_cancel_acceptance::c9/j1`（PTY 启动期主线程卡互斥量，回退本轮全部改动后症状逐字相同）。
  - 结论：本波次验收面全绿；3 项红均不具备与本改动的因果链（含回退对照实验）。
- 2026-11-06 23:55 用户实测反馈：1 行间距偏挤 ⇒ C8 原地修订为 `CHAT_BOTTOM_GAP = 2`（C11 判据同步 `prompt_row - 3`），A8/A9/A10 断言随动；实现只允许改常量值，几何推导不得变形。
- 2026-11-06 23:55 **用户实测改判**：底部间距 1 行偏挤 ⇒ C8 修订为 `CHAT_BOTTOM_GAP = 2`（A8/A9/A10 随动）。
  Lead 复核：`cargo test --test tui_thinking_bottom_gap` **exit 0 / 35 passed**；`cargo check --bin pony` exit 0；
  全量 TUI（no-fail-fast）仅 2 个红目标且均为外部：`tui_dialog_wave39::a4`（HOME 竞态，单跑 **exit 0 / 34 passed**）、
  `tui_overlay_cancel_acceptance::c9/j1`（PTY 启动挂起，已由回退对照实验定性）。24×10 最小尺寸下
  `chat_h = 1`（间距行 2 行照常保留），属 C11 修订 R2 的 best-effort 边界，无 panic/无降级提示。
