# Agent Note: TUI 状态行 min-dwell + 轮内工具活动聚合（折叠态「生成中」不再吞掉过程信息）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0096-tui-status-line-min-dwell-and-tool-activity.md` 之归档快照，仅供溯源。

日期：2026-11-08
契约真值源：`.dev-team/contract-status-dwell-v1.md`（SD1–SD6，v1.1 已实现口径）

## 背景

用户实测（默认显示偏好是「隐藏过程 + 折叠思考」，ADR 0079）后报缺陷：

> 工具调用的状态消息**一闪而过**，绝大部分时间状态行只有一句「生成中」，而工具消息里才有
> 「具体在干什么」的信息 —— 状态行因此丧失了绝大部分价值。

代码层能指认两个结构性原因（不是渲染参数问题）：

1. **状态行是「当前快照」而非「可见文本」**：`ui()` 每帧直接读 `status_word(state)`。
   `UiEvent::StreamToolCall` → `ExecutingTool`、`UiEvent::StreamToolResult` → `Responding`
   都是即时覆盖，工具执行时长（`read_file` 这类毫秒级工具）往往短于一次绘制间隔，
   于是折叠态里唯一的进程信息（`hidden_tool_status_text`，契约 C10）从没被画出来。
2. **「生成中」期间没有任何过程上下文**：工具结束后运行态回 `Responding`，状态行退回
   一个不含信息的词；而一轮里模型生成本身往往占几十秒，状态行大部分时间处于这种空洞态。

历史背景：`aa08c41e` 曾入库一份契约草案（`.dev-team/contract-status-dwell-v1.md`，
提交信息明确写「实现未落地」），代码从未接入 ⇒ 用户感知为「修过但没生效」。

## 决策

1. **状态行改为「受最小驻留约束的可见文本」**
   - 新增纯逻辑模块 `crates/pony-agent-core/src/bin/tui/status_dwell.rs`（`StatusDwell`）：
     `observe(now_ms, candidate)` 的状态机语义为
     ①无可见文本 ⇒ 立即显示；②同文本 ⇒ 只同步语义色、**不重置**计时；
     ③`Immediate` 抢占；④驻留满 `min_dwell` ⇒ 切换；⑤其余挂起为 pending（**只保留最新候选**，
     不为被覆盖的候选排队补帧）。
   - 分级：终态与用户动作反馈（`Ready` 一次性提示 / `WaitingApproval` / `Completed` /
     `Failed` / 「正在停止…」）为 `Immediate`（0ms）；过程态
     （`Connecting` / `Thinking` / `Responding` / `ExecutingTool`）为 `Normal`（`STATUS_DWELL_MS = 1200`）。
   - 时间口径：模块不读时钟，`now_ms` 单调毫秒由调用方注入；单测可喂假时间。

2. **折叠态状态行携带轮内工具活动（`ToolActivity`）**
   - 写点唯一：`StreamToolCall` → `record_call`（计数 + 当前工具），
     `StreamToolResult` → `complete`（当前 → 最近，不清计数）。
   - 复位唯一：`begin_turn_usage`（`turn:started`）、`reset_session_scope`、`clear_messages`、
     以及终态（`StreamCompleted` / `StreamFailed` / `apply_stop_terminal`）清驻留态。
   - 文案：工具执行期 `执行 · {工具名} · {参数摘要}`；生成/思考期
     `{状态词} · {N 个工具 · 最近 {最近工具}}`。宽度不足时按
     `状态词 · 摘要` → `状态词 · 最近工具` → `状态词 · N 个工具` → `状态词` 逐级降档
     （**优先保住「最近工具」**，计数先丢）。
   - 非折叠态维持原口径（正文里已有工具行，ADR 0043 §决策 3 的「一行一个短语」不破）。

