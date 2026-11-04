# wave-64 红相物理收据（Test Agent / 独立测试者）

> 契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2（F1/F2 前端，R1/R2 Rust）。
> 本文件记录**未实现前**的机器原生执行结果（命令原文 + exit code + 关键输出片段）。
> 纪律：断言的是契约要求的目标行为；禁止 skip / 吞异常 / 空断言。

## 0. 环境与分级

- 工作目录：`/home/dm/pony-agent`，HEAD=`06472465`（工作区含**另一并发会话**的未提交改动：otel / tool-result-meta 等，本测试者未触碰、未回退）。
- 分级：**L3**（跨 core/tauri/前端多模块；停止路径含并发/线程；含 UI 交互）。
- 断言实现自检：`npm run typecheck` → **EXIT 0**（新测试文件类型合法）。

## 1. 前端（vitest + @vue/test-utils + pinia）

### 命令原文

```bash
npx vitest run tests/acceptance/wave64 tests/stop-stop-turn.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts
```

### 结果：**EXIT 1**（红相成立）

```
 Test Files  6 failed (6)
      Tests  15 failed | 193 passed | 7 skipped (215)
```

### 关键失败片段（每条都是契约目标判据，不是夹具错误）

| 用例 | 关键失败信息 |
|---|---|
| `f1_turn_cancelled_projects_stopped_mode_and_clears_session_notice` | `AssertionError: F1：turn:cancelled 后 mode 必须是 stopped: expected 'info' to be 'stopped'` |
| `f1_non_user_stop_reason_also_projects_stopped_without_notice` | `expected 'info' to be 'stopped'` |
| `f1_stopped_status_bar_renders_pause_icon_with_neutral_tone` | `F1：stopped 图标必须是 lucide 图标（svg），而不是轨道占位 span: expected 'span' to be 'svg'`（现状 `stopped` 落到 orbit 占位 span） |
| `f1_stopped_terminal_status_row_is_visible_with_pause_icon` | `expected 'info' to be 'stopped'`（HomeWorkspace 状态行 data-mode） |
| `f1_terminal_status_row_leaves_terminal_mode_after_new_submission` | `expected 'info' to be 'stopped'` |
| `f2_cancelled_without_streamed_body_does_not_write_cancelled_placeholder` | `F2：turn:cancelled 禁止写入「本轮已停止。」占位: expected '本轮已停止。' not to be '本轮已停止。'` |
| `f2_browser_preview_cancel_does_not_write_cancelled_placeholder` | 同上（浏览器预览 `cancelBrowserPreviewTurn` 路径） |
| `f2_failed_turn_without_body_does_not_write_failed_placeholder` | `F2：turn:failed 禁止写入失败占位正文: expected '本轮执行失败，请查看右侧 trace。' not to be …` |
| `f2_failed_turn_keeps_already_streamed_body` | `expected '本轮执行失败，请查看右侧 trace。' to contain '这是已经流式输出出来的正文'`（已产出正文被占位覆盖） |
| `f2_failed_turn_status_row_carries_error_and_message_area_has_no_error_panel` | `F2：消息区不得再出现「错误详情」面板: expected '…错误详情tool chain exploded…' not to contain '错误详情'` |
| `tests/stop-stop-turn.spec.ts :: c7_user_cancelled_shows_stopped_notice_and_keeps_streamed_body`（迁移后） | `expected 'info' to be 'stopped'` |
| `tests/stop-stop-turn.spec.ts :: c7_non_user_stop_reason_shows_plain_stopped_notice`（迁移后） | `expected 'info' to be 'stopped'` |
| `tests/HomeWorkspace.spec.ts :: shows a failure summary when a saved session has failed trace history but no transcript`（迁移后） | `expected '错误详情tool chain exploded…' not to contain '错误详情'` |
| `tests/HomeWorkspace.spec.ts :: renders failed turn body inline while the terminal status row carries the error`（迁移后） | `expected '错误详情stack line 1…' not to contain '错误详情'` |
| `tests/runtime-store.spec.ts :: restores a cancelled browser-preview turn without writing a placeholder body`（迁移后） | `expected '本轮已停止。' to be ''` |

