# Agent Note: TUI 状态行以轮次生命周期为唯一门，且文本由纯函数产出（含已耗时）

Status: implemented

## Problem

用户报告 TUI「跑到一半就没有任何进行中提示」。上一轮（ADR 2026-11-16-tui-running-status-visibility）已把渲染门从 `is_streaming && ...` 收敛为纯函数 `should_render_running_status`，但现场仍有三个可复现的残余面：

1. **门没有类型化的生命周期真源**：`should_render_running_status`（`crates/pony-agent-core/src/bin/tui.rs`）用一段 `matches!(state.runtime_mode, Connecting | Thinking | Responding | ExecutingTool(_))` 表达「轮次在跑」。新增运行态（例如后续的 `WaitingFirstByte`）不会编译失败，而是**静默落入 Terminal**，缺陷以「某个入口没有状态行」的形式复发。`status_urgency` / `status_word_base` 各自重复同一段 match（三处口径）。
2. **状态行文本不是纯函数产物**：`bin/tui.rs` 的渲染点在 `ui()` 里把 `status_line_now()`（经 `StatusDwell`）的文本与 `elapsed_label(state)` 现场拼接。于是「运行态下这一行非空且含已耗时」这条验收口径**无法在纯函数层断言**，只能靠渲染快照间接验证；而一旦有人只改渲染门、不改拼接点，就会出现「行画出来了但秒数定格」的假进行中（同 ADR 已记录的时钟门分叉）。
3. **goal 自动续轮/续轮入口没有起点时间**：`turn_started_at` 只在主循环派发点、Ask 续轮、审批续轮三处写 `Some`（`tui.rs:11300/11879/11992`）；`UiEvent::TurnStarted` 事件臂不写。goal 自动续轮在轮次线程内直接开轮（`tui.rs:9098-9112`），因此该入口的 `elapsed_label` 恒为空——状态行看得见，但「已耗时」永远缺省。

backlog:B218 与已闭环的 backlog:B215 是同一缺陷（同一渲染门、同一指纹面），本轮合并为 B218 一张实施卡。

## Decision

把「轮次在跑」提升为一个显式类型，并把状态行文本下沉为单一纯函数：

1. `bin/tui/status_dwell.rs` 新增 `pub enum TurnLifecycle { Running, Terminal }`（无 ratatui 依赖，与既有 `StatusDwell` / `ToolActivity` 同层）。
2. `bin/tui.rs` 新增 `pub(crate) fn turn_lifecycle(state: &TuiState) -> TurnLifecycle` 作为**唯一**生命周期判定点；`should_render_running_status` 与 `status_urgency` 全部改为消费它，删除各自重复的 `matches!`。运行态集合只在这一处列举，未来新增模式漏写会直接落到 `Terminal` 分支并被既有「终态不渲染」测试抓住。
   - 生命周期还吸收两类**运行期覆盖**：`stopping_turn_id.is_some()`（用户已请求停止、终态未到）与 `provider_retry.is_some()`（上游正在重试）一律判 `Running`。它们不改变 `runtime_mode`，但此刻「轮次仍在跑」是用户唯一关心的事实；这条覆盖只写在 `turn_lifecycle` 里，不再由渲染门与 `status_urgency` 各写一份。
3. 新增 `pub(crate) fn compose_running_status_text(state: &TuiState, word: &str, elapsed: &str) -> Option<String>`：
   - `Terminal` ⇒ `None`（终态不得残留假进行中）；
   - `Running` ⇒ `Some(非空文本)`，`elapsed` 非空时**逐字**出现在文本中；`word` 为空白时回落非空占位（`处理中`），不允许产出空行。
   - 渲染点 `ui()` 改为经该函数组装，`StatusDwell` 仍只负责「显示哪个状态词」——min-dwell 语义零变更。
4. `UiEvent::TurnStarted` 事件臂在 `turn_started_at.is_none()` 时写入当前时刻：起点时间由**真实轮次事件**驱动，而不是只由派发点自觉设置；goal 续轮入口因此获得非空已耗时。

## Alternatives considered

- **A. 沿用 `matches!(runtime_mode, ...)`、只补测试**：落选。测试能抓住「当前遗漏」，但抓不住「未来新增模式漏写」——那正是本缺陷的复发机制；类型化生命周期让遗漏变成编译期可见的分支。
- **B. 用 `turn_started_at.is_some()` 当生命周期门**：落选。该字段在终态轮次里不会自行清空（只在会话切换复位），会把完成/失败渲染成进行中，比不渲染更误导（同上一轮 ADR 的否决理由）。
- **C. 把 elapsed 拼进 `StatusDwell` 的候选文本**：落选。`StatusDwell` 以「文本相同即续期」判等，秒数每秒变化会让驻留计时永不满足，min-dwell 直接失效；耗时必须在驻留判定**之外**拼接。
- **D. 新增独立的第二个状态行组件（对齐 dsh 的 RunningStatus 组件）**：落选。当前实现已是「正文尾部一行」，再加一层组件只增加渲染面与滚动几何的耦合；本缺陷缺的是门与文本的单点，不是组件。
- **E. 只修 goal 续轮入口的 `turn_started_at`，不加类型**：落选。它只修第三条症状，不解决「三处 match 各自为政」的结构问题。

## Consequences

- 状态行可见性、紧急级别、文本组装三处共享同一个 `turn_lifecycle`，口径分叉从「靠注释约束」变成「类型 + 单点」。
- 验收可机器判定：`cargo test -p pony-agent-core --test tui_scrollbar_drag`（`#[path]` 引入 `bin/tui.rs` 的既有接缝）内新增 B218 用例直接断言纯函数返回值非空、含 elapsed、终态为 `None`。
- goal 自动续轮、Ask 续轮、审批续轮三类入口的已耗时全部非空；`turn_started_at` 不再依赖派发点。
- 不新增对外接口、网络 IO、后台任务或 LLM 调用；状态行仍是终端本地呈现（`docs/observability.yaml` 四维归属不变）。
- 与 ADR 2026-11-16-tui-running-status-visibility 的关系：该 ADR 决定「运行期恒显」，本 ADR 决定「恒显的门与文本如何被类型化与单点化」；两者不冲突，后者是前者的结构收口。
