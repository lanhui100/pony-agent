# 中断轮次续跑上下文丢失修复 —— 冻结契约矩阵（wave: interrupted-turn-resume）

> 事故会话：`pa_01a1ce16-cf4e-72d0-a92b-ae9857aac737`（TUI）。基线分支 `fix/interrupted-turn-resume`，基线 commit `a7393954`。
> 工作树：`/home/dm/pony-agent-worktrees/interrupted-turn-resume`（主工作树正被其他会话改动，本波**一律在本 worktree 内**读写）。
> 构建约定：`export CARGO_TARGET_DIR=/home/dm/pony-agent/target`（复用依赖产物；禁在主工作树写文件）。

## 0. 事实（Lead + 2 路 Explorer 只读取证；`path:line` 以基线 `a7393954` 为准）

| 编号 | 结论 | 证据 |
|---|---|---|
| F1 | 中断键（Ctrl+C 单按 / Esc 双击）只发取消：`stop_turn` 取消令牌 + `turn:stop_requested`，**不落库、不 join**。（行号已复核修正） | `crates/pony-agent-core/src/bin/tui.rs:8873-8899`（Ctrl+C 分支，`stop_turn` 于 `:8890-8893`）、`:9516-9547`（流式态 Esc 双击，`stop_turn` 于 `:9540-9543`）；`agent/control_plane/mod.rs:2336-2380` |
| F2 | 落库只发生在轮收口：`persist_turn_outcome` 先 `flush_session_buffered_events`，再 `append_turn_with_kind`。（行号修正） | `agent/runtime/turn_persist.rs:87-131`（flush `:108-115`、append `:116-131`） |
| F3 | 收口时 assistant 正文被替换成检查点文本（`[中断轮次检查点] 已执行的工具步骤：…`），**已交付正文与工具往返不构成正文**。 | `agent/runtime/turn_persist.rs:744-780`（`persist_cancelled_turn_outcome`，检查点构造 `:760-764`；失败轮同构 `:719-741`）；`agent/session/store.rs:90-122`（`interrupted_turn_checkpoint_text`，`CHECKPOINT_TOOL_LINE_LIMIT=20` 于 `:77`） |
| F4 | 取消轮确有一条 native transcript 通道（已闭合 hop 时写入），但**续跑请求实际走的是 history carry**：实库 `build_context_observations` `bco:turn-040d93d2-…:207` = requestFormat `normalized-input`、12 条消息、**3×**`Previous assistant reply`（**修正**：原写 4×，4× 出自下一轮 `bco:turn-54d1cf25-…:441`；同族 6 轮观测依次为 0/1/2/3/4/5×，与历史轮数同步增长）。`conversationCarryMode=full_replay`。 | `agent/runtime/tool_recovery.rs:573-584`（构建）；`agent/runtime/turn_prep.rs:87-119`（retrieve→build_request）；`agent/context.rs:822-847`、`:1704-1721`（history→`Previous assistant reply` developer 消息）；`agent/provider/mod.rs:256-269`（`native_messages` 为空 ⇒ `requestFormat="normalized-input"`）。注：`observation_ref` 的 `:207` 是**事件日志 seq**（`agent/turn_event.rs:166`、`agent/sqlite_session.rs:1629`），不是 step。 |
| F5 | **已闭合（原 `[UNVERIFIED]` 撤销）**：598 KB transcript 未被使用**不是**对齐失败——`align_native_transcript_with_history` 五条分支全部返回 native（见 O2）；真因是请求在组装期选了**非 native 分支**：`should_use_native_tool_flow()` 为 false。剩余缺口见 §5 O2 末段（`[UNVERIFIED-候选]`，需 executor 二段侦察）。 | `agent/context_window.rs:219-292`（对齐无 fail-open）、`:295-318`（`should_use_native_tool_flow`）、`:14-22`（`has_native_tool_interactions`）；`agent/context.rs:555`、`:672-675`（`full_replay` 分支）；`agent/provider/mod.rs:1149-1152`（本事故模型 `supportsReasoning=false` ⇒ requires=false） |
| F6 | 收口 append 触发**隐式 fork**：`prepare_session_for_new_turn` 在 `visible_node_id != branch_head_node_id` 时建 `…-branch-2` 并把 cursor 置 `HistoricalDirty`。事故库中 branch-2 `base_node_id = forked_from_node_id = node-…314209-4`（该节点 `kind=turn_cancelled`），`forked_from_branch_id=branch-main`。**分离来源已定位**（见 §5 O3）：fork 前一步的 `ensure_history_graph` 把 `branch_head` 回退到「最后一个非 `TurnCancelled` 节点」。 | `agent/session/store.rs:3593-3635`（判据 `:3600`、建分支 `:3617-3629`、`HistoricalDirty` `:3634`）、`:703-704`（append 前 `ensure_history_graph` → `prepare_session_for_new_turn` 调用序）、`:889`、`:4299-4301`（`new_history_branch_id`）、`:3531-3536`/`:3554-3558`/`:3577-3580`（head 回退）；事故库 `normalized_history_branches`/`normalized_history_nodes` |
| F7 | 节点 `turn_id` 取自 `turn_trace_history.last()` ⇒ 事故库 **两个**节点被错标成同一上一轮 `turn-2ca51165`：`node-…314209-4`（`turn_cancelled`，实为 `turn-77ff7a3a`）与 `node-…192389-5`（实为 `turn-040d93d2`）；且 `session_turn_traces` 中**没有** `turn-77ff7a3a` 行（中断轮 trace 未入会话 trace 表），故 `last()` 连续两次停在 `turn-2ca51165`。 | `agent/session/store.rs:3637-3716`（`commit_history_node_from_live_state`，`turn_id` 于 `:3675-3678`）；主路径 `append_turn_with_kind` 不写 trace（`agent/runtime/turn_persist.rs:116-131`）；事故库 `normalized_history_nodes`、`session_turn_traces` |
| F8 | DSML 协议原文在 **SSE 解析层**被折叠：逐行删含 `|dsml|` 的行 + 补一次占位符，块内纯参数值行残留 ⇒ 折叠结果**写回** assistant/chunk、assistant/message、normalized_messages（事故库该步正文 = 75 字符占位串）。 | `agent/provider/openai_sse.rs:115-117`、`:219`、`:369-371`、`:421`；`agent/provider/mod.rs:5360-5391`（`is_dsml_protocol_line` `:5362-5364`、`sanitize_dsml_protocol_text` `:5366-5391`、占位 `:5383-5385`）；写回链 `agent/control_plane/mod.rs:1185-1214`（chunk 聚合→终态批提交；`commit_event_batch` `:1342`、`flush_events` `:1392`） |
| F9 | 续跑请求的上下文= `session.history` carry；事件折叠（`projection.rs`）只对 `event_seq_range` 非空的"引用化节点"生效，且 **tool/call 只记 name+description（合成检查点行）、tool/result 被丢弃**（`HistoryProjectionState::apply` 无 `ToolResult` 臂）。 | `agent/context.rs:822-847`；`agent/projection.rs:132-320`（`ToolCall` 臂 `:174-199`，`ToolResult` 无臂）；`agent/session/store.rs:2541-2556`（`fold_node_views` 仅 `event_seq_range` 非空时折叠） |
| F10 | 既有覆盖面只到"检查点文本可见/可续"，不含"工具往返进上下文"。 | `crates/pony-agent-core/tests/tui_session_resilience_acceptance.rs:162-185`（c5）、`:186-211`（c6）、`:318-391`（at2）；`.dev-team/contract-matrix-tui-session-resilience.md:42-43,56-57`；`[UNVERIFIED] U2` 于 `:24`（保留） |
| F11 | 回归基线：`cargo test -p pony-agent-core --lib` → **1518 passed / 0 failed**，795.84s（主工作树，`a7393954` 前一次提交附近；作为行为对照组）。**`[UNVERIFIED]`**：`.dev-team/regression/baseline-lib.log` 在本 worktree **不存在**（`ls .dev-team/regression/` 空），本 Agent 无法复核该物理凭证。 | `.dev-team/regression/baseline-lib.log`（Lead 跑，Exit 0）— 文件缺失，需 Lead 补凭证或改引主工作树日志 |

