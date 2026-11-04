# 契约：wave-64 — 停止即时生效 + 终态由「最后一条状态消息」唯一承载

> 冻结方：Lead。Executor 只读，Test Agent 依据 §2 写黑盒用例；变更走契约演进门禁。
> 分级：**B 级**（跨 core/tauri/前端多模块；命中 blast 关键字 `lock`/`thread`/`io`；含 UI 交互与并发停止路径）。
> 用户诉求（原话摘要）：①桌面端点停止不是马上终止；②「用户中止」不得另起一条消息、不得用绿色（非本项目色彩体系），应由消息区最后一条状态消息承载，并用语义化的暂停图标；③生成出错同样不得有额外错误信息，也由最后一条状态消息表达。

## 0. 非目标（Non-Goals）

1. 不改落库历史语义：取消轮哨兵句 `用户终止，发送消息可继续。` 继续留在 checkpoint/历史读取侧（`lib/runtime/cancelled-turn.ts`、`lib/runtime/messages.ts` strip 行为不动）。
2. 不改 TUI 侧呈现与按键语义。
3. 不重构 turn 执行为全异步；不消灭 `spawn_blocking` 不可抢占性。
4. 不改 `stop_turn` 的既有 C1/C2/C3 语义（wave-60 契约继续有效），只让桌面**实际停止路径**同样享有该语义。
5. 不新增对外错误码；`error` 字段既有码值不变。

## 1. 事实基线（取证，HEAD=06472465）

- 桌面 `stopTurn()` 因 `activeRunId` 恒非空 ⇒ 实际调用 `stop_graph_run`（src/stores/runtime.ts:5394）。
- `stop_graph_run` 只做 `request_stop`，**不派发** `turn:stop_requested`（C2）、**不启动**宽限监督（C5）（crates/pony-agent-core/src/agent/control_plane/mod.rs:2327-2333）。
- 工具执行期用户取消 ⇒ 走 `fail_stream_turn_with_hook_dispatch` ⇒ `turn:failed`（错误语义 + 错误消息），非 `turn:cancelled`（runtime/turn_stream.rs:458-478）。
- provider 读通道取消码 `provider_stream_cancelled` 冒泡后同样进 failed 分支（runtime/turn_stream.rs:2013-2083 一带）。
- 前端终态停止文案写 `sessionNotice`，投射为 `info` 模式 ⇒ emerald + `CheckCircle2`（status bar 无 stopped 模式）。
- 失败轮向助手消息写占位正文 `本轮执行失败，请查看右侧 trace。`，并在消息区渲染「错误详情」折叠面板（WorkspaceTurnItem.vue:741-768）。

## 2. 验收判据（黑盒，Test Agent 依据本节编写；Executor 只读）

### R1 桌面停止路径享有 C2/C5 语义（Rust）
- 对存在活跃 turn 的 graph run 调用 `stop_graph_run`，**必须**：① 同步派发**恰好一条** `turn:stop_requested`（`stopReason="user"`、`kind="stop_requested"`）；② 启动宽限监督；③ 宽限期（测试注入毫秒级小值）内循环仍未退出 ⇒ 幂等收口并派发**恰好一条** `turn:cancelled`（`stopReason="user"`）。
- 幂等：同一 run/turn 重复 stop 不得重复派发终态或 `turn:stop_requested`。
- graph run 已终态 / run 不存在时保持既有错误返回，不派发任何事件。

### R2 用户取消不得被归类为错误（Rust）
- 工具执行期（工具返回 `aborted`）且 `should_cancel_turn` 为真 ⇒ 终态**必须**是 `turn:cancelled`（`stopReason="user"`），**禁止**派生 `turn:failed`。
- provider 读通道以 `provider_stream_cancelled` 结束且 `should_cancel_turn` 为真 ⇒ 同样归 `turn:cancelled`，禁止 `turn:failed`；checkpoint 终态为 `cancelled`。
- 非用户取消的真实失败（无停止请求）行为不变：仍 `turn:failed`。

