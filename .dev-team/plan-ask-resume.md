# Ask「回答后本轮不继续」修复 —— 根因裁决与冻结契约

> 工单：TUI Ask 作答后本轮永久停住（独立工单，与面板 TTL 守卫无因果关系）
> 裁决人：Lead｜状态：**frozen**（任何变更须 Lead 原地改写 + 文末流水）
> 基线：`main` @ `01047c4d`（工作树含 token-speed / ask-persistence 两个并行会话的未提交改动，**严禁 revert/覆盖**）

## 0. 根因裁决：C = A + B（A 主修，B 兜底）

**事实链（已读码核实）**

| # | 事实 | 证据 |
|---|---|---|
| F1 | TUI 每轮走 `HostControlPlane::start_turn_stream`，内部调 `runtime.start_turn_stream_with_control`，该入口硬编码 `RunTurnFacts::default()` ⇒ `run_id = None` | `control_plane/mod.rs:1988`、`runtime/turn_stream.rs:1684-1698` |
| F2 | 无 run 事实 ⇒ `DispatchContext.run_id = None` ⇒ 持久化的 `PendingControlRequest.run_id = None` | `runtime/mod.rs:364-373` |
| F3 | `bind_ask_wait_for_request` 需要 `pending_request.run_id`，None 直接 `?` 返回 None ⇒ `bound_run_id = None`，graph ask-wait 未绑定 | `runtime/turn_control.rs:326-377`（338 行 `pending_request.run_id.clone()?`） |
| F4 | `suspension_is_unrecoverable` 只把 `Approval` 判不可恢复；`Interaction` 即使**没有任何 run 身份**也继续走 `turn:suspended` ⇒ 静默死胡同 | `runtime/turn_control.rs:63-70`、`turn_stream.rs:562`、`turn_sync.rs:190` |
| F5 | TUI `submit_ask_answers` 只有 `PendingUiRequest.run_id.is_some()` 才调 `graph_resume_ask`；`UiEvent::AskUserAnswered` 是空实现 ⇒ 作答后什么都不发生 | `bin/tui.rs:8497-8570`、`bin/tui.rs:10825` |
| F6 | `take_pending_ask_injection` 首行 `let run_id = run_id?;` ⇒ run_id 为 None 时注入永不生效（即使答了也没有工具结果回灌模型） | `runtime/mod.rs:270-278` |
| F7 | desktop 侧**始终**走 graph-run 流式链路（`start_graph_run_stream` → `prepare_start_graph_run_stream` → `execute_graph_run_stream`，facts 带真实 `run_id`），故 run 身份恒存在、`graph_resume_ask` 链路成立 | `src-tauri/src/lib.rs:87`、`control_plane/mod.rs:1555-1592`、`:1795-1815` |

**结论：缺口是 TUI 从未拥有 run 身份**（F1→F2→F3→F6 一条链），不是面板、不是 TTL、不是 CAS。

**方案 A（主修）**：让 TUI 每轮流式 turn 在**真实 graph run** 内执行，从而
`PendingControlRequest.run_id` 有值 → ask-wait 绑定成立 → `turn:suspended` 真正可恢复 →
作答后 `graph_resume_ask` 把答案作为**唯一终结工具结果**注入下一轮 → 模型产出下一段 assistant 输出。

**方案 B（兜底，fail-closed）**：只要挂起结果**没有可恢复的 run 身份**（`Interaction` 且
`pending_request.run_id == None`），就**绝不**下发裸 `turn:suspended`，改派发可读失败终态
`ask_resume_unavailable: …`。与既有 W1b 同族（`control_outcome_pending_without_request` /
`approval_resume_unavailable`），同一条判据函数。

**被否方案**
- **A′ 只补 `graph_resume_ask`，不建 run**：`bind_ask_wait` 没有 run 可绑，`resume_ask_wait` 必失败；被否。
- **A″ 在 TUI 手搓 run 状态**（自己 `bind_ask_wait`/改 phase）：与 `graph_runner` 的
  `has_unresolved_ask_waits` / `apply_turn_result` 守卫重复实现，且会绕开 terminal-once 与
  handoff 记账；被否。
- **B-only（只 fail-closed，不修续轮）**：把「不卡死」降级成「明确报错」，用户仍无法作答续轮，
  不满足工单「作答后本轮真的产出下一段 assistant 输出」；被否（B 仅作兜底）。
- **顺手重命名/重构 control_plane**：越界；被否（最小闭包）。

## 1. 语义不变量（实现必须保持）

