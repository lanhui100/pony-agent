# PA-076 Phase 4 Review — Plan/Ask Control, graph Ask wait/resume, control-plane commands, frontend

> 审核执行：2026-08-05 · task 4.6（此前标注"待完成"）· 只读审核，未修改任何代码。
> 参照 `2026-08-02-pa076-phase3-review.md` 的格式。

## 审核对象

- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/design.md)（Decisions 5/6/11、Verification Strategy）
- [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/tasks.md) 任务 4.1–4.6 + P1-1（Ask 真实接线）
- 阶段 4 产物：
  - `crates/pony-agent-core/src/agent/plan_state.rs`（PlanStore/PlanControlHandler，revisioned CAS）
  - `crates/pony-agent-core/src/agent/ask_control.rs`（Ask Interaction 适配器）
  - `crates/pony-agent-core/src/agent/graph.rs`（GraphAskWaitBinding / bind_ask_wait / resume_ask_wait / ask_waits）
  - `crates/pony-agent-core/src/agent/runtime/mod.rs`（`handle_sync_tool_turn` :1976、`handle_stream_tool_turn` :777、`suspend_turn_for_control_outcome` :2288、`build_suspended_turn_result` :2375、`tool_result_control_outcome_pending` :6472、`start_turn_stream_with_control_and_facts` :2905、`apply_governed_turn_context` :558）
  - `crates/pony-agent-core/src/agent/control_plane/mod.rs`（`execute_graph_run_stream` :1285 挂起短路 :1364/:1420、`advance_graph_run` :1660 挂起短路 :1719/:1770、`RecordingTurnEventSink::emit` :827、`HostControlPlaneBuilder::build` :923）
  - `crates/pony-agent-core/src/agent/control_plane/ask_plan_commands.rs`（Ask/Plan/graph Ask wait 宿主表面）
  - `crates/pony-agent-core/src/agent/tools.rs`（ToolExecutor::as_any :911、Ask 描述符、ToolOutcome pending :856）
  - `crates/pony-agent-core/src/agent/dispatcher.rs` / `dispatcher_composites.rs`（GovernedToolExecutor、DispatchContext、persist_pending_request :1156、consume_control_request :1244）
  - `crates/pony-agent-core/src/agent/governed_executor.rs`（LegacyCompatiblePolicyEvaluator、build_governed_executor）
  - `crates/pony-agent-core/src/agent/tool_runtime.rs`（PendingControlRequest CAS :124）
  - `src-tauri/src/lib.rs`（ask/plan/graph commands）
  - 前端：`src/stores/ask.ts`、`src/stores/plan.ts`、`src/components/AskPanel.vue`、`PlanPanel.vue`、`src/types/ask-plan.ts`

## 审核阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@architect-code` | **FAIL** | 后端挂起/绑定/恢复链路与测试扎实；但 **Ask 前端 resume 断链**：用户回答 Ask 后 run 永久卡在 `WaitingUser`，`continue_graph_run_stream` 被 `has_unresolved_ask_waits` 拒绝，设计 Decision 5 的"注入唯一终态"仅实现为产出 JSON blob、无任何路径注入 provider turn | P0-1（阻塞） |
| `@security-reviewer` | **FAIL** | 后端 CAS（replay/过期/跨 session/参数摘要不匹配）fail-closed 扎实；Ask≠approval 语义正确；但 `graph_resume_ask` 不校验 dispatcher 请求已被 CAS 消费，绕过单一消费入口；`LegacyCompatiblePolicyEvaluator` 使生产引擎 Write/Edit 全 Allow，phase-3 P1-2 保守默认被覆盖 | P1-2 / P1-3 |
| `@ux-reviewer` | **FAIL** | AskPanel 不消费 `turn:suspended` 终态事件且轮询空闲自终止 → 挂起后 Ask 卡片可能根本不出现；Plan 前端 stale_revision 不 reconcile | P1-1 / P2-5 |

无 P0 级安全漏洞；P0 是前端功能断链。后端四条新测试链路（sync 挂起+恢复、stream 挂起+恢复、共享 dispatcher、graph-run-stream 挂起+恢复）全部通过；`RecordingTurnEventSink` 终态集合正确包含 `turn:suspended`；graph `has_unresolved_ask_waits` 守卫使 `begin_turn`/`apply_turn_result` 在绑定未决时无法绕过。

