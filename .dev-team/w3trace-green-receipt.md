# W3 trace GREEN 收据（task-12）—— teammate/subagent turn 继承宿主 traceparent

- 角色：Executor（实现；不改 W1/W2/W3 冻结测试，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- 变更范围（6 个文件 / +326 / −12）：
  - `crates/pony-agent-core/src/agent/otel/mod.rs`（+194 / −1）
  - `crates/pony-agent-core/src/agent/subagent_runtime.rs`（+70 / −2）
  - `crates/pony-agent-core/src/agent/orchestration.rs`（+52 / −0）
  - `crates/pony-agent-core/src/agent/runtime/mod.rs`（+14 / −0）
  - `crates/pony-agent-core/src/agent/runtime/turn_sync.rs`（+4 / −3）
  - `crates/pony-agent-core/src/agent/runtime/turn_stream.rs`（+4 / −3）

## 1. 门禁（物理凭证）

```
$ cargo check -p pony-agent-core
    Finished `dev` profile [unoptimized] target(s) in 6.75s           # exit 0
$ cargo test -p pony-agent-core --lib agent::otel
    test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 1216 filtered out; finished in 9.48s   # exit 0
$ cargo test -p pony-agent-core --lib agent::runtime::tests
    test result: ok. 152 passed; 0 failed; 0 ignored; 0 measured; 1107 filtered out; finished in 34.08s  # exit 0
$ cargo build -p pony-agent-core --bin pony
    Finished `dev` profile [unoptimized] target(s) in 13.07s          # exit 0
```

新增两条单测（属于 `agent::otel::tests`）：

```
$ cargo test -p pony-agent-core --lib -- traceparent_parsing begin_turn_trace_with_parent --nocapture
[w3-trace] parent_trace=4bf92f3577b34da6a3ce929d0e0e4736 scope_trace=Some("4bf92f3577b34da6a3ce929d0e0e4736") current_traceparent=Some("00-4bf92f3577b34da6a3ce929d0e0e4736-f57789befc2449f0-01")
test agent::otel::tests::traceparent_parsing_yields_remote_parent_context ... ok
test agent::otel::tests::begin_turn_trace_with_parent_inherits_trace_id_locally ... ok
test result: ok. 2 passed; 0 failed; ...
```

未跑全量测试（按任务约束）。

## 2. 实现 file:line

| 项 | file:line |
| --- | --- |
| `traceparent_to_span_context`（W3C 严格解析：version != ff、32/16/2 位小写 hex、非全 0、remote=true） | `otel/mod.rs:1987` |
| `local_child_span_context`（降级通路：沿用父 trace_id + 新 span_id + sampled + 非 remote） | `otel/mod.rs:1960` |
| `begin_turn_trace_with_parent(session_id, turn_id, sequence, parent: Option<&str>)` | `otel/mod.rs:2051` |
| `begin_turn_trace(...)` 委托 `None`（原行为逐字不变） | `otel/mod.rs:2036` |
| `AgentRuntime.parent_traceparent: Option<String>` | `runtime/mod.rs:150`（初始化 `:236`） |
| `AgentRuntime::set_parent_traceparent(...)` | `runtime/mod.rs:249` |
| turn span 创建改调 with_parent（同步入口） | `runtime/turn_sync.rs:701`（`self.parent_traceparent.as_deref()`） |
| turn span 创建改调 with_parent（流式入口） | `runtime/turn_stream.rs:1715` |
| trait 默认方法 `run_turn_with_selection_and_parent(...)`（忽略 parent，委托 `run_turn_with_selection`） | `subagent_runtime.rs:111` |
| `HostTurnEngine` 覆写 `run_turn_with_selection_and_parent`（None/None 回落 `run_turn_in`，其余建栈注入父头） | `subagent_runtime.rs:451` |
| `run_turn_with_root_and_selection` 增加 `parent_traceparent: Option<String>` 参数 | `subagent_runtime.rs:272` |
| 建栈后、跑 turn 前注入 `runtime.set_parent_traceparent(...)` | `subagent_runtime.rs:324` |
| `TEAMMATE_TRACEPARENTS` 进程内表 | `orchestration.rs:417` |
| 登记 / 读取（`lead`、空值、非法值一律忽略 ⇒ root） | `orchestration.rs:420` / `:437` |
| 子代理派发捕获宿主 traceparent | `orchestration.rs:489` |
| 子代理调用 with-parent（后台/sync 两条路径） | `orchestration.rs:515`（bg）/ `:534`（sync） |
| 队友派生捕获宿主 traceparent（**派发时**，仍处宿主 turn 活动上下文） | `orchestration.rs:671` |
| 队友登记 traceparent 快照 | `orchestration.rs:682` |
| `drive_teammate_turn_in` 读取快照并传 with-parent | `orchestration.rs:1178` / `:1181` |
| 单测 ①（解析 trace/span/flags + 7 类非法输入 fail-closed） | `otel/mod.rs:5207` |
| 单测 ②（父 trace id 继承 + 新 span id + 子 traceparent + 非法父头回落 root） | `otel/mod.rs:5254` |