- **INV-1 CAS 唯一入口**：`answer_ask` 必须先 CAS 消费（`Pending → Consumed`）；
  `graph_resume_ask` 对未消费请求**必须**继续返回 Err（不得放宽）。
- **INV-2 desktop 链路零回归**：`start_graph_run_stream` / `resume_graph_run_stream` /
  `graph_resume_ask` 的 Tauri 命令签名与语义不变；`src-tauri` 与 `src/` 一行不改。
- **INV-3 可恢复 Ask 的 `turn:suspended` 含义不变**：有 run 身份时仍然只发 `turn:suspended`
  （不是 failed/completed），且**不**回灌 provider。
- **INV-4 `control_outcome_pending` 绝不回灌 provider**：挂起标记不得作为普通工具错误进入
  下一轮 provider 请求。
- **INV-5 B 只作用于「无 run 身份」这一种不可恢复形态**：不扩大为「无绑定即失败」，
  否则 `AgentRuntime` 单测里「governed + 无 store」的既有 suspended 语义会被误杀。

## 2. 冻结的接口契约（实施者按此实现，测试按此断言）

**C1 — 新增 `HostControlPlane::start_ask_aware_turn_stream`（改用「新文件」落点，避免与并行会话冲突）**
`crates/pony-agent-core/src/agent/control_plane/ask_resume_commands.rs`（**新建**；`impl HostControlPlane` 可跨文件，
子模块可访问父模块私有项——既有 `ask_plan_commands.rs` 即如此）
`crates/pony-agent-core/src/agent/control_plane/mod.rs` **只允许加一行** `mod ask_resume_commands;`
（原有 `start_turn_stream` 保持逐字不变，`cli.rs` / `non_tauri_harness.rs` 的调用面零改动 ⇒ 缩小爆炸半径）

- 该轮**必须**在真实 graph run 内执行：等价于
  `prepare_start_graph_run_stream(...)`/`prepare_continue_graph_run_stream(...)` +
  `execute_graph_run_stream_step(...)`，即以
  `RunTurnFacts { run_id: Some(run_id), turn_id: Some(turn_id), workspace_root }` 调用
  `runtime.start_turn_stream_with_control_and_facts`。
- run 解析规则（最小增长，避免持久化 store 无界膨胀）：取该 `session_id` 下**最近更新**的
  可续 run（非终态、`active_turn_id` 为空、无未决 ask wait）复用；否则新建一个 run。
  `session_id == None` 时一律新建。
- 既有副作用必须保留：`RecordingTurnEventSink` 的终态 trace 注解、返回前的
  `flush_session_buffered_events`。
- 失败兜底：run 解析/准备失败时**不得**静默 return —— 落一条结构化日志后回落到旧路径
  （无 run 事实），此时 B（C3）负责把 Ask 收口成可读失败终态而不是静默挂起。

**C2 — 作答后续轮（不新增公共 API）**
TUI 作答成功（`answer_ask` + `graph_resume_ask` 均 Ok）后，**必须**再发起一轮续轮：
沿用 `HostControlPlane::start_turn_stream`，`TurnInput.message` 为固定的宿主续轮提示文案
（非空，否则 `prepare_turn` 的 `reject_empty` 会拒绝），`session_id` 与挂起轮一致；
该轮经 C1 解析到刚被 `resume_ask_wait` 置为 `Ready` 的同一个 run，从而
`take_pending_ask_injection(run_id)` 消费一次性注入，provider 看到「原 call_id 的终结工具结果」。

**C3 — 不可恢复挂起 fail-closed**
`crates/pony-agent-core/src/agent/runtime/turn_control.rs`

- 新增常量 `pub(super) const ASK_RESUME_UNAVAILABLE_CODE: &str = "ask_resume_unavailable";`
  与 `pub(super) fn build_ask_resume_unavailable_error() -> String`（首行格式
  `ask_resume_unavailable: <中文可读说明>`，与既有错误码同族）。
- `suspension_is_unrecoverable` 追加判据：`request_kind == Interaction && run_id.is_none()` ⇒ true。
- 流式与非流式两处（`turn_stream.rs:562`、`turn_sync.rs:190`）的错误选择按 kind 分派：
  `Approval` → `approval_resume_unavailable`；`Interaction` 且无 run 身份 →
  `ask_resume_unavailable`；`None` → `control_outcome_pending_without_request`（不变）。

**C4 — TUI 续轮接线**
`crates/pony-agent-core/src/bin/tui.rs`