3. **唯一候选组装点与唯一刷新点**
   - 组装：`status_line_candidate(state, budget)`；语义色收敛为 `StatusTone`
     （`Muted/Faint/Warn/Success/Danger`），由 `status_tone_style` 单点映射到 `theme`，
     `status_word_base` 不再直接产出 `Style`。
   - 刷新：`ui()` 内 `status_line_now(state, budget)`（喂候选 → 取可见文本），事件写点不直接改可见文本。
   - 唤醒：主循环把 `status_dwell.next_wake_ms()` 并入既有 `cap_poll_ms`（与 goal 相位取最小），
     到点置 `dirty` 重绘 —— 不依赖 spinner tick 的 160ms 精度。

4. **`hidden_tool_status_text` 改读单点真值**
   - 优先 `tool_activity.running()`；旧路径（测试/直接 `add_tool_event`）回落末条 `ToolCall` 消息，
     保持契约 C10 的既有文案与调用面不变。

## 备选方案与取舍

- **只调大动画帧率 / 缩短绘制间隔**：闪烁来自「状态快照被事件覆盖」，与帧率无关；提高帧率只会让
  消失更快。拒绝。
- **工具态与生成态同权重轮播（状态行按固定节奏在「生成中」和「最近工具」之间来回切）**：
  用户读状态时会被无关的周期性变化干扰，且实现上要额外维护相位与相位边界，信息量不增。拒绝。
- **工具执行结束后把「执行 · x」一直保持到下一次工具**：会让「模型正在生成」这一事实完全消失
  （三态语义被打破）。改为 `生成中 · N 个工具 · 最近 x`：先给状态、再给上下文。
- **所有候选一律立即切换，只靠「工具态最少显示」兜底**：需要两套计时规则（显示侧 + 压制侧），
  且密集工具序列仍会互相踩。统一成「可见文本满 min-dwell 才可被 Normal 替换」更简单。
- **把计数/最近工具写进消息区（折叠时不渲染）或指标行**：指标行是资源读数（token/上下文），
  混入过程语义会污染其口径（ADR 0034 / 0084）；消息区在折叠态本就不可见。拒绝。
- **在 `add_or_update_tool_event` 里修补（原位覆写导致并行工具互相覆盖）**：事件不携带 tool id，
  需改上游载荷，与本次「状态行可见性」是两件事；明确不在范围（与契约草案一致）。

## 后果

- 新增 `crates/pony-agent-core/src/bin/tui/status_dwell.rs`（含 9 条单测：首次显示 / 短工具态驻留 /
  pending 只保留最新 / Immediate 抢占 / 同文本不重置计时 / 语义色同步 / 0ms 终态可被替换 /
  工具聚合文案 / reset）。
- `crates/pony-agent-core/src/bin/tui.rs`：`TuiState` 增 `status_dwell` / `tool_activity`；
  `status_word_base` 返回 `(String, StatusTone)`；新增 `status_line_candidate` /
  `status_with_tool_context` / `status_line_now` / `status_tone_style`；工具事件、轮次起点、
  会话复位、终态清点各一处写入/复位；主循环唤醒并入 `cap_poll_ms`。
- 验收：`tests/tui_process_fold_acceptance.rs::a15_folded_generation_keeps_tool_activity`
  （折叠态生成期携带 `1 个工具` / `2 个工具 · 最近 …` / 窄屏降档，且三态词互斥、工具行仍隐藏）。
- 既有测试无需放宽：`tui_acceptance` 118、`tui_process_fold_acceptance` 97、`tui_render_layout` 92、
  `tui_dialog_wave39` 95、`tui_streaming_input_acceptance` 105 全绿。
- 已知残余：状态行 spinner 图标仍取自**当前** `runtime_mode`，在驻留窗口内（≤1.2s）可能与可见文本
  来自不同状态；两者都是过程动画且同为 `ACCENT_DIM`，不构成语义误读，未额外建「可见状态种类」字段。

## 关联

- ADR 0043（三态状态词互斥：`生成中` 不与 `执行 ·` 同行）
- ADR 0079（过程默认隐藏 + 思考折叠；C10 隐藏态状态行承担工具信息）
- ADR 0042 / 0084（底部指标行口径：状态词不进指标行）
- `.dev-team/contract-status-dwell-v1.md`（本特性契约真值源，已更新为实现口径）