### 0.1 deepseek-harness（DSH）对照——本仓要迁移的设计原则

| 编号 | DSH 事实 | 证据（DSH 仓） |
|---|---|---|
| R1 | 会话是 append-only 事件日志，模型历史是**纯函数投影**（`deriveMessages()`）；核心无任何代码按 `turn/end.reason` 过滤模型消息。 | `packages/core/session/src/types.ts:277`、`index.ts:984`、`surface.ts:56-61` |
| R2 | 中断即落盘：已交付正文/reasoning 写成 `assistant/message{interrupted:true}`，tool_call 整块丢弃；空前缀降级为 `assistant/attempt`。 | `packages/core/agent-loop/src/agent.ts:464-490` |
| R3 | 补齐优先于截断：`openTurnClosers` 对未闭合 tool_call 补 **error tool/result（TOOL_OUTCOME_UNKNOWN / TOOL_NOT_STARTED）→ step/end → turn/end{interrupted}**，并**写进日志**（durable）。 | `packages/core/session/src/repair.ts:53-98`、`:15-41`、`:166-194`、`:191`；`agent-loop/src/index.ts:888-899` |
| R4 | 合成结果必须携带"事实不确定性"且可操作（"Do not retry blindly…"），不得伪装成功。 | `repair.ts:15-18`、`:32-41` |
| R5 | 展示与上下文分层：UI 依 `turn/end.reason` 出状态行；"Interrupted" 工具卡只是 UI 投影，**不进日志**。 | `packages/client/ui-chat/src/client/conversation-nodes/turn-error.ts:36-50`、`tool.ts:248-262` |
| R6 | 明确否决"加载时截断被中断轮"：`Truncate interrupted final turns on load` 被拒（"a single turn can contain substantial real work…"）。 | `.agents/notes/rejected/simplification/2026-06-20-truncate-interrupted-turns.md:3` |
| R7 | 测试钉死语义：resume 后中断内容仍在磁盘且在下一请求 messages 里；SIGKILL 真进程也能补齐。 | `agent-loop/tests/resume.spec.ts:479,504`、`cancel.spec.ts:571,597,664`、`session-checkpoint-policy/tests/crash-recovery.e2e.ts:98-137` |

