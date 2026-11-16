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

- 单测：`cargo test -p pony-agent-core --bin pony running_status_visibility`（10 例，含 4 例时钟驱动门）
- 回归面：`cargo check -p pony-agent-core --all-targets`（`bin/tui.rs` 为大文件，改动集中在三个纯函数与两个 `if`）

## 补充（2026-11-16 收口）：驱动门必须与渲染门同源

首版修复只改了**渲染门**，主循环节拍仍是 `if (state.is_streaming || awaiting_answer)`。这是同一缺陷的第二个真源：`is_streaming` 未置位的续轮入口会把状态行画出来，但 `spinner_tick` 与 `elapsed_label` 不再推进（无 dirty ⇒ 不重绘），用户看到的是「秒数定格」的假进行中——与没有这一行同样会被读成卡死。

- `bin/tui.rs` 新增 `should_drive_running_clock(state, awaiting_answer) = awaiting_answer || should_render_running_status(state)`，节拍条件改为它；`is_streaming` 只保留在 `tick_ms`（160ms 流式 / 450ms 静默段）这一处频率选择上，不再充当门。
- 契约测试补 4 例：无流式的续轮入口要驱动、工具执行期无流式要驱动、终态空转、Ask 等待期独立驱动。
- 登记为 `backlog:B210`（B197 的残余缺口，独立成条而不改写已闭环的 B197）。

## 对照 deepseek harness 的复核（含对首版结论的纠偏）

| 维度 | dsh 事实（本仓路径） | 我们的实现 | 判定 |
|---|---|---|---|
| 挂载条件 | `ChatFlow.tsx:60` `running ? [<RunningStatus/>] : []`，`running` 来自 `useSession(snapshot => snapshot.running)`，即 `ui-session/index.ts` 的 host `api-session/status` 事件 | `should_render_running_status(state)`，由 `RuntimeMode` 正典派生 | 等价（都以「运行态」为唯一真源，不认流式增量） |
| 计时驱动 | `RunningStatus.tsx` 自带 `setInterval(..., LIVE_RUN_CLOCK_INTERVAL_MS=1000)`，与 React 消息渲染解耦 | 主循环帧节拍驱动（本次修正为运行态门） | **首版漏项**：只改渲染门会留下定格秒数，已由 `B210` 闭合 |
| 文案 | `chat.deepDivingFor`「深度求索中，用时 {duration} ···」，`Math.max(1000, now - startTime)`；`startTime` 缺失时退化为无时长文案 | `状态词 + format_elapsed_seconds`；`turn_started_at` 缺失时只出状态词（`elapsed_label` 返回空串） | 等价（含「无起点就不编时长」的退化路径） |
| 重试进度 | 无 per-request 重试文案 | 「正在重试（n/N）…」取自真实 `RetryNotice` | 我们更精确，保留 |
| 审批等待 | host 仍 running ⇒ 尾行继续显示 | `WaitingApproval` 不渲染该行（Ask 面板承载） | 刻意差异：面板已占焦点，底部再挂一行属噪音；若将来需要，改 `should_render_running_status` 一处并补例 |
| 停滞提示 | 无独立 stall 文案，仅靠时长 + shimmer | 同上（时长 + spinner 帧） | 一致；「已等待 N 秒」被首版 Alternatives E 否决，本次复核维持 |

纠偏结论：首版 ADR 的「差异只剩文案与动画」低估了时钟独立性——dsh 的时钟不依赖任何上层事件，而我们的时钟依赖主循环是否 dirty。它不是文案差异，是**反馈能否持续成立**的差异，故单列 `B210` 并在本次一并闭合。

