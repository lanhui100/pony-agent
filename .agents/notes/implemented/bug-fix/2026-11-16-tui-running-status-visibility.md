# Agent Note: TUI 进行中状态行改为「运行期恒显」——长耗时不再被误判成会话停止

Status: implemented

## Problem

用户报告：TUI 会话 `pa_01a1df2f-8315-7090-8a4c-978934c4d9cb` 跑到一半「无故停止」，底部没有任何提示，只能靠猜。

现场证据（`~/.local/share/PonyAgent/sessions.db` + `~/.pony-agent/tui-logs/tui-runtime.log`）：

1. 该轮 `turn-90395c04-1395-4452-b0e4-286387dd33d2` 实际跑了 **1,013,009 ms（16.9 min）**、26 次模型调用、35 次工具调用——轮次并没有停止，而是**界面在末段完全没有进行中反馈**。
2. 末段失败链（`[pony-provider]` 日志）：`Write /tmp/err_scan.py` 被沙箱拒绝（`outside_workspace_write_denied`）→ 需要一次 tool-followup → SSE 流读 75,520 ms 后 `error decoding response body` → `followup:stream-fallback` 转同步兜底 → `attempt=1/5` connect 31,000 ms 失败 → `attempt=2..5/5` 每次 21,161 ms 超时 → `budget exhausted: attempt_limit_exhausted` → `followup:stream-error` → 本地兜底摘要（701 字），`turn_end.reason=completed`。
3. 期间同一网关（`tokens.ponyjob.top`）连续失败，`otel.ponyjob.top` 导出同窗口也全部 `TimedOut`——**上游链路抖动**是外部事实，不是可修的产品缺陷。
4. 但「用户以为会话停止」是产品缺陷：`bin/tui.rs` 的状态行渲染门写作

   ```
   if state.is_streaming && (stopping || hide_process || !matches!(mode, ExecutingTool(_)))
   ```

   三重错位：
   - **门用了 `is_streaming`**（派发点标志），而同一文件 `status_word_base` 的注释明确写着「不拿 `is_streaming` 当门：goal 自动续轮等入口在轮次线程内直接开轮」——候选算得出、渲染点不画，两处口径互相打架；
   - **工具执行期整行抑制**（非折叠态）：长工具输出会把工具行顶出视口，此时底部**一个像素的过程信号都没有**；
   - **没有静默心跳**：`Responding` 期若上游长时间不吐增量（本例 180 s），状态行只在「恰好被渲染」时才有——而 `ExecutingTool` 之后的静默段正好被上一条吞掉。

   对照 deepseek harness（`packages/client/ui-chat/src/client/chat/RunningStatus.tsx` + `ChatFlow.tsx`）：它把运行状态作为**运行期恒定尾行**追加（`running ? [<RunningStatus/>] : []`），内部用 1 s 独立时钟渲染 `deepDivingFor(duration)`，与工具行并存、不受「当前在跑什么」影响。我们的实现是「当前状态快照」，任何事件都能把它换掉或干脆不画。

## Decision

把状态行从「当前状态快照」改成「**运行期恒显的进行中尾行**」，门收敛为唯一纯函数：

1. 新增 `should_render_running_status(state) -> bool`（`bin/tui.rs`），作为正文末尾状态行的**唯一**渲染门：
   - `stopping_turn_id.is_some()` 或 `provider_retry.is_some()` ⇒ 恒真（用户此刻唯一关心的事实）；
   - `RuntimeMode::{Connecting, Thinking, Responding, ExecutingTool(_)}` ⇒ 真（**含非折叠态的工具执行期**）；
   - `Ready / Completed / Failed / WaitingApproval` ⇒ 假（终态由 `status_notice`、消息区与 Ask 面板承载）。
2. 渲染点 `ui()` 改为 `if should_render_running_status(state)`，**删除 `is_streaming` 先决条件**，消除「候选算得出、渲染点却不画」的双真源。
3. 不改候选文案与耗时口径：`status_word_base`（重试进度 `正在重试（n/N）…` 优先、`执行 · {工具名}`）与 `elapsed_label`（轮次已耗时）原样保留，因此工具执行期从「无反馈」变成「`执行 · Run  3:12`」。
4. 契约测试 `crates/pony-agent-core/src/bin/tui/tests/running_status_visibility_tests.rs`（6 例）钉死：工具执行期可见、折叠态可见、思考/生成/连接可见、门不依赖 `is_streaming`、终态不渲染、重试/停止强制可见。

## Alternatives considered

- **A. 只把 `is_streaming` 换成 `turn_started_at.is_some()`**：落选。`turn_started_at` 只在会话切换（`reset_session_scope`）时清空，终态轮次里它仍为 `Some` ⇒ 完成/失败后残留一行「完成 X 秒」，比不显示更误导。
- **B. 保留工具执行期抑制，改为「工具执行超过 N 秒才显示」**：落选。需要额外计时器状态与阈值常量，且「N 秒内到底有没有反馈」仍取决于工具行是否在视口内——判据依旧不可靠。恒显的门是纯运行态判据，无时间依赖。
- **C. 每轮结束把 `turn_started_at` 清空，再用它当门**：落选。等价于把「轮次进行中」这个事实拆到两个字段上（`turn_started_at` + 终态事件），清空点一旦漏一处就复发；`RuntimeMode` 已经是该事实的正典。
- **D. 在库层（`agent::retry`）为 stream→sync 兜底加阶段事件**：本轮不做。它能让文案更精确（「同步兜底 3/5」），但需要新增 turn 事件类型、改四端消费与持久化契约；而「用户看不到任何反馈」这一**症状**已在展示层闭合。已作为独立条目 `backlog:B198` 保留（阶段文案），不与本次展示层修复混做。
- **E. 文案追加「已等待 N 秒」心跳**：本轮不做。`elapsed_label` 已每帧给出轮次耗时（160 ms 节拍），再加一句同义文案只是噪音；真正缺失的是**行本身**，不是秒数。

## Consequences

- 长工具调用 / 长思考 / 上游重试期，底部恒有一行「图标 + 状态词 + 已耗时」；折叠态下仍携带轮内工具上下文（既有 `status_line_candidate` 分支不变）。
- 终态不再渲染该行 ⇒ 完成/失败后不会残留假进行中（与既有 `status_dwell.reset()` 语义一致）。
- 门是纯函数且单点，后续再有人加「某模式不显示」的诉求，改一处并补一例测试即可，不会再出现「候选与渲染点各自为政」。
- 与 deepseek harness 的 `RunningStatus` 语义对齐；差异只剩文案与动画（dsh 为 `TextShimmer` 呼吸，我们是 `spinner_tick` 帧）。
- 上游链路抖动仍是外部事实：本次修复只保证**用户能看见「还在跑」**，不掩盖 `B198`（阶段文案）与 `B199`（兜底后误记 `completed`）两个独立缺陷。

## 影响与验证

- 单测：`cargo test -p pony-agent-core --bin pony running_status_visibility`
- 回归面：`cargo check -p pony-agent-core --all-targets`（`bin/tui.rs` 为大文件，改动集中在两个纯函数与一个 `if`）