- `submit_ask_answers` 在 `graph_resume_ask` 成功后返回「是否需要续轮」信号（如
  `Option<String>` run_id 或 bool），由调用方（Ask 覆盖层 Enter 分支）负责把续轮**派发到事件循环**，
  再由主循环用既有轮次线程基建（`TuiTurnSink` + `TuiTurnSinkGuard` + `current_turn_id` +
  `turn_loop_in_flight`）跑续轮；**不得**在按键分支里阻塞跑 turn。
- 保留 `report_control_request_error` 与「不把 raw CAS 串上屏」纪律，一行不回退。
  **跨波次勘误（R2）**：工单提到的面板 TTL 守卫（`pending_request_expired` / `expire_pending_request_now` /
  `ASK_EXPIRED_NOTICE`）在本单实施期间已被**并行波次 ADR 0074**（墙钟 TTL 退出产品语义）整体移除，
  现行为 `ASK_STALE_NOTICE`。本单**既不回退也不重建**该守卫——它不是本单的因果链，且重建会违反 ADR 0074。
- 续轮失败必须上屏可读提示 + 结构化日志，不得静默。

## 3. 验收（测试分权）

**红相已冻结**（原锚点 `3548c692`：sha256 `760e0e3d…b2c80`，942 行，实测 `0 passed; 5 failed`）。
**R3 再冻结**（`56265df2`：sha256 `e9e6e5de248d1207c059c75fbc967f783c9f71af295e70c64139ac34344ed8d8`，971 行，
迁移到 scoped Ask API 后实测 `5 passed`；diff 仅入口/串行夹具/文档，**零断言改动**）。
**实施者只读，禁止任何改动**；收口时按最新 sha256 复核。

红相验收文件（Test Agent 唯一写，冻结后实施者只读）：
`crates/pony-agent-core/tests/ask_resume_e2e.rs`

- **T1** 作答后同一 run 续轮产出下一段 assistant 输出（`start_turn_stream` → Ask 挂起 →
  `answer_ask` → `graph_resume_ask` → `start_turn_stream`(续轮) ⇒ 恰好一条 `turn:completed`，
  文本 = mock 的第二段 assistant 文本）。
- **T2** 挂起轮持有真实 run 身份：`list_pending_asks()[0].runId.is_some()`、
  run phase == `WaitingUser`、恰一条 ask wait 且 `requestId`/`expectedVersion` 对齐、
  恰一条 `turn:suspended` 且**零** `turn:failed`。
- **T3** 无 run 身份时 fail-closed：`turn:failed` 且 error 以 `ask_resume_unavailable` 开头，
  且**零** `turn:suspended`（红线：不允许再出现无终态 suspended）。
- **T4** CAS 不变量：`graph_resume_ask` 对未 CAS 消费的请求必须 Err；`answer_ask` 过期版本必须 Err。
- **T5** `control_outcome_pending` 不回灌：挂起时 provider 只被请求 1 次，第 2 次请求必须包含
  原 `call_id` 的 tool 结果且不含 `control_outcome_pending` 字样。

PTY 端到端（Audit Agent 唯一写）：
`tools/tui-audit/run_audit.sh` 新场景 `ask-resume` + `tools/tui-audit/checks.py` 断言 +
`tools/tui-audit/mock_llm.py` 增加「首轮 tool_call 为 `Ask`」的可选行为：
真实 PTY 里模型发起 Ask → 面板作答 → 断言屏幕在作答后出现第二段 assistant 文本（`FINAL_TEXT` 标记）。

## 4. 门禁（物理收据）

1. `cargo check -p pony-agent-core --all-targets` 退出 0
2. 红相转绿（`cargo test -p pony-agent-core --test ask_resume_e2e`）
3. `cargo test -p pony-agent-core --bin pony`（基线 **13 passed**）
4. `cargo test -p pony-agent-core --test tui_batch3_regression`（基线 **26 passed**）
5. PTY：`tools/tui-audit/run_audit.sh --phase after --bin target/debug/pony --only ask-resume`
   退出 0（重构：以 `--only ask-resume` + `--only ask-panel,ask-loop` 两条命令分别留收据）
   **fail-closed 断言的归属（R2 勘误）**：`PONY_AUDIT_ASK_BOOTSTRAP` 只凭空造一条 pending Ask、**不跑 turn**，
   因此它不会产生 `turn:suspended`，不能用它证明本不变量。该红线断言由冻结核验
   `ask_resume_e2e::t3_…fails_closed_instead_of_silent_suspend` 承担（`turn:failed` 且 error 以
   `ask_resume_unavailable` 开头、`turn:suspended` 恰 0 条）。