### 前端既有门禁用例（未被本次改动影响，绿）

```bash
npx vitest run tests/runtime-status.spec.ts   # EXIT 0，11 passed
```

## 2. Rust（cargo test，core 公开 API 黑盒）

### 命令原文

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_graph_run_wave64 --target-dir target-test
```

### 结果：**EXIT 101**（红相成立；**编译通过**，失败来自真实断言，不是 API 不存在）

```
test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.15s
```

### 关键失败片段

```
thread 'r1_stop_graph_run_dispatches_single_stop_requested_and_grace_cancels_turn' panicked at
crates/pony-agent-core/tests/stop_graph_run_wave64.rs:226:5:
assertion `left == right` failed: R1：stop_graph_run 被接受时必须**同步**派发恰好一条 turn:stop_requested，
实际事件序列 = []
```

```
thread 'r2_provider_stream_cancelled_terminates_cancelled_not_failed' panicked at
crates/pony-agent-core/tests/stop_graph_run_wave64.rs:721:5:
R2：provider_stream_cancelled + 用户取消 ⇒ 终态必须是 turn:cancelled，
实际事件序列 = ["turn:started", "turn:trace", "turn:trace", "turn:failed"]
```

### 同文件已通过（HEAD 现状已满足契约，作为回归护栏保留）

```
test r1_stop_graph_run_unknown_run_keeps_error_and_emits_nothing ... ok
test r2_tool_aborted_during_user_stop_terminates_cancelled_not_failed ... ok
test r2_non_user_failure_still_terminates_failed ... ok
```

> 事实说明（不粉饰）：契约 §1 称「工具执行期用户取消 ⇒ `fail_stream_turn_with_hook_dispatch` ⇒ `turn:failed`」，
> 但以真实流式轮夹具（本地回环 provider + 注入 `ToolExecutor`，工具执行中发出**被接受**的 stop 后返回 `aborted`）
> 在 HEAD 实测得到的是 `turn:cancelled`：`handle_stream_tool_turn` 在工具返回后**先**查
> `should_cancel_turn` 再走 `aborted` 分支（turn_stream.rs:263 早于 :451），因此该路径当前已是取消语义。
> 真正把用户取消误判为失败的是 **provider 读通道取消码 `provider_stream_cancelled`** 分支（R2 第 2 条，红）。

## 3. 覆盖/未覆盖

见 `.dev-team/l2at-done-wave64-tests.json` 的 `uncovered` 字段（含原因与建议入口）。

---

# 第二轮（L3 审查收口后复跑）

## 4.1 测试侧修复项

1. **空集合 for 假绿**（L3 审查项 1）：`f1_terminal_status_row_leaves_terminal_mode_after_new_submission`
   在 `for (const bar of bars)` 之前补前置断言
   `expect(bars.length, "…").toBeGreaterThan(0)`。
   全 `tests/acceptance/wave64/` 自查：断言类集合循环仅此一处（另一处 `for (const timer of rafTimers.values())`
   属 afterEach 清理，不是断言，无假绿风险）。
2. **F2 契约修订补充红相/回归断言**（L3 审查项 2，含 wire `text` 归一化条款）：
   - `f2_failed_wire_empty_compat_text_does_not_become_body`（`text = "This turn failed."` ⇒ 不得写入正文；状态行 error + 真实错误 label）
   - `f2_failed_wire_partial_text_suffix_is_stripped_from_content`（`正文 + "\n\n（该轮失败前已产出以上内容）"` ⇒ content 只含正文）
   - `f2_failed_wire_partial_text_suffix_is_not_rendered_in_message_area`（HomeWorkspace 消息区不得出现失败说明后缀）
   - `f2_background_session_failed_does_not_persist_placeholder_body`（`runningSessionMap` / `updatePersistedBackgroundSession` 分支同样不得落占位正文，且不污染前台会话）

## 4.2 复跑命令与结果

```bash
npx vitest run tests/acceptance/wave64 tests/stop-stop-turn.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts
```

**EXIT 0**

```
 Test Files  6 passed (6)
      Tests  212 passed | 7 skipped (219)