## 发现与处置

### P0-1 Ask 前端 resume 断链：回答后 run 永久卡在 WaitingUser（task 4.4 "resume" 未接线）

**file:line**
- `src/stores/ask.ts`（全文件）：只封装 `ask_list_pending`/`ask_answer`/`ask_cancel`/`ask_expire`，**无任何 `graph_resume_ask` 调用**。
- `src/components/AskPanel.vue:41-52`：`answerWithOption`/`answerTyped`/`cancelAsk` 只调 `askStore.answer`/`cancel`。
- 全 `src/` grep：`graph_resume_ask`、`graph_list_ask_waits`、`graph_bind_ask_wait` **0 命中**（命令在 `src-tauri/src/lib.rs:276-304` 已注册，前端从未调用）。
- 后端 `control_plane/ask_plan_commands.rs:37-52`：`answer_ask` 只 CAS 消费 dispatcher 的 pending request，**不触碰 graph binding**。
- `graph.rs:558`（`begin_turn`）与 `graph.rs:590`（`apply_turn_result`）：`has_unresolved_ask_waits` 为真时均返回 `None`。

**失败场景**
1. 模型调用 Ask → dispatcher 持久化 Interaction → 挂起 run + `bind_ask_wait` → run 进入 `WaitingUser`（此段已接线且测试覆盖）。
2. 用户在 AskPanel 回答 → `ask_answer` 消费 pending request（state→Consumed，version 1→2）。
3. **无人调用 `graph_resume_ask`**：graph binding 仍在，run 保持 `WaitingUser`。
4. 用户继续发消息 → `continue_graph_run_stream` → `begin_graph_run_stream` → `begin_turn` 返回 `None` → "Graph run cannot accept a new turn"。**run 永久停摆，无任何恢复路径。**

**次要缺陷（同一根因）**：即使前端补调 `graph_resume_ask`，`resume_ask_wait` 产出的 `terminal_result`（`graph.rs:833-844`）只作为命令返回值交给宿主；runtime/control-plane **没有任何机制把该终态喂回 provider turn**（不追加到 conversation history、不触发 provider follow-up）。设计 Decision 5 的"恢复后，runtime 仅以原 tool_call_id 注入一个唯一终态 tool result"目前只实现了"产出 JSON blob"，未实现"注入"。测试 `ask_wait_resume_injects_exactly_one_terminal_result_for_original_call_id`（`graph.rs:1932-2003`）只断言 `terminal_result` 的 shape，未断言任何注入动作。

**建议处置（P0，阻塞）**
1. 前端在 `ask_answer` 成功后，用 ask 载荷中的 `runId`/`requestId` + `graph_list_ask_waits` 返回的 `expectedVersion` 调用 `graph_resume_ask`，随后 `refresh` run 状态。
2. 明确 resume 语义并落地其一：(a) 把 `terminal_result` 追加进下一 turn 的 `TurnInput.history` 后触发 `continue_graph_run_stream`；(b) 或定义 runtime 级 "resume 该 turn" 入口，把终态喂给 provider_followup。二者择一，并补一条"回答后 run 实际继续、provider 收到终态"的前端/集成测试。

### P1-1 前端不消费 `turn:suspended` 终态事件，Ask 面板可能根本不出现

**file:line**
- `src/stores/ask.ts:7`：`ASK_REFRESH_EVENTS = ["turn:completed", "turn:failed", "turn:cancelled"]`，**缺少 `turn:suspended`**。
- `src/stores/ask.ts:216-229`：轮询在 `pendingAsks.length === 0` 后自终止；`startPolling` 只在 AskPanel `onMounted`（`AskPanel.vue:80-89`）调用。
- 后端 `runtime/mod.rs:1170-1209` 挂起时发射 `turn:suspended` 终态事件。

**失败场景**：App 启动后首次 refresh 无 pending ask → 轮询停止。之后 run 挂起（`turn:suspended` 发射），前端无监听者触发 refresh → Ask 卡片不出现，用户不知需要回答。直到 AskPanel 被重新挂载才会恢复。

**建议处置**：把 `turn:suspended` 加入 `ASK_REFRESH_EVENTS`；或让挂起响应（`GraphRunTurnResponse`）显式触发 `askStore.refresh()`。

