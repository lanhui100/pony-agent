# Agent Team 无回执 / 追问断流 / TUI 无团队视图 —— 只读代码审计

- 审计对象：`/home/dm/pony-agent`，会话 `session-b1e4d476`（标题「为什么当前项目tui退出后…dev team诊断并修复」）
- 审计方式：只读（未修改任何仓库文件、未执行 git 写、未跑全量测试）
- 结论等级：**A/B 两项均有确定性物理证据（DB trace + 运行期全量日志）**，C 为代码能力盘点，D 为变更一致性核对
- 证据源（均可复现）：
  1. `~/.local/share/PonyAgent/sessions.db`（只读副本 `/tmp/audit_sessions.db`）的 `turn_events` / `sessions` / `normalized_messages`
  2. `~/.pony-agent/tui-logs/tui-runtime.log`（该会话的完整运行期日志，12 MB）
  3. 源码 `crates/pony-agent-core/**`（行号以当前工作树为准，含未提交改动）

---

## 0. 一句话结论（TL;DR）

| 编号 | 现象 | 真实根因（一句话） |
|---|---|---|
| A | tester/reviewer 永远不回信息 | **驱动循环真实存在且真的跑起来了**；但队友 turn 在 `HostTurnEngine` 里用**另一套 runtime**（未继承父会话 provider），落到默认 `openai/gpt-4.1-mini`，立即 `missing API key` 失败；错误回执被投进 **lead 收件箱**，而 lead 收件箱**没有任何消费者**把内容注回 Lead 的对话/UI → 表现为「石沉大海」 |
| B | 追问「检查tester和reviewer的进展」「继续」直接崩掉 | `LocalTurnPlanner::preflight_decision` **抢在模型调用之前**用启发式造了一个工具调用；历史里 assistant 那句「tester **/** reviewer 仍在异步收敛」中的 ` / ` 被当成路径 `/`，于是生成 `Read{path:"/"}`；`/` 在工作区外 → 治理层判 `ApprovalRequired` → 落 `PendingControlRequest` → 但 runtime **匹配不到**该请求（`pending_request=None`）→ 只发一个空 `turn:suspended`，无 assistant 正文、无审批弹窗、无恢复路径 → 用户眼里就是「没反应、断流」 |
| C | TUI 没有 agent team 视图 | 11 个斜杠命令里无 `/agents`；唯一像样的 `/tasks` 面板其实喂的是 **TodoWrite 的 todo**，不是 `team_task_*` 看板；子代理会话未持久化，无法「进入队友会话」；不过**数据面已具备**（`list_agents/inbox_pending_len/team_task_list/is_driving` 均为同进程 pub API），可直接复用 |
| D | 未提交变更 / ADR 0062 | 变更只涉及终端还原 + `pony resume`，与 A/B/C 结论**不冲突**（未触碰 orchestration/planner）；但 ADR 0062 的「验收」章节引用了**两个不存在**的交付物，`Status: implemented` 属**超范围声明** |

---

## A. tester / reviewer 为什么「始终没有返回信息」

### A.1 结论先行：不是假回执，是「真驱动 + 真失败 + 回执无消费者」

工具回执本身**不是桩**：`spawn_teammate` 走的是 `spawn_teammate_in_workspace`，它在引擎在位时**真的**入队并后台驱动一次队友 turn（`crates/pony-agent-core/src/agent/tools.rs:1679` → `crates/pony-agent-core/src/agent/orchestration.rs:441-463`）。运行期日志证明队友 turn 真的启动了 3 次（tester×2、reviewer×1）：

```
[pony-runtime] turn:run requested=openai provider=openai protocol=openai-completions model=gpt-4.1-mini
              message_preview=You are teammate 'tester'. Incoming messages:\n- 你是 pony-agent 项目的 tester ...
[pony-provider] followup:error provider openai missing API key; please save a key in the model config page (field: OPENAI_API_KEY).
[pony-agent][runtime] emit turn:failed event_type=turn.failed turn=sync:subagent-78192736-... phase=Some("failed")
```
精确行号：`:62119`（turn:run 启动，provider=openai/gpt-4.1-mini）→ `:62122`（首跳 `BatchExecute` 预校验失败 `error.code=child_not_allowed`：子调用 `Read` 解析为 `builtin:workspace_gather_context` 不在只读批量子调用白名单内）→ `:62125`（`followup:error provider openai missing API key`）→ `:62126`（`turn:failed ... text_len=17`）→ `:62127`（`no trace row session=subagent-…`，队友会话未落库）。
第二、三次完全同型：`:108681`+`:108687`（tester 第二次）、`:115250`+`:115256`（reviewer）。
关键观察：**失败的最后一击不是业务错误，而是 provider 认证错误** —— `child_not_allowed` 只是把首跳工具打回给模型（`followup:start mode=sync`，`:62124`），真正致命的是随后那次 follow-up 用了错误的 provider。

### A.2 三个确定性缺陷（file:line）

**(A-1) 子代理 runtime 不继承父会话 provider → 必失败**（决定性根因）
- `crates/pony-agent-core/src/agent/subagent_runtime.rs:216-231`：`HostTurnEngine::run_turn_with_root` 内 `AgentRuntimeBuilder::new()` 新建 runtime，并构造 `TurnInput{ provider_id: None, model_id: None, ... }`（`:222-223`）——**没有** `provider_resolver`。
- 对比宿主（TUI）：`crates/pony-agent-core/src/bin/tui.rs:3149-3156` 用 `AgentRuntimeBuilder::desktop().provider_resolver(DynamicProviderResolver{..})`（ponyllm + 用户所选模型）。
- 结果：父会话 provider = `ponyllm / deepseek/deepseek-v4.1-flash`（日志 `:56706`），子代理 provider = 默认 registry 的 `openai / gpt-4.1-mini`（日志 `:62119`）；`OPENAI_API_KEY` 未配置 → 立即 `followup:error`。
- **「为什么偏偏是 openai / gpt-4.1-mini 且缺 key」的默认解析路径**（三步，均可指认）：
  1. `subagent_runtime.rs:216` 只调 `AgentRuntimeBuilder::new()`（未注入 `provider_resolver`）⇒ builder 在 `crates/pony-agent-core/src/agent/runtime/builder.rs:100-104` 回落 `Box::new(ProviderRegistryStore::new())`；
  2. `ProviderRegistryStore::new()`（`crates/pony-agent-core/src/agent/config.rs:264-270`）读本地 `pony-agent/providers.json`；文件不存在/无选中项时用内置模板 `default_provider_templates()`（`config.rs:1037`）播种，其中 **openai** 项写死 `api_key_env_var: "OPENAI_API_KEY"`、`selected_model_id: model-openai-default` → `model: "gpt-4.1-mini"`（`config.rs:1074-1102`，模型名 `:1079-1086`）；
  3. `TurnInput{provider_id:None, model_id:None}`（`subagent_runtime.rs:222-223`）→ `ProviderRegistryStore::resolve_selection(None, None)`（`config.rs:366-395`）先按 `selected_provider_id`、再按 `storage.providers.first()` 落定 provider，再按 `selected_model_id`/`models.first()` 落定模型 ⇒ `openai` + `gpt-4.1-mini`；密钥解析 `resolve_provider_api_key`（`config.rs:621`）读 `OPENAI_API_KEY`，本机未配置 ⇒ `followup:error`。
  ⇒ 与宿主（ponyllm）**零共享**；`gpt-4.1-mini` 是「内置模板第一位 + 环境变量缺省」的必然结果，不是随机漂移。
- 注意：`spawn_teammate_in_workspace` / `send_message_in_workspace` 确实传了**工作区根**（`orchestration.rs:441-463`、`:549-580`），但 **provider/model 从来不在继承契约里** —— 这是设计缺口，不是偶发环境问题：任何用户的默认 provider 只要不是 openai，队友就永远跑不起来。
- 同一缺陷也解释了 `subagent` 工具（`orchestration.rs:359-410`）在 TUI 里同样不可用。

**(A-2) 失败/成功回执只写进 lead 的**内存**收件箱，没有消费者**
- 失败补偿：`orchestration.rs:946-960`（`requeue_messages` `:948` + `set_teammate_status(Inactive)` `:949` + `send_message(lead, "Teammate '…' turn failed (… message(s) requeued): …")` `:951-957`）。
- 成功回执：`orchestration.rs:982-997`（收口 `:982-988`，随后 `send_message(lead, "Teammate '…' finished turn:\n…")` `:993-997`）。
- 消费侧：`lead` 收件箱**唯一**的读入口是模型主动调用 `drain_inbox` 工具（`tools.rs:1789-1804` → `orchestration.rs:720-754`）。全仓 grep 无第二处 `drain_inbox(` 调用点；
- 该会话的 `turn_events` 里 **143 次 tool/call 中 drain_inbox = 0**（`/tmp/audit_sessions.db` 查询）；日志 `tool-execute.*drain_inbox` 命中数 = 0。
- 而系统提示词的团队策略段（`crates/pony-agent-core/src/agent/context.rs` 的 `AGENT_TEAM_POLICY_PROMPT`，`:57-80`）**通篇没有提到 `drain_inbox`**，只教模型 `list_agents` / `team_task_list` / `send_message` / `interrupt_agent`。⇒ 模型没有任何线索去取回执，UI 也没有入口，回执永久滞留内存。
- 结论：这是「**驱动循环存在、回执链路半截**」，不是「假回执」；用「假回执」描述会误判修复方向（换掉 orchestration 无济于事，真正要修的是 provider 继承 + 回执注入/展示）。

**(A-3) `status=inactive` 不是「没人跑」，`reviewer 缺席` 不是丢失**
- `inactive` 恰好是**引擎真的执行过**的收口：`drive_teammate_turn_in` 在 `engine_was_wired == true` 时置 `Inactive`（`orchestration.rs:975-987`）；无引擎降级路径反而置 `Active`（`:975-982`）。⇒ DB 里 `tester.status=inactive`（trace seq 423）反证「引擎在位 + 驱动跑过 + 失败收口」。
- reviewer「不在列表」是**时序**问题，不是丢失：`list_agents`（seq 422，step 1547）**早于** reviewer 的 `spawn_teammate`（seq 463，step 1642）。此后本会话再无 `list_agents` 调用。附带的次要不一致：`list_agents` 返回 `HashMap::values()`（`orchestration.rs:515-540`，`:533`），顺序不稳定，且无「按角色/状态排序」。

### A.3 其它被问到的 API 的真实状态

| API | 真实实现 | 结论 |
|---|---|---|
| `spawn_teammate`（库层） | `orchestration.rs:431-437` 仅 `register_teammate`，**不投递不驱动**（注释与冻结用例有意如此） | 库层是「纯登记」；模型可见入口用的是 `_in_workspace` 版本（`tools.rs:1679`），会真驱动 |
| `spawn_teammate_in_workspace` | `orchestration.rs:441-463`：登记 → 投递初始 prompt → 后台驱动 | 真实 |
| `list_agents` | `orchestration.rs:515-540`，进程内 `static TEAMMATES`（`:337`） | 真实但只读内存表；进程级、无持久化 |
| `send_message` / `send_message_in_workspace` | `orchestration.rs:542-580`：入队 → 单飞 `begin_driving`（`:134`、`:150`）→ `spawn` 后台 `drive_loop_in`（`:584-609`） | 真实；`delivered:true/mode:queue/pending_inbox_len:1/processed:false` 的语义在 `:105-130` 与 `:611-675`，**明确写了「delivered≠已处理」**，回执本身不撒谎 |
| `drive_loop_in` / `drive_teammate_turn_in` | `orchestration.rs:584-609`、`:894-1000`：`drain → run_turn → 回执 → 再 drain`，唯一退出条件＝收件箱空 | **自主执行循环真实存在**（不是桩），只是它跑的是会失败的 runtime |
| `interrupt_agent` | `orchestration.rs:688-718`，一次性 `INTERRUPTED` 标记（`:164-182`），仅当真有驱动在跑才置标 | 真实，且已修过一版「残留标记」缺陷 |
| `wait_agent` | `orchestration.rs:851-879`：`let _timeout = args.timeout_ms.unwrap_or(30_000);` **完全不等待**，随即返回 | **纯桩**（注释自认「模拟等待返回」）；而且**没有对外暴露为工具**（`tools.rs` 只有 import `:60`，无 `TOOL_WAIT_AGENT` 常量/分发分支）⇒ 模型根本调不到 |
| `team_task_create/update/list` | `orchestration.rs:767-848`、`:1003-1016`，进程内 `static TEAM_TASKS`（`:331`） | 真实；但**没有任何组件会去 claim**（队友失败/不存在），所以任务恒 `pending / owner:null / revision:1` —— 与 trace seq 427、467 完全一致 |
| 是否派生独立会话 | `subagent_runtime.rs:203` 预生成 `subagent-{uuid}` 作为 `session_id`，但同一文件 `:216` 用 `AgentRuntimeBuilder::new()`（独立 `SessionStore`），运行期只打日志「no trace row session=subagent-…」（日志 `:62127` 等） | 队友 turn **不落库、不产生可枚举会话**，因此既不能审计也不能在 UI 里打开 |

### A.4 处置建议（不在本次范围内实施，仅供 Lead 决策）
1. `HostTurnEngine` 必须继承父会话的 `provider_id/model_id`（或注入父 `ProviderResolver`），否则团队能力在任何非 openai 环境下恒为假。（`subagent_runtime.rs:216-231` + `:37-60` trait 需要新增「父 provider 快照」字段）
2. lead 收件箱需要有**消费者**：要么 turn 开始前自动 drain 并作为 system/developer 消息注入，要么给 TUI 一个 inbox 面板（C 节已给出数据面）。
3. 系统提示词的团队策略段必须写明 `drain_inbox` 是唯一的回执读取方式（`context.rs:63-77`）。
4. 队友 turn 应落库为可枚举会话（当前 `AgentRuntimeBuilder::new()` 导致孤儿）。

---

## B. 追问为什么「跳过模型调用直接断流」

### B.1 `Read {path:"/"}` 是谁生成的：**本地启发式 planner，不是模型**

- 文案唯一定义点：`crates/pony-agent-core/src/agent/planner.rs:391`
  `"description": format!("收集路径 `{}` 的上下文。", path)`（`lineCount:80, limit:40` 见 `:392-394`）
  —— 与 trace 里的参数**逐字一致**：`{"description":"收集路径 `/` 的上下文。","limit":40,"lineCount":80,"path":"/"}`。
- 该分支是 `infer_local_tool_call` 的**兜底无条件分支**：`planner.rs:352-399`（`if let Some(path) = referenced_path { ... return Some(ToolCall{ workspace_gather_context }) }`）。也就是说：**只要历史里能猜出任意路径，任何一句不含其它关键词的追问都会被翻译成「读那个路径」**。
- `referenced_path` 来源：`planner.rs:236` `explicit_path.or_else(|| Self::infer_last_referenced_path(history))`；`infer_last_referenced_path`（`:535-540`）**从最后一条历史消息里取第一个"像路径"的 token**。
- 「像路径」判定极宽：`looks_like_path`（`:791-813`）——`segment.contains('/')` 即为真（`:802`）⇒ 单字符 `"/"` 命中。
- `/` 的真实出处：该会话最后一条 assistant 消息首行是
  `## 修复已完成（tester / reviewer 仍在异步收敛）`
  （`normalized_messages` message_id=`msg-1-assistant`）；`extract_path_candidates`（`:576-597`）按空白切词，`tester`/`reviewer` 因不含 `.`/`/` 被过滤（`:802-812`），**` / ` 分隔符本身成为候选 `/`** → 它就是「最后一条历史里的第一个合法路径」。
- 两次追问均如此（无路径的 `继续` 也一样）：这就是「同一 bug 稳定复现」的原因。

### B.2 为什么跳过模型调用

- `crates/pony-agent-core/src/agent/runtime/turn_prep.rs:163-207`：
  `preflight_decision = self.planner.preflight_decision(...)`（`:164`）；随后**只有在非 native tool flow 时才用真实 provider**（`:191-207`）。若 `preflight_decision` 为 `Some`，**直接把它当作首跳决策**（`:203-205` 的 `Some(decision) => (decision, None)`），`provider_decision_with_mode` 根本不被调用。
- `LocalTurnPlanner::preflight_decision`（`planner.rs:143-151` → `:501-518`）会把 `infer_local_tool_call` 的结果包成 `ProviderDecision{ provider_source:"local_planner", provider_mode:"tool-first" }`。
- 日志侧证据（两条追问完全同型，`tui-runtime.log` 尾部）：
  ```
  turn:start ... message_preview=检查tester和reviewer的进展
  emit turn:started  ... phase=Some("calling_model")  text_len=32   <- 注意：无 model_call_started / 无 request:openai
  emit turn:tool_call_started  capability_id=builtin:workspace_gather_context
  emit turn:tool_call_completed
  emit turn:suspended          seq=6 phase=suspended             <- 最后一帧，无 assistant 输出
  ```
  （`:118858-118865` 与 `:118867-118874`）；DB `turn_events` 亦只有 6 条、无 `turn/end`。

### B.3 `control_outcome_pending` 的产生 → 消费链路（含断点）

**产生**（工具结果变成「待控制应答」）：
1. 治理层路径门禁：`crates/pony-agent-core/src/agent/governed_executor.rs:236-283` —— 读类工具（含 `workspace_gather_context`/`Read`，`:249-258`）若目标路径在工作区外且无授权，`PathPermissionChecker::classify` 返回 `RequiresAuthorization` → `PermissionVerdict::ApprovalRequired`（`:269-277`）。
   `path:"/"` 是绝对的系统根，必然在工作区外（工作区 `PathBuf::from(ws_root)`，`:262-265`）。
2. 派发器落 `PendingControlRequest`：`crates/pony-agent-core/src/agent/dispatcher.rs:1248-1330`（`persist_pending_request`）；`ApprovalRequired → ToolControlKind::ApprovalRequired`（`:1261-1266`）、`request_kind = Approval`（`:1299`）；返回 `ToolOutcome::pending(...)`（`:1330`）。
3. 旧契约适配层把 pending 降级成**不可消费的错误**：`crates/pony-agent-core/src/agent/tools.rs:963-1014`，尤其 `:997-1013`（`into_legacy_result` → `status:"error"` + `code:"control_outcome_pending"` + 文案 `:1008`「工具调用尚处于控制请求状态，尚未产生可供 provider 消费的终态结果。」）—— 与 trace 的 `tool_result` 逐字一致。

**消费**（本应挂起并弹审批）：
4. 流式路径：`crates/pony-agent-core/src/agent/runtime/turn_stream.rs:545-604` → `tool_result_control_outcome_pending`（`:575-590`，另有 `stream_support.rs:575-590`）→ `suspend_turn_for_control_outcome`（`turn_control.rs:222-249`）→ 收集 `pending_request` → 若 `Some` 才写 `turn:ask-suspend-stream` 日志（`:549-559`）并带 request 信息发 `turn:suspended`（`:581-604`）。
5. TUI：`crates/pony-agent-core/src/bin/tui.rs:2945-2949`（`turn:suspended → UiEvent::TurnSuspended{run_id:None}`）→ `:8161-8166`（置 `is_streaming=false`、状态文案「等待用户输入确认...」+ 立即 `check_pending_asks`）→ `:6708-6736`（500ms 轮询 `cp.list_pending_asks(Some(session_id))`，命中则 `start_permission_approval`）。

**断点（本次两次追问真正卡住的地方）**：
- 日志中这两轮**没有** `turn:ask-suspend-stream hop=…` 行 —— 而其它轮次有（同一日志 `:49645`、`:51336`）。该行只在 `suspension.pending_request` 为 `Some` 时打印（`turn_stream.rs:549-559`）。
  ⇒ **`pending_request == None`**，即「请求已落盘，但 runtime 匹配不回来」。
- 匹配函数：`crates/pony-agent-core/src/agent/runtime/stream_support.rs:657-675`。
  - 第一分支要求 `request.call_id == tool_call.call_id`（`:661-666`）；
  - 兜底分支**只接受 `Interaction`（Ask）**（`:667-675`），`Approval` 被结构性排除。
  - 而本次 planner 造出的 `ToolCall` 的 `call_id` 是 `None`（`planner.rs` 内所有构造点，如 `:383-398`；旁证：trace 的 `call_id` 被 telemetry 兜底成 `tool-Read`，见 `crates/pony-agent-core/src/agent/telemetry.rs:395-407`，`:403-405`）。
  ⇒ **Approval 类挂起对 planner/任何无 call_id 的调用是"不可达"的**：必然 `None` → 不弹窗、不打印、只发一帧无载荷的 `turn:suspended`。
- 连带后果：
  - `TurnSuspended{run_id: None}`（`tui.rs:2948`）到位后，`check_pending_asks` 用 `ask_dispatcher` 列表过滤 session（`crates/pony-agent-core/src/agent/control_plane/ask_plan_commands.rs:20-32`）；即便列表能看到该 Approval，TUI 的恢复分支也只为 `Interaction` 绑 `graph_run`（`turn_control.rs:270-286` 注释与实现，`Approval` 直接 `return None`），无「批准后重放该工具」的实现，**没有闭环恢复路径**。
  - turn 就此终止：无 assistant 正文、无错误提示、无重试、`turn:end` 都不产生（DB trace 仅 6 条事件）——用户看到的就是「我追问了，它像没听见」。

### B.4 缺陷清单（B 项）
| # | 位置 | 缺陷 |
|---|---|---|
| B-1 | `planner.rs:352-399`（兜底分支）+ `:236` + `:535-540` | 无关键词的追问被无条件改写成「读历史里猜到的路径」，且路径从**任意历史文本**（含 assistant 正文里的 ` / `）猜 |
| B-2 | `planner.rs:791-813`（`:802`） | `looks_like_path` 把单字符 `/` 判为合法路径 |
| B-3 | `planner.rs:143-151` / `:501-518` + `turn_prep.rs:163-207` | preflight 本地决策**先于/替代**模型调用，一旦启发式跑偏就永久跳过 LLM（用户无感、无兜底） |
| B-4 | `stream_support.rs:661-675` | 无 `call_id` 的调用无法匹配 `Approval` 类挂起（只兜底 `Interaction`）→ 审批请求成为孤儿 |
| B-5 | `turn_stream.rs:545-604` + `turn_control.rs:270-286` | `pending_request=None` 时仍发 `turn:suspended`，但既不带载荷也无「无法匹配」告警；`Approval` 无恢复实现 |
| B-6 | `tools.rs:997-1013`（`:1008`） | pending 在 legacy 契约里被伪装成普通 `error`，`mcp/cli/TUI` 侧若不走 stream 挂起分支就只能看到「报错」（fail-closed 是对的，但缺「已挂起」的可见语义） |

### B.5 与 Lead 假说的差异（明确分叉点）

Lead 假说：`path:"/"` 来自 Developer 层 `Session label: graph=state-machine-v1 / session=...` 里那条裸 `/`，经 `extract_path_candidates` + `infer_last_referenced_path` 命中。
**我的结论：来源是 assistant 最后一条正文「（tester / reviewer 仍在异步收敛）」里的 ` / `，不是 Developer label。** 依据：

1. 该 Developer 标签确实存在（日志 `:11`、`:26`、`:131` 形如 `Developer:Session label: graph=state-machine-v1 / session=...`），但它是**每轮动态注入的 provider 层消息**，不进入 `planner_history`：
   - `planner_history()` 只返回 `session_context.recent_history`（`crates/pony-agent-core/src/agent/context.rs:221-223`）；
   - `recent_history = trim_turn_history_to_turn_boundary(query.session.history.clone())`（`context.rs:813`），而 `session.history` 的 role 只有 `user` / `assistant`（`crates/pony-agent-core/src/agent/session/store.rs:450`、`:456`、`:591`、`:597`）。
2. 该会话最后一条历史正是 assistant 正文，且**首行**就含 ` / `：`normalized_messages` `message_id=msg-1-assistant`，内容 `## 修复已完成（tester / reviewer 仍在异步收敛）…`。`infer_last_referenced_path` 取「最后一条含路径的消息」的第一个路径 token ⇒ ` / ` 的 `/`。
3. 两条追问（`检查tester和reviewer的进展`、`继续`）都发生在该 assistant 消息之后，两轮 trace 完全同型，均可由「最后一条历史 = 该 assistant 消息」解释。
4. 若真来自 Developer label，则**每一次**追问都会同样断流（label 每轮都在）；观察到的两次只是该会话仅有的两次追问，样本不足以区分——但代码路径（`session.history` 只含 user/assistant）足以判定 label 不在 `planner_history` 里。

> 说明：无论来源是 ` / ` 还是裸 label `/`，**修复面相同**（B-2 收紧 `looks_like_path`、B-1 给兜底分支加意图门、B-3 preflight 只在明确意图时短路）。但归因影响「回归测试怎么写」：应当用 `history=[assistant("…（tester / reviewer 仍在异步收敛）")] + user("继续")` 作为固定夹具。

### B.6 可复现最小断言（供修复后做 red→green，本条只给断言设计，不落码）

1. **planner 归因（冻结当前缺陷行为）**：`LocalTurnPlanner::infer_local_tool_call("继续", &[history("assistant", "## 修复已完成（tester / reviewer 仍在异步收敛）")])`
   → 现状：`Some(call)`，`call.name == "Read"`，`call.arguments.path == "/"`，`call.arguments.limit == 40`，`description == "收集路径 `/` 的上下文。"`。
   参照既有同型用例：`planner.rs:981-999`（`planner_can_follow_last_referenced_path_from_history`，用 `history("user","先看看 src-tauri/src/agent/planner.rs")` + `"这个文件是什么？"` 断言落到 `Read`）——**该用例本身就在冻结「从历史猜路径」这条契约**，修复时必须同步改写。
2. **修复后的期望（green）**：同一夹具应返回 `None`（无明确意图不造工具调用），或至少不得把单字符 `/` 当路径；`looks_like_path("/") == false` 的最小断言直接押在 `planner.rs:791-813`。
3. **挂起可观测性（防回归）**：任何 `control_outcome_pending` 轮次必须满足——要么日志出现 `turn:ask-suspend-stream …`（`turn_stream.rs:549-559`），要么 `turn:suspended` 事件携带非空 `request_id`；当前两轮两者皆无（日志 `:118864`、`:118873` 只有裸 `turn:suspended`）⇒ 断言「不允许出现既无 ask-suspend 日志又无 request_id 的 suspended」。
4. **审批可恢复**：对 `ApprovalRequired` 的 pending 请求，`tui.rs` 的 `check_pending_asks` 命中后必须能打开审批弹窗并在批准后重放该工具（现状 `turn_control.rs:270-286` 对 `Approval` 直接 `return None`，无重放实现）⇒ 断言：批准后同一 `call_id` 的工具被执行且产生 provider 可消费结果。

---

## C. TUI 的 agent team 能力盘点

### C.1 现状矩阵

| 能力 | 有无 | 证据 |
|---|---|---|
| 角色 roster（成员/状态） | **无** | `AVAILABLE_COMMANDS` 共 11 条（`tui.rs:715-728`），无 `/agents`；全文件 `team` 仅 3 次命中（`:721` 命令说明、`:5834`/`:5853` 面板标题），无任何 `Teammate/list_agents` 渲染代码 |
| 会话/状态展示（队友 running/inactive） | **无** | 无 `TeammateStatus` 引用（`grep Teammate` 零命中） |
| Team 任务看板 | **有，但名不副实（接的是 todo，不是团队看板）** | 见下方 C.1.1 完整证据链 |
| 进入子 agent / teammate 会话 | **无** | `/resume`（`:718`）只列 `SessionStore::list_sessions()`（`api` 见 `resolve_resume_target` `:8223-8240`）；队友 turn 用独立 runtime 且不落库（`subagent_runtime.rs:216`，日志「no trace row session=subagent-…」）⇒ 无可进入的会话对象 |
| 返回主会话 | 部分 | 会话级有（`/resume`、`reset_session_scope` `:1679`）；teammate 层级**不存在**，因为从来进不去 |
| lead 收件箱 / 回执展示 | **无** | 全文件无 `inbox`/`drain_inbox` 字样 |

### C.1.1 「名不副实」完整证据链（`/tasks` ≠ Team 任务看板）

1. **命令文案**：`tui.rs:721` `("/tasks", "切换/查看 Team 任务看板（Ctrl+T）")`；面板标题也写 `Team 共享任务看板`（`tui.rs:5834`、`:5853`）。
2. **真实数据源是 todo store，不是 `team_task_*`**：
   - `tui.rs:482-506` `pub fn sync_tasks_from_store(state)` → 调 `pony_agent_core::agent::todo::todo_list(...)`（`:486-489`）；
   - `tui.rs:462-481` `tui_tasks_from_todos(todos: Vec<TodoItem>) -> Vec<TuiTask>`，且把团队字段**写死为空**：`owner: None`（`:474`）、`blocked_by: Vec::new()`（`:475`）、`write_scopes: Vec::new()`（`:476`）；
   - 调用点只有三处且都在「todo 列表变化」时机：`tui.rs:7567`、`:7721`、`:7988`。
   - 面板空态文案自认：`tui.rs:5834-5844`「当前没有任务；模型调用 todo 工具后会实时汇总到这里」。
3. **连「团队」这条事件通道都是死代码**：`UiEvent::TaskBoardUpdated(Vec<TuiTask>)` 在 `tui.rs:352` 声明、`:8155` 消费，但**全 crate 无任何发送点**（`grep TaskBoardUpdated` 仅这两处命中）。
4. **团队任务看板的真实数据面从未被 TUI 读取**：`team_task_list()`（`orchestration.rs:1003-1016`）在 `crates/pony-agent-core/src/bin/tui.rs` 中**零命中**；`list_agents` / `spawn_teammate` / `teammate` / `Teammate` 在该文件里 **0 次命中**（实测 `grep -c` = 0）。
   ⇒ 该会话里 `team_task_create` 建的两条任务（trace seq 156/466）在 TUI 上**永远不可见**，用户看到的「Team 看板」只反映 Lead 自己的 TODO。

### C.2 可直接复用的数据面（无需新造存储）
- `pony_agent_core::agent::orchestration::list_agents()`（`orchestration.rs:515-540`）→ `Vec<TeammateMember{target,name,role,status,description}>`（`:54-62`）
- `inbox_pending_len(target)`（`:756-765`）→ 只读未消费条数（非破坏性，专为 UI/观测设计）
- `is_driving(target)`（`:184-194`）→ 是否有驱动在跑
- `team_task_list()`（`:1003-1016`）→ `TeamTaskItem{subject,status,owner,revision,write_scopes,blocked_by}`（`:222-238`）
- `drain_inbox(target)`（`:720-754`）→ 若做 UI 面板，应**只读展示**用 `inbox_pending_len` + 需要时另开只读 peek，否则会抢走驱动循环应消费的消息（该风险在 `:747-755` 注释里明说）
- 关键便利：TUI 与 orchestration 同进程（`static TEAMMATES` 等进程级），上述函数**直接可调**，无需 IPC。
- 与桌面端一致性参照：`TuiState.session_list`（`tui.rs:1454`、`:2152`）已有「列表 + 选中索引 + 进入」的成熟模式，roster 视图可照抄。

### C.3 建议的最小实现面（供 Lead 参考）
`/agents` 命令 + `InteractionMode::AgentRoster` 面板：`list_agents()` 渲染 name/role/status/pending_len/is_driving，`team_task_list()` 渲染 owner/revision；Enter 打开该成员收件箱（只读 peek）。**不需要**改 orchestration，纯展示层。

---

## D. 未提交变更与 ADR 0062 一致性核对

### D.1 `git status`（只读，未做任何写操作）
```
 M crates/pony-agent-core/src/bin/tui.rs        (+478 / -188, 665 行改动)
 M docs/decisions/README.md                     (+1 索引行)
?? crates/pony-agent-core/tests/tui_terminal_restore.rs   (113 行)
?? docs/decisions/0062-tui-exit-restore-and-resume-entry.md
```

### D.2 与 A/B/C 结论的关系
- `git diff` 内 **零命中** `team|teammate|orchestration`（实测 `grep -in` 无输出）⇒ 本次变更**没有**触碰 A 的 provider 继承缺陷、也没有触碰 B 的 planner/挂起链路。**结论与变更互不冲突，也互不掩盖**：TUI 退出还原修好了，团队与追问两条链路依旧原样带缺陷。
- 变更内容与 ADR 一致的部分（代码实测存在）：`resume_hint_line`（`tui.rs:229`）、`CliAction::ResumeTui`（`:8184`，解析 `:8196-8200`）、`resolve_resume_target`（`:8223`）、main 分发（`:8487`）、`TERMINAL_INPUT_MODES_RESET/TERMINAL_SCREEN_RESET`（测试断言引用）、`hydrate_state_from_session` 抽取。

### D.3 ADR 0062 的**不一致点**（需 Lead 裁决，本次不改）
1. **验收物缺失**：ADR「影响/验收」章节声明
   `crates/pony-agent-core/tests/tui_resume_cli.rs` 与 `tools/tui-audit/check_exit_resume.py` 为验收依据，
   但两者在本工作区**均不存在**（`ls` 报 No such file），`.dev-team/wave-exit-resume-red.md / green.md` 也**不存在** —— 它们本该由 tester 产出，而 tester 因 A 的缺陷从未产出（A 的直接后果之一）。
2. **`Status: implemented` 属超前**：现存唯一测试是 Lead 自写的 `crates/pony-agent-core/tests/tui_terminal_restore.rs`（113 行，仅断言还原串组成/入口形态），**没有** resume CLI 解析的机器收据，也没有真实 PTY 的 `check_exit_resume.py` 收据。按本仓「物理凭证先决」纪律，建议降级为 `proposed`/`partially implemented`，或补齐上述两件验收物后再置 implemented。
3. 语义小差异（非阻塞）：ADR 决策 5 写「缺省取最近一次会话」；`resolve_resume_target(None)` 在空库时返回 `Err("当前没有可恢复的会话…")`（`tui.rs:8235-8239`）——与决策 5 自洽，但与 Lead 在 `send_message` 中给 tester 的口径「空库时退出码 2」未被任何代码/收据验证（该断言建议在 `tui_resume_cli.rs` 里补测）。

---

## 附录 1：证据复现命令（只读）
```bash
# 1) 会话事件（DB 只读副本，避免碰运行中的 WAL）
cp ~/.local/share/PonyAgent/sessions.db /tmp/audit_sessions.db   # + -wal/-shm（如需）
python3 - <<'EOF'
import sqlite3
c=sqlite3.connect('/tmp/audit_sessions.db'); c.row_factory=sqlite3.Row
sid='session-b1e4d476'
for r in c.execute("select turn_id,event_type,seq,payload from turn_events where session_id=? order by turn_id,seq",(sid,)):
    print(r['turn_id'][:20], r['seq'], r['event_type'], r['payload'][:120])
EOF

# 2) 运行期日志：队友 turn 真实启动 + 真实失败
grep -n "You are teammate" ~/.pony-agent/tui-logs/tui-runtime.log
grep -n "missing API key" ~/.pony-agent/tui-logs/tui-runtime.log

# 3) 追问两轮没有模型调用、直接挂起
grep -n "turn-1c5533b9\|turn-829bd610\|ask-suspend" ~/.pony-agent/tui-logs/tui-runtime.log
```

## 附录 2：关键事实核对表（对用户/Lead 已核实证据的确认或修正）

| 用户给出的证据 | 审计判定 |
|---|---|
| spawn_teammate ok，member=tester/active | ✅ 确认（trace seq 160-161）；**且它是真驱动**，不是假回执 |
| 随后 list_agents 只有 tester(inactive)+Lead，reviewer 缺席 | ✅ 确认，**但修正归因**：`list_agents` 早于 reviewer spawn（seq 422 vs 463），不是丢失；`inactive` 是「引擎跑过并失败收口」 |
| send_message delivered:true / driving / queue / len 1 / processed:false | ✅ 确认（seq 432-433），且字段语义与代码注释一致（`orchestration.rs:105-130`），**不是撒谎的回执** |
| team_task 恒 pending / owner null / revision 1 | ✅ 确认（seq 427、467）；根因＝没有任何 agent 会去 claim（队友从未活到 claim） |
| 追问 turn 在 step 开始后无 call_model，直接 Read{path:"/"} + control_outcome_pending，然后无输出结束 | ✅ 完全确认，并补充机制：preflight 本地决策跳过模型（`turn_prep.rs:164/203-205`）、路径来自历史里的 ` / `（`planner.rs:236/535/802`）、挂起请求匹配失败（`stream_support.rs:661-675`，日志缺 `ask-suspend-stream`） |
| 第一轮 trace step-call-tool=error；assistant 最后说「tester / reviewer 仍在异步收敛」 | ✅ 确认；**该句正是下一轮 `/` 路径的输入来源**（闭环因果） |
| Lead 补充：队友首跳 `child_not_allowed`、随后 `missing OPENAI_API_KEY`、`text_len=17` | ✅ 全部复核一致（日志 `:62122`/`:62125`/`:62126`）；**补充判定**：`child_not_allowed` 只把工具错误回喂模型（followup `:62124`），致死原因是紧接着的 follow-up 用了 openai provider |
| Lead 假说：`/` 来自 Developer `Session label: graph=state-machine-v1 / session=` | ⚠️ **不一致**：label 不在 `planner_history`（`context.rs:221-223`、`:813`；`session.history` 仅 user/assistant，`session/store.rs:450/456`）。实际来源是 assistant 正文里的 ` / `（详见 B.5）；修复面相同，夹具与归因不同 |

## 附录 3：未做/未验证事项（诚实边界）
- 未执行 `cargo check -p pony-agent-core`：本次审计无需编译变更即可定论（依据为运行期日志 + 现有 trace），且会话日志显示 Lead 侧 `cargo check/build` 均已通过（`tui-runtime.log:62114` 前后与 `check` 阶段日志）。如需可另行补跑。
- 队友 turn 的 `subagent-*` 会话是否写入独立 db 未穷尽（日志明确「no trace row session=subagent-…」，且 `sessions` 表无 `subagent-*` 行），但**不影响 A 的结论**。
- `Approval` 挂起在桌面端（非 TUI）是否可用未验证；本次只对 TUI 链路下结论。
