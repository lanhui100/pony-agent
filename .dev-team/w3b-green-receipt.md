# W3b GREEN 收据（task-11）—— 共享宿主 SessionStore

- 角色：Executor（实现；不改 W1/W2/W3 冻结测试，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- 变更范围（4 个文件 / +184 / −15）：
  - `crates/pony-agent-core/src/agent/runtime/mod.rs`（+42 / −9）
  - `crates/pony-agent-core/src/agent/runtime/builder.rs`（+55 / −6）
  - `crates/pony-agent-core/src/agent/subagent_runtime.rs`（+27 / −1）
  - `crates/pony-agent-core/src/agent/runtime/tests.rs`（+75 / −0，**新增**两个用例，未改任何既有用例）

## 1. 门禁（物理凭证）

```
$ cargo check -p pony-agent-core
    Finished `dev` profile [unoptimized] target(s) in 0.23s          # exit 0
$ cargo test -p pony-agent-core --lib agent::runtime::tests
    test result: ok. 152 passed; 0 failed; 0 ignored; 0 measured; 1105 filtered out; finished in 33.78s   # exit 0
$ cargo test -p pony-agent-core --lib agent::planner
    test result: ok. 33 passed; 0 failed; ...                         # exit 0
$ cargo test -p pony-agent-core --lib agent::runtime::stream_support
    test result: ok. 8 passed; 0 failed; ...                          # exit 0
$ cargo build -p pony-agent-core --bin pony
    Finished `dev` profile [unoptimized] target(s) in 0.23s          # exit 0
```

（`agent::runtime::tests` 共 **152** 条，全部 ok；其中包含本次新增的 2 条 W3b 用例，见下。）

新用例单独运行：

```
$ cargo test -p pony-agent-core --lib -- w3b_shared_store --nocapture
test agent::runtime::tests::w3b_shared_store_makes_teammate_session_visible_to_host_control_plane ... ok
test result: ok. 1 passed; ... finished in 0.11s
$ cargo test -p pony-agent-core --lib -- builder_shared_session_store
test agent::runtime::tests::builder_shared_session_store_is_the_runtime_sessions_handle ... ok
```

未跑全量测试（按任务约束）。

## 2. 实现 file:line

| 项 | file:line |
| --- | --- |
| `AgentRuntimeBuilder.sessions_shared: Option<Arc<RwLock<SessionStore>>>` | `runtime/builder.rs:7`（初始化 `:22`） |
| `pub fn shared_session_store(...)`（优先于 `session_store`） | `runtime/builder.rs:47` |
| `build()`：统一得到一个 Arc（shared 优先 > 自有 store > `SessionStore::new()`） | `runtime/builder.rs:107-111` |
| `build()`：把同一 Arc 交给 `ensure_host_subagent_engine(...)` | `runtime/builder.rs:112-115` |
| `build()`：用 `AgentRuntime::with_shared_sessions(arc, ...)` 建栈 | `runtime/builder.rs:124` |
| `ensure_host_subagent_engine(workspace_root, shared_sessions)`（幂等，不 panic） | `runtime/builder.rs:181-195` |
| `AgentRuntime::with_shared_sessions(Arc<RwLock<SessionStore>>, ...)`（新构造器） | `runtime/mod.rs:188` |
| `AgentRuntime::with_dependencies` 改为包装 Arc 后委托（行为不变） | `runtime/mod.rs:163-179` |
| `HostTurnEngine.shared_sessions` 字段 | `subagent_runtime.rs:182` |
| `HostTurnEngine::with_shared_sessions(Option<Arc<...>>)` | `subagent_runtime.rs:207` |
| worker 前克隆句柄（`self` 不跨线程） | `subagent_runtime.rs:268` |
| 队友建栈时 `.shared_session_store(shared)` | `subagent_runtime.rs:291-293` |
| 测试 ①：`Arc::ptr_eq(runtime.sessions_handle(), &shared)` | `runtime/tests.rs:13234` |
| 测试 ②：HostTurnEngine 走 `with_shared_sessions` 跑真实 turn → `control_plane.list_sessions()` 含队友会话 | `runtime/tests.rs:13255` |

## 3. `control_plane.list_sessions()` 是否能看到队友会话？—— **能，已有测试证据**

机制链（全部字节级可查）：