### P1-2 `graph_resume_ask` 不校验 dispatcher 请求已被 CAS 消费，绕过单一消费入口

**file:line**
- `control_plane/ask_plan_commands.rs:193-213`：`graph_resume_ask` 只调 `graph_runner.resume_ask_wait`，**不检查 `ask_dispatcher.pending_request(request_id).state == Consumed`**。
- 对比：`answer_ask`（同文件 :37-52）走完整 `ControlRequestAuthorization::for_request` → `consume_control_request` CAS（`dispatcher.rs:1244-1305`）。

**失败场景**：宿主（或任何能调 Tauri 命令的调用方）可对未消费的 pending Ask 直接 `graph_resume_ask(run_id, request_id, version, answer)`：binding 被移除、run 恢复、任意 answer 注入终态，而 pending request 仍留在 dispatcher 为 `Pending`（泄漏且可被另一路 `ask_answer` 二次消费/污染）。设计 Decision 5 要求 answer/approve/cancel 均 CAS 一次性消费；`graph_resume_ask` 提供了绕过该契约的独立入口。

**建议处置**：`graph_resume_ask` 先校验 `ask_dispatcher.pending_request(request_id)` 存在且 `state == Consumed`（版本为消费后版本），否则 fail closed；或把"answer + resume"合并为一个宿主级原子命令。

### P1-3 `LegacyCompatiblePolicyEvaluator` 使生产引擎对 Write/Edit 全 Allow，phase-3 P1-2 保守默认被覆盖

**file:line**
- `governed_executor.rs:59-87`：`LegacyCompatiblePolicyEvaluator::evaluate` 除 Ask 外全部 `PermissionVerdict::Allow`。
- `runtime/mod.rs:447`：`AgentRuntimeBuilder::build()` 默认 `build_governed_executor`；`AgentRuntime::new()`（:480）与 Tauri `HostControlPlane::new()`（`lib.rs:763`）均走此路径——**生产引擎即 governed executor**。
- phase-3 修复的 `DescriptorPolicyEvaluator`（Write/Execute → ApprovalRequired）被"宿主注册更精确 evaluator"覆盖。

**说明与处置**：这是 phase-3 记录在案的迁移决定（"切换时需注册真实 policy evaluator"），且相对旧 ToolRouter 无行为回归（旧路径本就无审批）。但 runtime 切换已在本卡前落地为默认引擎，phase-4 又让审批/Ask 机制真实化——`WorkspaceWrite`（Write/Edit）在真实生产路径上无审批、无 `permission_denied`，与 design.md Decision 4"未知 scope fail closed、child scope 单调上收"的最终目标不一致。Run 仍 fail-closed（`sandbox_unavailable`），但 Write/Edit 未受审批门。**建议本里程碑明确决策**：(a) 在 `build_governed_executor` 注册一个对 Write/Edit 返回 `ApprovalRequired` 的校准 evaluator，或 (b) 显式记录"迁移期接受"，并给出关闭该接受的时间点。

### P2-1 `match_pending_control_request` 的 fallback 跨 session/run 匹配

**file:line**：`runtime/mod.rs:6502-6509`。

**失败场景**：tool_call 无 `call_id` 时，fallback 取**整个 dispatcher**最近一个 `Pending` + `Interaction` 请求，不按当前 session/run 过滤。并发多个 run 各自挂起时，可能把 A run 的 ask 绑到 B run 的 `bind_ask_wait_for_request`（`runtime/mod.rs:2330-2332` 用 `pending_request.run_id`，若匹配到错误的 request 则绑错 run）。

**建议**：fallback 增加 `session_id`/`run_id` 过滤（tool_call 无 call_id 时用 `DispatchContext` 的当前事实）；或直接拒绝无 call_id 的 Ask 挂起。

### P2-2 sync 与 stream 挂起路径对 ToolCallEnd hook 调用不对称

**file:line**：sync `runtime/mod.rs:2059`（挂起检测在 tool_end hook 之前）；stream `runtime/mod.rs:1023-1087`（先 dispatch ToolCallEnd 再于 :1136 检测挂起）。

**失败场景**：同一个 Ask 挂起事件，sync 路径的观察者收不到 `ToolCallEnd`，stream 路径收得到；hook trace 与生命周期观测不对称。

