# 契约：状态行 min-dwell（方案1）+ 工具活动聚合（方案2）

状态行现状：`ui()` 直接读 `status_word(state)` / `hidden_tool_status_text(state)` 组装，
每个 `UiEvent` 都把 `dirty` 置真 ⇒ 存活 < 一帧（16ms）的工具状态必然被下一个事件覆盖。
本契约把「显示什么」从「当前状态快照」改为「受 min-dwell 约束的可见文本 + 轮内聚合计数」。

## SD1 时间口径

- 一切时间用**单调毫秒** `now_ms: u64`（`Instant` 起点 + `OnceLock`，TUI 进程内唯一）。
- 纯逻辑模块**不得**自行读时钟：`observe` / `record_call` 的 `now_ms` 由调用方注入，
  单测可直接喂假时间。渲染层不推进计时（只读可见文本）。

## SD2 分级 min-dwell

| 候选性质 | 判定 | min-dwell |
|---|---|---|
| `Immediate` | `stopping_turn_id.is_some()` / `WaitingApproval` / `Failed` / `Completed` / `Ready` 的一次性提示 | 0ms（立即切换） |
| `Normal` | `Thinking` / `Responding` / `Connecting` / `ExecutingTool` | `DWELL_TOOL_MS = 1200` |

- **耗时秒数不参与候选文本**：由渲染层拼接，因此秒数刷新永不重置 dwell 计时（肉眼可见的实时感保留）。
- 相同文本重复 observe（包括同文本换了 urgency 且未升级）⇒ **不改计时**，仅可能升级 urgency。
- 密集候选不会缩短可见时长：可见文本必须满 `min_dwell` 才允许被 `Normal` 候选替换；
  切换后新文本从切换时刻重新起算 `DWELL_TOOL_MS`（不做「累加清零」的可变窗口）。

## SD3 模块 API（`crates/pony-agent-core/src/bin/tui/status_dwell.rs`）

```rust
pub const DWELL_TOOL_MS: u64 = 1200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusUrgency { Normal, Immediate }

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusCandidate { pub text: String, pub urgency: StatusUrgency }

#[derive(Default)]
pub struct StatusDwell { /* private */ }

impl StatusDwell {
    pub fn new() -> Self;
    /// 喂入本帧候选。now_ms 单调递增。
    pub fn observe(&mut self, now_ms: u64, candidate: StatusCandidate);
    /// 当前应显示的文本；None ⇒ 尚无候选（渲染层回落 status_word）。
    pub fn visible_text(&self) -> Option<&str>;
    /// 是否存在被压制、待切换的候选。
    pub fn has_pending(&self) -> bool;
    /// 下一次需要唤醒重绘的时刻（None ⇒ 无待决切换）。
    pub fn next_wake_ms(&self) -> Option<u64>;
    pub fn reset(&mut self);
}
```

`observe` 语义（按序判定，第一条命中即返回）：

1. `visible` 为 `None` ⇒ 立即显示，`started = now_ms`，`min_dwell = dwell(urgency)`。
2. `candidate.urgency == Immediate` ⇒ 立即切换（覆盖任何 pending），`started = now_ms`，`min_dwell = 0`。
3. `candidate.text == visible.text` ⇒ 不动（`min_dwell` 可保留）。
4. `now_ms >= started + min_dwell` ⇒ 切换到 `candidate`，`started = now_ms`，`min_dwell = dwell(urgency)`。
5. 其余 ⇒ `pending = Some(candidate)`，可见文本不变。

`next_wake_ms`：存在 pending 时返回 `started + min_dwell`，否则 `None`。

## SD4 工具活动聚合（同模块，方案2）

```rust
#[derive(Default)]
pub struct ToolActivity { /* private */ }

impl ToolActivity {
    /// 记录一次工具调用（`StreamToolCall` 唯一写入点）。
    pub fn record_call(&mut self, name: &str);
    /// 轮次边界（`begin_turn_usage`）与 `ToolResult` 收口时清空当前工具（不清计数）。
    pub fn clear_current(&mut self);
    pub fn reset(&mut self);
    pub fn call_count(&self) -> u64;
    pub fn current(&self) -> Option<&str>;
    /// 状态行聚合文案；`call_count == 0` ⇒ None。
    pub fn summary(&self) -> Option<String>;
}
```

`summary` 文案（不截断；宽度由渲染层 `truncate_width` 负责）：

- 1 次：`执行 · {current}`
- ≥2 次：`执行 · {n} 个工具 · {current}`
- `current` 为 None 时：`执行 · {n} 个工具`

## SD5 TUI 接入（`tui.rs`）

1. `#[path = "tui/status_dwell.rs"] mod status_dwell;`，`TuiState` 增字段
   `pub status_dwell: StatusDwell`、`pub tool_activity: ToolActivity`（默认构造补齐）。
2. 唯一候选组装点：`TuiState::status_line_candidate(&self) -> StatusCandidate`
   - `hide_process && ExecutingTool` ⇒ 文本取 `tool_activity.summary()`（None 回落 `执行 · {工具名}`）；
   - 其余取 `status_word_base(self).0`；
   - urgency 按 SD2 判定。
3. 唯一刷新点：主循环每轮（`draw` 之前）调 `state.refresh_status_line(now_ms)`；
   写入点（`StreamToolCall` → `tool_activity.record_call`、`StreamToolResult` → `clear_current`、
   `begin_turn_usage` → `reset`）**不直接 observe**，全部经这一处收敛。
4. 渲染：状态行 word 取 `state.status_dwell.visible_text()`，`None` 回落 `status_word`；
   elapsed 仍由 `elapsed_label` 实时拼接。
5. 唤醒：`cap_poll_ms` 与 `status_dwell.next_wake_ms()` 取 min（保留既有 30ms 下限与 goal banner 逻辑）。

## SD6 验收

- `cargo test -p pony-agent-core --bin pony -- status_dwell`（单元测试全绿，见 checklist）
- `cargo check -p pony-agent-core --bin pony` 零 warning（生产构建 `-D warnings`）
- 可见时长：同一轮内 3 个连续工具调用（间隔 100ms）时，可见文本切换次数 ≤ 2，每次可见 ≥ 1200ms（单测断言）

## 明确不在本次范围

- `add_or_update_tool_event`（tui.rs:2629）在末条为 `ToolCall` 时无条件原位覆写 —— 并行工具互相覆盖
  是独立缺陷（事件不携带 tool id，需改上游载荷），本契约不动。
