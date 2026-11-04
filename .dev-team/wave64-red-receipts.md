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