6. 可观测性：`docs/observability.yaml` 四维归属补 `ask_resume_unavailable` 收口点，
   `audit-diff.sh --base 3548c692` 退出 0（实测）；`check-declaration.sh` 活体实测 exit 2，
   唯一 FAIL 维是 `tracing` T4 dynamic PARTIAL（1h 短窗 Tempo 0 条 / 24h 71 条），本轮未触及
   otel 导出/采样，判为流量稀疏噪声，按 rubric 如实记 deficiency 并在交付说明列明，不改 `applicable`。

## 5. 写域（write_scopes，正交）

| 角色 | 写域 |
|---|---|
| Test Agent | `crates/pony-agent-core/tests/ask_resume_e2e.rs`（新建，唯一；**已冻结**） |
| Audit Agent | `tools/tui-audit/`（`run_audit.sh`/`checks.py`/`mock_llm.py` 追加） |
| Executor | **新建** `crates/pony-agent-core/src/agent/control_plane/ask_resume_commands.rs`；**一行** `mod` 声明于 `control_plane/mod.rs`；`crates/pony-agent-core/src/agent/runtime/turn_control.rs`、`turn_stream.rs`、`turn_sync.rs`；`crates/pony-agent-core/src/bin/tui.rs`；`docs/observability.yaml` |

**禁止**：`src-tauri/`、`src/`、任何 `*_persistence*`/`*ask_persistence*` 文件、`turn_prep.rs`、`graph.rs`、`tests/tui_batch3_regression.rs`、以及与 token-speed 波次相关的任何文件。

---
变更流水：
- 2026-11-06 Lead：初版冻结（裁决 C = A + B）。
- 2026-11-06 Lead：**契约修订 R1**——C1 落点由「`control_plane/mod.rs` 内改造、`start_turn_stream` 逐字不变」
  修订为「新增 `ask_resume_commands.rs` 承载 `start_ask_aware_turn_stream` + 旧函数体逐字搬为
  `start_turn_stream_legacy` 兜底；`mod.rs` 的 `start_turn_stream` 改为**单行委派**（签名/可见性逐字不变）」。
  触发原因：冻结验收 T1/T2/T4/T5 的验收面就是既有 `start_turn_stream`，「旧方法不动」与「作答后续轮复用同一 run」
  物理不可同时成立。裁决依据（L3 审查 ADVISORY#3 已登记）：测试分权优先于写域洁癖；爆炸半径由
  「mod.rs 内 40 行逻辑」降为「1 行 mod + 1 行委派」，实际与原约束等价。
- 2026-11-06 Lead：**契约修订 R2（跨波次勘误）**——① C4 的「TTL 守卫一行不回退」改为「保留
  `report_control_request_error` 与 raw-CAS-不上屏纪律；TTL 守卫已被并行波次 ADR 0074 整体移除，
  本单不回退也不重建」；② 门禁 5 的 fail-closed 断言归属从「PTY bootstrap 路径」更正为「冻结核验 T3」
  （bootstrap 不跑 turn，物理上无法产生 suspended）；③ 门禁 6 的 `check-declaration.sh` 期望由
  「退出 0」更正为「如实记录 exit 2 + tracing-T4 环境噪声定性」。
- 2026-11-06 Lead：**契约修订 R3（跨波次 API 迁移，用户已批准）**——并行波次 ADR 0074 于 18:33 把
  `HostControlPlane::list_pending_asks(None)` / `answer_ask(..)` / `cancel_ask(..)` 改为 fail-closed，
  新增 scoped 入口 `answer_ask_in_session` / `cancel_ask_in_session`。本单据此适配：
  ① 冻结验收 `ask_resume_e2e.rs` 的 pending 列表与作答调用迁到 scoped 入口，**断言逐条不变**，迁移后
  以新 sha256 再冻结（Test Agent 执行，Lead 复核）；
  ② TUI 4 处 legacy 调用（`answer_ask` + 3×`cancel_ask`）迁到 scoped 入口（session = `state.session_id`）；
  ③ 批准 F1 修复新增的公共入口 `continue_ask_resume_turn_stream`（出货路径：TUI 续轮；`prepare_*` 失败回落
  `start_ask_aware_turn_stream`），并补 fallback 单测；
  ④ 修正 `docs/observability.yaml` 三处精度问题（`[pony-tui]` 日志真实落点、forbidden pattern 判据、
  新增公共入口与日志点登记）。
  未变：`graph_resume_ask` 签名与语义、`answer_ask` 的 CAS 语义（现在是 scoped 入口的四元组 CAS，仍是唯一消费点）。