**建议**：统一两种路径挂起时的 hook 顺序，并加断言测试固定语义。

### P2-3 `host_control_available` 恒为 true，headless 无 host 时 fail-closed 不生效

**file:line**：`runtime/mod.rs:571` 硬编码 `host_control_available: true`；`dispatcher.rs:66-81` 默认亦为 true。`persist_pending_request`（`dispatcher.rs:1162-1167`）的 `interaction_unavailable` 分支在生产路径不可达（仅测试 :2186 覆盖）。

**说明**：design.md Decision 5 要求"无交互 host 必须生成 interaction_unavailable 终态执行失败"。当前 Tauri 桌面总有 host，故实际无影响；但 core 保持 host-agnostic 的目标下，headless runtime 会挂起等待一个永不回答的 host。phase-3 记为 P3 推迟，现 Ask 已真实接线，建议在 runtime 增加"无 host"显式标志并接入。

### P2-4 Plan 工具信任模型提供的 session_id，可跨 session 操作

**file:line**：`plan_state.rs:434-454`（每个 op 带 `session_id`）、`plan_state.rs:472-496`（`PlanControlHandler::execute_operation` 直接用参数 session_id）。CrossSession 守卫（`plan_state.rs:385-402`）只拦截 `(session_id, plan_id)` 不匹配，不拦截模型自选其他 session 后对他人 plan 做 replace/complete。

**说明**：单用户桌面可接受，但与"Session-owned / CrossSession fail closed"的表述有差距。建议用 `DispatchContext.session_id` 约束 op 的 session_id，不一致时 fail closed。

### P2-5 Plan 前端 stale_revision 失败不 reconcile

**file:line**：`src/stores/plan.ts:160-193`（`completeStep` 捕获错误只 set error 不刷新）；`src/components/PlanPanel.vue:82-84`。

**失败场景**：CAS 失败（如另一入口已改 plan）后，本地保留旧 revision 的 plan；用户再次点"完成"必然再次 stale_revision，无法自我恢复。

**建议**：捕获 `stale_revision` 时自动 `planStore.list(sessionId)` 刷新。

### P2-6 resume 版本语义脆弱：binding 版本 vs 消费后版本

**file:line**：`graph.rs:818-823`（`resume_ask_wait` 要求 `expected_version == binding.expected_version`，即 bind 时版本 1）；`ask_answer` 消费后 dispatcher 版本变 2。宿主必须用 binding 的版本（`graph_list_ask_waits` 返回的 `expectedVersion`）而不是请求当前版本，否则 stale 失败。

**说明**：语义正确但易错。建议文档化合同，并在前端封装 `answerAskAndResume` 原子步骤（同时解决 P0-1 与 P1-2）。

### P2-7 挂起后 execution checkpoint 未更新，telemetry 显示 turn 仍在 calling_tool

**file:line**：stream 挂起路径 `runtime/mod.rs:1136-1211` 只 `emit_stream_event`，无 `update_execution_checkpoint`；`:816-830` 最后一次 checkpoint 为 `calling_tool`。

**失败场景**：挂起后 `load_execution_checkpoint` 仍报 turn `running/calling_tool`，与 graph run 的 `WaitingUser` 不一致，影响 trace/决策观测（挂起短路不消费该状态，故无功能影响）。

**建议**：挂起时把 checkpoint 置为 `suspended`/`waiting_user`。

## 已确认健壮的环节（本轮无问题）