## 1. 根因（本仓）

**RC1（上下文丢失，主因）**：中断轮的工具往返不是"真值"，只在运行时内存；收口时被替换成一条检查点文本（F3）。续跑请求走 history carry（F4/F9），且 native transcript 分支因对齐失败被跳过（F5）⇒ 模型对中断轮的全部记忆 = 一条 492 字符检查点 ⇒ 在飞工作"消失"。违反 R1/R3/R6。

**RC2（历史图污染）**：收口 append 因 `visible≠head` 隐式 fork（F6），且节点 `turn_id` 取 `turn_trace_history.last()` 造成错标（F7）⇒ 同一轮在历史里出现两条记录，进一步破坏 native/history 对齐与后续轮次归属。

**RC3（展示污染）**：DSML 折叠在解析层写回持久化正文（F8）⇒ 用户看到 `［工具调用］` 连排与参数残片，上下文里也留下垃圾正文。违反 R5。

## 2. 冻结验收断言（红相唯一判据；Test Agent 按此写用例）

| ID | 断言（黑盒，可机器判定） | 判定口径 |
|---|---|---|
| C1 | **中断轮工具往返进上下文**：构造"本轮执行 N≥2 次已闭合工具调用后被中断 → 用户继续"的会话，下一轮 provider 请求的 messages 必须包含这 N 次调用的工具名与结果内容（native 协议族：role=tool / tool_calls；非 native 族：等价的可见步骤文本）。**禁止**只出现一条检查点句。 | 断言工具名与结果唯一哨兵串出现在请求体；且请求 token 计数 > 仅检查点场景（用纯函数/序列化层断言，不依赖真实网络） |
| C2 | **历史恰好一条中断轮**：中断 + 继续后，`history`/`history_nodes` 中该 turn 恰好一条记录，`turn_id` == 被打断的 turn id（不得是上一轮）。 | 纯函数/内存 store 断言 |
| C3 | **不得因中断隐式 fork**：`visible == branch_head` 的正常续跑路径下 `history_branches.len()` 不增加、`active_branch_id` 不变；只有显式 rewind/checkout 才允许 fork。 | store 断言 |
| C4 | **展示：一行状态 + 步骤可查**：TUI 信息流对中断轮只渲染一行状态（`已停止/已中断`），其已完成步骤进"过程明细"；渲染文本**不得**包含 `［工具调用］` 连排或参数残片。 | 纯函数（渲染/清洗）断言 + `errors.rs` 现有入口 |
| C5 | **折叠不写回**：DSML 协议块的折叠结果不得进入持久化 assistant 正文/事件（assistant chunk/message 载荷、normalized_messages 正文）；持久化只留"已交付正文"；协议块整体丢弃。旧库既有脏行只读兼容（不回写、不再放大）。 | 纯函数 + 事件载荷断言 |
| C6 | **恢复幂等**：同一中断轮连续两次恢复/重启，不新增重复轮次、不重复注入 transcript、seq 单调。 | 断言 history 长度与 seq |
| C7 | **失败轮同构**：`turn:failed` 与用户中断走同一"落库 + 闭合"路径（同一纯函数出口），失败原因保留。 | 断言两路径产出的闭合结构同构 |
| C8 | **向后兼容**：旧会话（无新字段）装载/续跑不变；成功轮上下文与展示逐字不变；无新增列/迁移；`normalized_messages` schema 不变。 | 回归用例 + 基线对照 |