## 3. 语义与证据

1. **有导出后端时**：`build_with_context(..., &Context::new().with_remote_span_context(parent))`
   ⇒ 队友 `agent.turn` span 是宿主 turn span 的**子 span**，同 trace id（父 span id = 宿主当前 turn
   span id）。
2. **无导出后端 / 门禁关闭时（本机测试态）**：走降级本地通路，但 `local_child_span_context`
   沿用父 trace id、只换新 span id ⇒ 日志 `trace_id` 与出网 `traceparent` 与宿主同 trace。
   单测 ② 的物理输出：`parent_trace=4bf92f35…4736`、`scope_trace=Some("4bf92f35…4736")`、
   `current_traceparent=Some("00-4bf92f35…4736-f57789befc2449f0-01")`（同 trace，新 span）。
3. **非法/缺失父头**：`traceparent_to_span_context` 返回 `None` ⇒ 空 `Context` ⇒ root trace
   （与改造前逐字一致；不会因脏头 panic 或造出无效 trace）。
4. **链路完整性**：派发时（`spawn_teammate_in_workspace_with_provider` /
   `subagent_in_workspace_with_provider`，仍在宿主 turn 内）`current_traceparent()` 快照 →
   `TEAMMATE_TRACEPARENTS`（队友）/ 直接入参（子代理）→ `drive_teammate_turn_in` →
   `HostTurnEngine::run_turn_with_selection_and_parent` → 建栈 → `set_parent_traceparent`
   → 该栈的 `begin_turn_trace_with_parent`。

## 4. 验证边界 / 未覆盖

1. **未接真实 OTel 后端**：本机无 collector，证据以「本地上下文 trace id 断言」为准（任务允许）。
   有后端时父子关系由 `with_remote_span_context` 保证（OTel SDK 语义），本波未做导出后
   Langfuse 端到端核对。
2. **未在队友 turn 的出网请求头上直接断言**：`MockHttpServer` 只捕获请求体不捕获 headers，
   因此「队友 provider 请求的 `traceparent` header 与宿主同 trace」由 ①otel 单测 + ②代码路径
   （同一 `set_parent_traceparent` → `begin_turn_trace_with_parent` → `current_traceparent()` 注入）
   共同支撑，未做 header 级单测（需扩展测试 helper，超出本任务写域）。
3. 派发时若**没有**活动宿主 turn（例如外部直接库调用 spawn），`current_traceparent()` 为 `None`
   ⇒ 队友 turn 自起 root trace（fail-closed，语义正确）。
4. `set_parent_traceparent` 作用于该 subagent runtime 的**所有** turn；当前引擎每次驱动都新建
   runtime（一 turn 一栈），因此无跨 turn 串味；若未来复用 runtime 需重新评估。
5. 未跑全量测试；未做 TUI 手工验收（无 TTY）。