- **共享 dispatcher 真实共享**：`HostControlPlaneBuilder::build`（`control_plane/mod.rs:930-935`）把 `runtime.set_graph_run_store` 注入同一 `Arc<Mutex<GraphRunStore>>`；`ask_dispatcher` 经 `runtime.governed_dispatcher()`（`runtime/mod.rs:542-551`，OnceLock 缓存）得到与 `GovernedToolExecutor` 同一 `Arc<Inner>`。测试 `governed_ask_host_and_runtime_share_same_dispatcher`（`runtime/mod.rs:13605`）验证 `Arc::ptr_eq`。
- **CAS 原子性与 fail-closed**：`consume_control_request`（`dispatcher.rs:1244-1305`）单 Mutex 全程持锁，校验 session/version/nonce/expiry/descriptor snapshot/descriptor id/final args digest/policy digest/kind；`ask_control::for_request`（`ask_control.rs:64-76`）保证宿主从不手搓 CAS 事实。
- **Ask≠approval**：`persist_pending_request`（`dispatcher.rs:1206-1215`）按 verdict 派生 `Interaction`/`Approval`；`answer_control_request` 强制 `Interaction`（kind mismatch 拒绝）；`bind_ask_wait_for_request`（`runtime/mod.rs:2327-2329`）只对 Interaction 绑定 graph ask-wait，Approval 不绑定。
- **唯一终态**：`resume_ask_wait` 移除 binding 后产出单条 `terminal_result`（`graph.rs:833-844`），run 回 `Ready`、`resume_count` +1；stale version 拒绝且保留 binding（`graph.rs:818-823`，测试 `ask_wait_resume_rejects_stale_version_and_keeps_the_binding`）。
- **守卫完整性**：`has_unresolved_ask_waits` 同时挡 `begin_turn`（`graph.rs:558`）与 `apply_turn_result`（`graph.rs:590`）；`execute_graph_run_stream`/`advance_graph_run` 的挂起短路（`:1364/:1420`、`:1719/:1770`）均不调 planner 与 `apply_turn_result`，run 保持 `WaitingUser`。
- **`turn:suspended` 终态集合**：`RecordingTurnEventSink::emit`（`control_plane/mod.rs:830-835`）含 `turn:suspended`。
- **流式路径不触发 provider followup**：测试用空 MockHttpServer 断言 `server.finish()` 为空（`runtime/mod.rs:13495/13795`），证明挂起后无 provider 调用。

## 测试证据

- `cargo test -p pony-agent-core --lib -- ask`：**32 passed / 0 failed**（含 `governed_ask_*` sync/stream 挂起+恢复、共享 dispatcher）。
- `cargo test -p pony-agent-core --test dispatcher_matrix`：**27 passed / 0 failed**。
- `cargo test -p pony-agent --test tool_router_regression`：**13 passed / 0 failed**。
- `cargo test -p pony-agent --test session_regression`：**5 passed / 0 failed**。
- `npx openspec validate harden-and-expand-agent-tool-runtime`：**valid**（`--all` 的 33 项失败均为既有其他 spec，非本 change）。

**测试缺口（与 P0-1/P1-1 对应）**
- 无"前端调用 `ask_answer` 后自动 `graph_resume_ask` 并继续 run"的端到端测试——后端测试 `graph_run_stream_ask_suspends_returns_waiting_user_and_resumes_unique_terminal`（`control_plane/mod.rs:8016`）只覆盖后端链路，未覆盖前端接线。
- 无 `turn:suspended` 事件触发前端 refresh 的断言。
- 无 `graph_resume_ask` 对"请求未 Consumed"拒绝的测试。
- 无挂起后 execution checkpoint 语义的断言。

## 结论

**FAIL（P0 阻塞）。**

后端（dispatcher CAS、graph Ask wait 绑定/恢复、sync+stream 挂起短路、共享 dispatcher、`turn:suspended` 终态）实现与测试均扎实，可判定为高质量。但 **P0-1（Ask 前端 resume 断链）使 task 4.4 明确要求的 "resume Ask requests" 未完成**：用户回答 Ask 后 run 永久卡在 `WaitingUser`，且即使补调 `graph_resume_ask`，设计 Decision 5 的"注入唯一终态"也仅停留在产出 JSON blob、无注入 provider turn 的路径。P1-1（`turn:suspended` 事件前端未消费）叠加轮询自终止，挂起后的 Ask 甚至可能不显示。P1-2（`graph_resume_ask` 绕过 CAS 消费校验）与 P1-3（生产引擎 Write/Edit 全 Allow，phase-3 保守默认被覆盖）需在本里程碑做出明确修复或记录决策。

复测通过条件（P0/P1 清零后）：
1. 前端 `ask_answer` 成功 → `graph_resume_ask` → run 恢复可继续；终态注入语义落地并测试。
2. `turn:suspended` 触发前端 Ask 刷新。
3. `graph_resume_ask` 对未 Consumed 请求 fail closed。
4. Write/Edit 审批语义或显式接受记录。
