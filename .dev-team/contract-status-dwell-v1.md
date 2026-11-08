# 契约：状态行 min-dwell（方案1）+ 工具活动聚合（方案2）

Status: implemented（v1.1，2026-11-08；实现见 ADR 0096）

> v1 是「实现未落地」的草案（`aa08c41e`）。本版按**实际落地口径**修订：
> 新增 `StatusTone` 字段、明确「所有 Normal 候选同一条 1200ms 规则」、
> 明确折叠态文案与宽度降档顺序、明确唤醒走既有 `cap_poll_ms`。

问题根因：`ui()` 直接读 `status_word(state)` / `hidden_tool_status_text(state)` 组装，
每个 `UiEvent` 都即时覆盖 ⇒ 存活 < 一帧的工具状态必然被下一个事件盖掉；
工具结束后运行态回 `Responding`，状态行只剩「生成中」。
本契约把「显示什么」从「当前状态快照」改为「受 min-dwell 约束的可见文本 + 轮内聚合计数」。

## SD1 时间口径

- 一切时间用**单调毫秒** `now_ms: u64`（`OnceLock<Instant>` 起点，TUI 进程内唯一，`tui/status_dwell.rs::now_ms`）。
- 纯逻辑模块**不得**自行读时钟：`observe` / `record_call` 的 `now_ms` 由调用方注入，
  单测直接喂假时间。渲染层不推进计时（只读可见文本）。

## SD2 分级 min-dwell

| 候选性质 | 判定 | min-dwell |
|---|---|---|
| `Immediate` | `stopping_turn_id.is_some()` / `WaitingApproval` / `Completed` / `Failed` / `Ready`（含一次性提示） | 0ms（立即切换） |
| `Normal` | `Thinking` / `Responding` / `Connecting` / `ExecutingTool` | `STATUS_DWELL_MS = 1200` |

- **耗时秒数不参与候选文本**：由渲染层拼接，因此秒数刷新永不重置 dwell 计时。
- 相同文本重复 observe（包括同文本换了 tone）⇒ **不改计时**，只同步 tone。
- 密集候选不会缩短可见时长：可见文本必须满 `min_dwell` 才允许被 `Normal` 候选替换；
  切换后新文本从切换时刻重新起算。**pending 只保留最新候选**，不为被覆盖的候选补帧。

## SD3 模块 API（`crates/pony-agent-core/src/bin/tui/status_dwell.rs`）

```rust
pub const STATUS_DWELL_MS: u64 = 1200;

pub enum StatusUrgency { Normal, Immediate }
pub enum StatusTone { Muted, Faint, Warn, Success, Danger }

pub struct StatusCandidate { pub text: String, pub urgency: StatusUrgency, pub tone: StatusTone }

pub struct StatusDwell { /* private */ }
impl StatusDwell {
    pub fn new() -> Self;
    pub fn observe(&mut self, now_ms: u64, candidate: StatusCandidate);
    pub fn visible_text(&self) -> Option<&str>;
    pub fn visible_tone(&self) -> Option<StatusTone>;
    pub fn has_pending(&self) -> bool;
    pub fn next_wake_ms(&self) -> Option<u64>;
    pub fn reset(&mut self);
}

pub fn now_ms() -> u64;
```

`observe` 语义（按序判定，第一条命中即返回）：

1. `visible` 为 `None` ⇒ 立即显示，`started = now_ms`。
2. `candidate.text == visible.text` ⇒ 只同步 tone；若 pending 同文本则清 pending。
3. `candidate.urgency == Immediate` ⇒ 立即切换（覆盖任何 pending）。
4. `now_ms >= started + min_dwell` ⇒ 切换。
5. 其余 ⇒ `pending = Some(candidate)`。

`next_wake_ms`：存在 pending 时返回 `started + min_dwell`，否则 `None`。

## SD4 工具活动聚合（同模块，方案2）

```rust
pub struct ToolActivity { /* private */ }
impl ToolActivity {
    pub fn record_call(&mut self, label: &str);   // StreamToolCall 唯一写入点（计数 +1，置 current）
    pub fn complete(&mut self, label: &str);      // StreamToolResult：current → last，不清计数
    pub fn reset(&mut self);                      // 轮次起点 / 会话复位
    pub fn call_count(&self) -> u64;
    pub fn running(&self) -> Option<&str>;
    pub fn last(&self) -> Option<&str>;
    pub fn running_label(&self) -> Option<String>; // "执行 · {current}"
    pub fn summary(&self) -> Option<String>;
}
```

`summary` 文案（不截断；宽度由渲染层负责）：

- 1 次：`1 个工具 · {label}`
- ≥2 次：`{n} 个工具 · 最近 {label}`
- `label` 未知：`{n} 个工具`

## SD5 TUI 接入（`tui.rs`）

1. `TuiState` 增字段 `status_dwell: StatusDwell`、`tool_activity: ToolActivity`（`Default` 补齐）。
2. 唯一候选组装点：`status_line_candidate(state, budget) -> StatusCandidate`
   - 非折叠态：`status_word_base(state)` 的 `(文本, tone)`；
   - 折叠 + `ExecutingTool`：`tool_activity.running_label()`，回落 `hidden_tool_status_text`，再回落基础词；
   - 折叠 + `Thinking|Responding` 且有工具：宽度降档顺序
     `{词} · {summary}` → `{词} · {最近工具}` → `{词} · {N 个工具}` → `{词}`
     （**优先保住最近工具**，计数先丢）。
3. 唯一刷新点：`ui()` 内 `status_line_now(state, budget)`（喂候选 → 取可见文本 + tone），
   写入点（`StreamToolCall` → `record_call`、`StreamToolResult` → `complete`、
   `begin_turn_usage` → 双 reset、`reset_session_scope` / `clear_messages` / 终态 → reset）
   **不直接 observe**。
4. 语义色：`StatusTone` 经 `status_tone_style` 单点映射到 `theme`；`status_word_base` 不再产出 `Style`。
5. 唤醒：`status_dwell.next_wake_ms()` 并入 `cap_poll_ms`（与 goal 相位取最小，30ms 下限），
   并在画帧前置 `dirty`；不新增独立定时线程。

## SD6 验收

- `cargo test -p pony-agent-core --bin pony status_dwell`（9 条单测：首次显示 / 短工具态驻留 /
  pending 只留最新 / Immediate 抢占 / 同文本不重置 / tone 同步 / 终态 0ms 可替换 / 聚合文案 / reset）
- `cargo test -p pony-agent-core --test tui_process_fold_acceptance`（含
  `a15_folded_generation_keeps_tool_activity`：折叠态生成期携带 `1 个工具` / `2 个工具 · 最近 …`、
  窄屏降档、三态词互斥、工具行仍隐藏）
- `cargo check -p pony-agent-core --bin pony` 零 warning；`tui_acceptance` / `tui_render_layout` /
  `tui_dialog_wave39` / `tui_streaming_input_acceptance` 全绿。
- 可见时长：3 个连续工具调用（间隔 100ms）时，可见文本切换次数 ≤ 2，每次可见 ≥ 1200ms（单测覆盖）。

## 明确不在本次范围

- `add_or_update_tool_event`（`tui.rs`）在末条为 `ToolCall` 时无条件原位覆写 —— 并行工具互相覆盖
  是独立缺陷（事件不携带 tool id，需改上游载荷），本契约不动。
- 状态行 spinner 图标仍取自当前 `runtime_mode`，驻留窗口内（≤1200ms）可能与可见文本来自
  不同状态；两者同为 `ACCENT_DIM` 过程动画，尚未建「可见状态种类」字段。