1. 宿主 `AgentRuntimeBuilder::build()` 统一出唯一的 `Arc<RwLock<SessionStore>>`，
   同一 Arc 既给 `AgentRuntime`（`sessions_handle()`），又给注册的 `HostTurnEngine`
   （`builder.rs:107-115`）。
2. `HostControlPlaneBuilder::runtime(runtime)` 内部取 `runtime.sessions_handle()`
   （`control_plane/mod.rs:1045`），因此 control plane 与 runtime 是**同一张会话表**。
3. 队友 turn 由 `HostTurnEngine` 在 worker 线程建栈，`shared_session_store(shared)` 注入同一 Arc
   （`subagent_runtime.rs:291`），于是队友 turn 的 `append_turn` 直接写进宿主读取的那张表。

**测试证据**（`w3b_shared_store_makes_teammate_session_visible_to_host_control_plane`）：
- 用 `SessionStore::memory_only()` 共享 Arc 建宿主 runtime + `HostControlPlaneBuilder`；
- 直接实例化 `HostTurnEngine::with_shared_sessions(Some(shared))`，以本地 mock provider
  跑一次真实队友 turn（日志：`turn:run requested=test-openai-chat ... message_preview=reply with ok`
  → `turn:completed`，session = `subagent-ff825f73-…`）；
- 断言 `control_plane.list_sessions()` 中存在 `outcome.session_id` —— **通过**（exit 0）。

## 4. `/agents` Enter 是否从“提示不可加载”变为可进入？—— **前置条件成立，Enter 路径现在真的会走 hydrate**

- `session_is_loadable`（`tui.rs`）判的就是 `control_plane.list_sessions()` 成员；§3 已证明队友会话
  现在就在这张表里 ⇒ 判定为真 ⇒ `enter_teammate_session` 走“保存 return → 切 id → reset →
  `hydrate_state_from_session` → `mode=Normal`”分支，而不是提示分支。
- 适用前提（如实）：宿主 `AgentRuntimeBuilder::build()` 必须是**本进程首个**注册子代理引擎的 builder
  （TUI 正是启动时先建宿主栈，`tui.rs:3284`）。`OnceLock` 首写者胜的语义未改：若某嵌入方在
  “没有宿主 shared Arc”的 builder 上先建栈，引擎会绑定那个 store（该 store 仍是它自己 control
  plane 的那一张，只是不是后来者的） —— 这是既有注册语义，本任务不改变。
- 验证边界：无真实 TTY，未做终端手工交互；`/agents` 的按键→hydrate 走的是既有 `/resume` 同源函数，
  其正确性由既有 `/resume` 用例与本次 `list_sessions` 证据共同支撑。

## 5. 风险自查（按任务要求，未擅自改语义）

1. **每个队友 runtime 仍 spawn 一个 trace persistence worker，指向同一 Arc**（`runtime/mod.rs:184` →
   `trace_persistence.rs:44-69`）。每个 worker 持有**独立** channel，不存在同一条命令被多次处理；
   共享面只是 `RwLock<SessionStore>` 的写锁，写入仍串行且正确。代价：N 个队友 runtime ⇒ N 条
   `pony-trace-persist` 线程 + 共享写锁上的竞争（本次 e2e 与 152 条 runtime 用例均未观察到
   死锁/丢写）。**按要求未改 worker 语义。**
2. 队友会话现在会出现在宿主 `/resume` 列表（它们已成为宿主 store 里的真实会话）——属共享的
   预期副作用，未加过滤（过滤需改 session schema/查询口径，超出本任务边界）。
3. `AgentRuntime::with_shared_sessions` 在构造期对共享 store 取一次**读锁**（读 persisted MCP /
   skill 快照），与队友建栈并发时是短暂读锁，不持锁调用外部逻辑；无锁顺序反转路径。
4. 与 W3 的 `TEAMMATE_SESSIONS` 互补：map 负责“哪个 target 最近一次成功会话”，共享 store 负责
   “该会话对宿主可加载”；两者一致性由 `drive_teammate_turn_in` 同一 `Ok(outcome)` 分支保证
   （先落库、后登记；都在同一成功路径）。

## 6. 未实现 / 未覆盖

- 未跑全量测试；未做 TUI 终端手工验收（无 TTY）。
- 未给 `HostTurnEngine` 增加“运行期热切换 shared store”能力（构造期注入，符合任务范围）。
- 未改 `SessionStore` 的任何持久化/schema 行为；`with_dependencies` 语义与字节行为不变（仅内部包装）。