### F1 终态状态消息（前端唯一承载面）
- `AgentRuntimeMode` 新增 `"stopped"`。图标集合：`stopped` ⇒ lucide `Pause`（`data-testid="agent-runtime-status-icon"`、`data-mode="stopped"`）；`error` ⇒ `AlertCircle`（既有）。
- `stopped` 语义色 = **中性 stone**（禁止 emerald / 绿系）；`error` = rose（既有）。
- `stopped` 无 shimmer 扫光、无耗时计时（`startTime` 为 null）。
- store：`turn:cancelled` 后 `agentRuntimeStatus.mode === "stopped"`，`label === "已停止"`，且 **`sessionNotice` 不再被写入**停止文案（`sessionNotice` 仅保留 C-9 斜杠命令 info 用途）。
- store：`turn:failed` 后 `agentRuntimeStatus.mode === "error"`，label 为真实错误文案。
- 终态状态行必须可见（`isRuntimeStatusVisible` 对 `stopped` 与 `error` 均返回 true），位置保持在消息区末尾（`workspace-runtime-status-row`）。
- 新一轮提交（`isSubmitting` 变 true）后终态状态行消失。

### F2 不得另起消息（前端）
- `turn:cancelled` 时**禁止**向助手消息 content 写入 `本轮已停止。` 占位（浏览器预览取消路径同规则）。
- `turn:failed` 时**禁止**写入 `本轮执行失败，请查看右侧 trace。` 占位正文。
- 消息区**不再渲染**「错误详情」折叠面板（`WorkspaceTurnItem` error 分支、`HomeWorkspace` 的 error 事件注入与 `shouldRenderAssistantAsError` 正文抑制一并移除）；失败轮已产出的正文照常渲染。
- 错误明细仍保留在 store 的 `errorDetail` 与右侧 Trace 面板（不新增 UI）。

## 3. 写域（正交）

| 分支 | write_scopes |
|---|---|
| executor-fe | `src/lib/runtime/status.ts`、`src/components/chat/AgentRuntimeStatusBar.vue`、`src/stores/runtime.ts`、`src/components/HomeWorkspace.vue`、`src/components/chat/WorkspaceTurnItem.vue`、`src/lib/runtime/constants.ts` |
| executor-rs | `crates/pony-agent-core/src/agent/control_plane/mod.rs`、`crates/pony-agent-core/src/agent/runtime/turn_stream.rs`、`crates/pony-agent-core/src/agent/stop_escalation.rs`、`src-tauri/src/lib.rs` |
| test-agent | `tests/acceptance/wave64/`、`crates/pony-agent-core/tests/stop_graph_run_wave64.rs`、`tests/stop-stop-turn.spec.ts`、`tests/HomeWorkspace.spec.ts`、`tests/runtime-store.spec.ts`（契约演进迁移） |

## 4. 测试迁移（契约演进，登记 breaking）

- `tests/stop-stop-turn.spec.ts` 中断言 `sessionNotice === "你已停止本轮 · 发送新消息可继续"` 的判据**作废**，迁移为 `agentRuntimeStatus.mode === "stopped"` + `sessionNotice === null`。
- `tests/HomeWorkspace.spec.ts` 关于「错误详情」面板存在的**两处**断言（约 :863-864 与 :890-894）**作废**，迁移为「状态行 error 模式承载错误、消息区无错误详情面板」。
- `tests/runtime-store.spec.ts` 中「浏览器预览取消后 content === 本轮已停止。」的断言**作废**，迁移为「取消后不得写入占位正文」；同类失败占位正文断言一并迁移。
- **终态状态派生优先序**（Lead 裁决 C）：`sessionOperation` > `sessionError` > 终态 `error` > `sessionNotice`(C-9 斜杠命令 info) > 终态 `stopped`。理由：notice 是显式瞬时反馈，不得被陈旧终态吞掉。

## 5. 门禁

- 前端：`npx vitest run tests/acceptance/wave64 tests/stop-stop-turn.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-status.spec.ts`，`npm run typecheck`。
- Rust：`cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_graph_run_wave64`，`cargo check --manifest-path src-tauri/Cargo.toml`。
- 真实浏览器（Playwright browser-preview 通道）：Console Error = 0 + 关键截图落 `docs/release/evidence/wave-64/`。
- 可观测性：`audit-diff.sh --base <baseline>` Exit 0 或按 WARN 逐条说明；`docs/observability.yaml` 四维归属需覆盖本次 UI 交互改动。