## 3. NFR 基准（严格 5 键；Protocol Agent 落 `.dev-team/nfr-baseline-interrupted-turn-resume.json` 并过 `dev-nfr-lint.py`）

| 字段 | 取值 | 说明 |
|---|---|---|
| `external_call_timeout_ms` | 3000 | 不新增网络/外部 IO；本波只改请求组装与持久化 |
| `retry` | `max_attempts=5` / `backoff_cap_ms=8000` / `applicable=true`（沿用既有 provider 预算，禁新增重试层） | 复用 `agent/retry.rs` |
| `concurrency_lock` | 复用 `sessions` RwLock 与 staging-commit；新增状态一律走现有锁序，禁新增长期持锁/嵌套锁 | `turn_persist.rs` 现有模式 |
| `logging` | `format=json`、`trace_id=true`；新日志必须带 `session_id`/`turn_id`，禁裸 `println!` | `docs/observability.yaml` |
| `resource_release` | 无新增句柄/线程/连接；折叠与对齐为纯内存计算，单会话有界（沿用 history 上限） | — |

## 4. 写域（正交、按路径段）

| 任务 | 写域 |
|---|---|
| T0 Protocol | `.dev-team/`（本文件 + NFR + 红相记录）；`crates/pony-agent-core/src/agent/session/interrupted_turn.rs`（**仅空桩/类型/常量**） |
| T1 Test | `crates/pony-agent-core/tests/interrupted_turn_resume_acceptance.rs`；`crates/pony-agent-core/tests/acceptance/adversarial/interrupted_turn_resume/`；`.dev-team/red-phase-interrupted-turn-resume.json`、`.dev-team/l2at-done-*.json` |
| T2 Executor | `crates/pony-agent-core/src/agent/**`（`session/interrupted_turn.rs` 实现、`runtime/*`、`context.rs`、`session/store.rs`、`projection.rs`、`provider/mod.rs`）；`crates/pony-agent-core/src/bin/tui.rs`（仅展示/清洗调用点）、`bin/tui/errors.rs`；`docs/observability.yaml`；`.dev-team/green-receipt-interrupted-turn-resume.md` |
| T3 Lead | `.agents/notes/implemented/bug-fix/`（ADR）、`backlog/`（账本留痕）、join-barrier 收口提交 |

