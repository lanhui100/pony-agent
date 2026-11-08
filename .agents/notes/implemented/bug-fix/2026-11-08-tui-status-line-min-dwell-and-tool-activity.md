# Agent Note: TUI 折叠态状态行「工具态一闪而过、绝大部分是生成中」

Status: implemented

## Context

用户实测（产品默认 `hide_process = true` + `thinking_collapsed = true`，ADR 0079）报缺陷：折叠后
状态行绝大部分时间只有「生成中」，而工具调用才有「具体在干什么」的信息，且工具态一闪而过。

根因是结构性的，不是渲染参数：

1. `ui()` 每帧直接读 `status_word(state)`，工具信息由 `hidden_tool_status_text` 只在
   `RuntimeMode::ExecutingTool` 期间提供；`StreamToolCall` → `ExecutingTool`、
   `StreamToolResult` → `Responding` 都是**事件即时覆盖**，毫秒级工具（`read_file` 等）
   短于一次绘制间隔 ⇒ 折叠态唯一的过程信息从未真正上屏。
2. 工具结束后回 `Responding`，状态行退化成无上下文的「生成中」，而一轮里生成往往占几十秒。

历史：`aa08c41e` 入库过契约草案 `.dev-team/contract-status-dwell-v1.md`（提交信息写明「实现未落地」），
代码从未接入，用户感知为「修过但没生效」。

决策正文见 ADR 0096：[`docs/decisions/0096-tui-status-line-min-dwell-and-tool-activity.md`](../../../../docs/decisions/0096-tui-status-line-min-dwell-and-tool-activity.md)。

## Decision

1. 新增 `crates/pony-agent-core/src/bin/tui/status_dwell.rs`：
   `StatusDwell`（min-dwell 状态机，`Normal = 1200ms` / `Immediate = 0ms`，pending 只保留最新）+
   `ToolActivity`（轮内调用计数 / 当前 / 最近）+ `now_ms`（单调毫秒注入）。
2. `TuiState` 增 `status_dwell` / `tool_activity`；唯一候选组装点 `status_line_candidate`、
   唯一刷新点 `ui()` 内 `status_line_now`；`status_word_base` 改产出 `(String, StatusTone)`，
   `StatusTone` 经 `status_tone_style` 单点映射 `theme`。
3. 折叠态文案：执行期 `执行 · {工具名} · {参数摘要}`；生成/思考期
   `{状态词} · {N 个工具 · 最近 {工具}}`，宽度不足按「保工具、舍计数」降档
   （不引入「执行 ·」词，守住 ADR 0043 三态互斥）。
4. 写点/复位点收敛：`StreamToolCall` / `StreamToolResult` 写；`begin_turn_usage`、
   `reset_session_scope`、`clear_messages`、`StreamCompleted` / `StreamFailed` /
   `apply_stop_terminal` 复位。
5. 唤醒：`status_dwell.next_wake_ms()` 并入主循环 `cap_poll_ms`（30ms 下限）并在画帧前置 `dirty`，
   不新增定时线程。
6. 测试：模块 9 条单测（假时间）+ `tui_process_fold_acceptance::a15_folded_generation_keeps_tool_activity`
   （折叠态生成期携带工具聚合、窄屏降档、三态互斥、工具行仍隐藏）。

## Alternatives considered

- **提高帧率 / 缩短绘制间隔**：闪烁来自「快照被事件覆盖」，与帧率无关，只会消失得更快 —— 落选。
- **状态行按固定节奏轮播「生成中 / 最近工具」**：引入与内容无关的周期性变化，干扰阅读且信息量不增 —— 落选。
- **工具结束后一直保留「执行 · x」**：会让「模型正在生成」这一事实消失，破坏三态语义 —— 落选，
  改为 `生成中 · N 个工具 · 最近 x`。
- **只在 `hidden_tool_status_text` 前置一个 stick 计时器**：显示侧与压制侧两套计时，密集工具序列仍互踩 ——
  统一为「可见文本满 min-dwell 才可被 Normal 替换」。
- **把工具计数写进指标行 / 消息区**：指标行口径被污染（ADR 0042/0084），消息区折叠态不可见 —— 落选。
- 完整比较见 ADR 0096。