```

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_graph_run_wave64 --target-dir target-test
```

**EXIT 0**

```
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 4.3 解读（不粉饰）

- 第一轮 §1/§2 的红相收据**原样保留**，未删除、未改写；第二轮转绿的原因是 **executor-fe / executor-rs
  的实现已落地**（`src/lib/runtime/status.ts` 的 `stopped`、`AgentRuntimeStatusBar` 的 `Pause`、
  `runtime.ts` 的 `normalizeFailedTurnContent`、core 侧 `stop_graph_run` 的 C2/C5 与
  `provider_stream_cancelled` 取消语义）。
- 新增的 4 条断言属**契约目标行为**：在实现前必然红（wire 文案会写成正文 / 后缀会渲染 / 后台分支会落占位），
  现阶段为回归护栏。

---

# 第三轮：R3 追加红相（L3-A BLOCKING · terminal-once）

## 5.1 用例与夹具（确定性时序，无 sleep 依赖）

用例：`r3_in_band_closure_after_grace_escalation_keeps_single_cancelled_terminal`
（`crates/pony-agent-core/tests/stop_graph_run_wave64.rs`）

场景：C5 宽限监督**已经** escalate（checkpoint 置 cancelled + 派发一条 `turn:cancelled`）之后，
in-band 流式循环恢复并收口 ⇒ 必须仍只派发恰好一条 `turn:cancelled`，且不得被后续 `turn:failed` 覆盖/追加。

夹具与时序（全部为确定性握手，**不使用真实 2s 宽限、不盲等**）：
1. 本地回环 mock provider 进入 **SSE 停帧**（发响应头后不发帧），收到请求即置 `seen`
   —— 这是 in-band 循环「已进入 provider 读通道」的握手点（`server.wait_request()`）；
2. 主线程 `stop_turn(StopTurnCommand{ stop_reason: Some(User) })`
   ⇒ C2 同步派发 `turn:stop_requested` + 启动 C5 宽限监督（`StopEscalationConfig::for_test(40ms, 5ms)`）；
   40ms 刻意 **小于** provider 读通道取消观测节拍（`ProviderStreamGuard::DEFAULT_CANCEL_TICK = 250ms`），
   保证「escalation 先、in-band 后」；
3. 条件轮询到**第一条** `turn:cancelled`（并断言 checkpoint 已 cancelled）——确认 escalation 已收口；
4. `server.release_stream()` 打开停帧闸门 ⇒ mock 关闭连接，in-band 循环恢复（EOF / 取消码收口）；
5. `wait_until(worker.is_finished())` 后断言终态计数。

## 5.2 命令与结果：**EXIT 101（红相成立）**

```bash
cargo test --manifest-path crates/pony-agent-core/Cargo.toml --test stop_graph_run_wave64 --target-dir target-test
```

```
test r3_in_band_closure_after_grace_escalation_keeps_single_cancelled_terminal ... FAILED
test result: FAILED. 5 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.41s
```

关键失败片段：

```
thread 'r3_in_band_closure_after_grace_escalation_keeps_single_cancelled_terminal' panicked at
crates/pony-agent-core/tests/stop_graph_run_wave64.rs:908:5:
assertion `left == right` failed: R3：C5 escalation 之后 in-band 收口不得再派发第二条终态 ——
turn:cancelled 必须恰好一条；实际事件序列 =
["turn:started", "turn:trace", "turn:stop_requested", "turn:cancelled", "turn:cancelled"]
```

即：**同一 turn 派发了两条 `turn:cancelled`**（C5 escalation 一条 + in-band 收口一条），
正是 executor-rs 待落地的 **terminal-once 闸门**要拦掉的场景；`turn:failed` 计数为 0（本时序未走到 failed 覆盖，
但断言同样锁死「不得追加 failed」）。

## 5.3 说明

- 未伪造：宽限监督走真实 `stop_turn` 入口（C2+C5），in-band 走真实
  `AgentRuntime::start_turn_stream_with_control` + 真实 provider 读通道；二者均非 mock 判据本身。
- 无 `#[ignore]`、无吞异常、无空断言；失败信息包含完整事件序列以便定位。