**公共契约不变量**：不新增对外命令形状/端点/环境变量/依赖/列/迁移；`TurnHistoryMessage`、`TurnTraceRecord` 字段编号与 serde 属性不动；`provider_native_transcript` 为唯一工具往返载体；事件类型不新增（沿用 `assistant/message`/`tool_call`/`tool_result`/`turn_end`）。

## 5. 未决项（Protocol Agent T0 只读取证；结论已原地闭合）

- **O1（已闭合）结论：前提不成立——不存在「同一 `turn_id` 重新发起请求」，既不是 TUI 重入也不是 provider 重试。** ①只读实库 `turn_events`：该会话 6 个轮次 6 个互异 `turn_id`，每轮 `turn/start` 恰 1 条、`context/observation` 恰 1 条（seq 依次 1/47/72/207/441/500），**无任何 turn 出现第二个 `turn/start`**。②代码：`stop_turn` 只做「取消令牌 + 恰好一条 `turn:stop_requested` + 宽限监督收口」，全程不重放、不 join（`agent/control_plane/mod.rs:2336-2380`，`:2373-2378` 为监督）；`turn_runner.rs` 全文无重跑入口（只有 hook 派发与 checkpoint 更新，`:323-362`）；provider 重试发生在同一请求内、不产生新 `step/start` 事件（`agent/runtime/turn_stream.rs:2236-2245` 的取消检查接在帧循环上，`:2569-2573`/`:2608-2629` 取消即 `return`）。③「从 step 0」是**新轮的常态**：每轮首个请求由 `step=0` 组装（事件 payload `"step": 0`，`agent/projection.rs`/`build_context_observation` 口径），事故里 `turn-040d93d2` 的 user/message 是用户输入「发生了错误，先记录，然后继续」，请求为 12 条消息 / 27,837 字符 requestMessagesText + 45 工具定义（≈16k tokens 量级；对照 `turn-54d1cf25` trace `input_tokens=16175`）——所以「16k 重启」的真身是：**上一轮以 error/中断收口（`turn/end{reason:"error"}`、无 `assistant/message`、无 `step/end`）→ 用户继续 → 新 turn 从 step 0 组装一个小请求，那 43 次工具往返只剩一条检查点**。④TUI 侧唯一「重排」是**非流式闩窗口内 Esc 把草稿回队**（`bin/tui.rs:9549-9557` `requeue_front`），且下次提交仍是新 `turn_id`；与事故无关。
- **O2（已闭合）结论：`align_native_transcript_with_history` 没有「对齐失败即跳过」的分支——它从不减少 native，最多「不再追加」。598 KB 未被使用的原因是请求走了非 native（`full_replay`）分支。** 判据与五条分支（`agent/context_window.rs:219-292`）：`history` 空 ⇒ 原样返回 native（`:223-225`）；native 空 ⇒ `history_to_native_messages` 纯文本回退（`:226-228`）；末条 user 无可锚定文本 ⇒ 有工具交互则保留 native、否则纯文本回退（`:230-237`）；history 中无同文 user 锚点（异构/深度重写）⇒ **保留 native 不追加**（`:249-252`）；重复文案且无 native 末条 assistant 可消歧 ⇒ 保留 native（`:259-274`）；锚点后有更晚 user 轮 ⇒ 追加纯文本尾轮（`:277-288`）。入口 `load_native_transcript_messages`（`:295-308`）= 滑动切片 → `prune_native_messages_in_place`（只截长结果，不删消息，`tool_pruner.rs:117-132`）→ `recover_incomplete_tool_roundtrips`（补/滤孤儿，仍保留 tool 消息，`:479-481`）→ 对齐。分支选择在 `agent/context.rs:555` 的 `native_tool_flow(provider, retrieved)` = `requires_provider_native_tool_flow() || (supports_native_tool_transcript_replay() && has_native_tool_interactions(...))`（`agent/context_window.rs:311-318`）；事故模型 `workbuddy/deepseek-v4.1-flash`（`~/.config/pony-agent/providers.json`：`openai-completions`、`supportsTools=true`、`supportsReasoning=false`）⇒ requires=false、replay=true，**唯一可能为假的是 `has_native_tool_interactions`**；它为假时 `context.rs:672-675` 落 `full_replay`，实库 `conversationCarryMode=full_replay` 与 `requestFormat=normalized-input`（`agent/provider/mod.rs:256-269`：native_messages 为空即此值）互相印证。**剩余缺口 `[UNVERIFIED-候选]`（只读无法回放当时内存态，交 executor 二段侦察）**：(a) 该时刻 live `session.provider_native_transcript` 为空 ⇒ 走 `history_to_native_messages` 纯文本回退（`agent/context.rs:833-836` + `context_window.rs:172-184`，只产 `role/content`）；(b) `sanitize_provider_native_transcript` 因 `has_legacy_reasoning_gap` **整段 `clear()`**（`agent/session/store.rs:5120-5140`，清空于 `:5135`；本模型 `supportsReasoning=false` 但实库 transcript 里多条 assistant 带非空 `reasoning_content`，故 `is_reasoning_transcript` 判真 `:5146-5157`，一旦出现「`tool` 之后的 assistant 无 reasoning」即整段清空）。倾向 (a)：实库 216 条 transcript = 第 1–4 轮累计（104 条 tool = 9+4+43+48），(b) 的清空会经 `should_save` 落盘成空，与实库不符。
- **O3（已闭合，对 fork 修法决定性）结论：能自然出现，且**不需要** rewind。`visible ≠ branch_head` 由 `ensure_history_graph` 在每次 append 前**主动制造**：它把 main 分支 head 重算为「最后一个 `kind != TurnCancelled` 的节点」（`agent/session/store.rs:3531-3536`），写回分支行（`:3554-3558`），再**覆盖** `cursor.branch_head_node_id`（`:3577-3580`）——而 `visible_node_id` 仍停在刚提交的 `turn_cancelled` 节点（`commit_history_node_from_live_state` `:3709-3711` 三值同置）。append 的调用序正是「先 `ensure_history_graph` 再 `prepare_session_for_new_turn`」（`:703-704`），于是**任何一次中断/失败轮收口之后的下一轮 append 都必然触发隐式 fork**（`:3600-3634`）。事故库三重佐证：`branch-main.head_node_id = node-…564991-3`（最后一个 `turn_committed`，跳过了 `turn_cancelled` 的 `node-…314209-4`）；branch-2 `base_node_id = forked_from_node_id = node-…314209-4`（被跳过的那个中断节点）；`followed fork` 发生在 `turn-040d93d2` 的 append。⇒ **RC2 的修法是「修 cursor/分支头一致性」，不是「改 fork 判定」**：`TurnCancelled` 节点必须参与分支头计算（或在收口时把 `visible` 与 `branch_head` 一致地停在上一 `turn_committed`），fork 语义只留给显式 rewind/checkout；同时节点 `turn_id` 必须由本轮显式传入（`turn_persist.rs:116-131` 的 append 路径不写 trace ⇒ `last()` 恒为上一轮，见 F7）。
