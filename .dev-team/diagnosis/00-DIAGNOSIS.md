# Pony Agent TUI 会话诊断（session-b1e4d476 / trace 438d861f…）

- 诊断对象：`/home/dm/pony-agent`，最近一次 Pony TUI 会话 `session-b1e4d476`（标题「为什么当前项目tui退出后…dev team诊断并修复」，2026-11-05 16:49 完成首轮）。
- 诊断方式：Lead + 3 路只读诊断（trace 取证 / 代码审计 / TUI tmux 复现），**未改任何源码、配置、git**；仅新增本目录诊断报告与 `/tmp` 证据。
- 详细证据：
  - [trace.md](trace.md)：trace 上传核验（四后端）
  - [team-audit.md](team-audit.md)：agent team 与追问断流根因（file:line）
  - [tui-repro.md](tui-repro.md)：TUI 复现与退出/续谈验证（tmux 截图与 pty 原始字节）

---

## 0. 结论摘要

| # | 结论 | 严重度 |
|---|---|---|
| 1 | **该会话的 trace 没有上传**（四后端零命中；本地库有、远端零条）。原因：TUI 进程 OTel token 四源全空，公网 edge 401 全量丢弃。 | P1（可观测性） |
| 2 | **tester/reviewer 无回执不是假回执，而是真驱动 + 真失败 + 回执无消费者**：队友 turn 在独立 runtime 里跑成 `openai/gpt-4.1-mini`（未继承宿主 ponyllm），缺 `OPENAI_API_KEY` 立即失败；错误只进 lead 内存收件箱，TUI 无展示、模型也从未调用 `drain_inbox`。 | P0（核心能力） |
| 3 | **用户追问无后续是本地 planner 劫持 + 挂起请求无法匹配**：上一轮 assistant 正文里的「tester **/** reviewer」中裸 `/` 被当路径 → planner 在模型调用前伪造 `Read(path="/")` → 工作区外读触发 Approval → runtime 匹配不到该请求 → 只发裸 `turn:suspended`，无正文/无弹窗/无终态。 | P0（用户可见断流） |
| 4 | **TUI 没有 agent team 视图**：无角色 roster、无队友状态、无子会话入口、无法返回主会话；`/tasks`（“Team 共享任务看板”）实际渲染的是 `TodoWrite` 的 todo，不是 `team_task_*`。 | P1（产品缺口） |
| 5 | **TUI 退出乱码修复在工作树中已生效**（/quit、Ctrl-C、SIGTERM 三路径均验证），但为未提交 665 行改动；ADR 0062 声称的验收物不存在，`Status: implemented` 属超前声明。 | P2（交付质量） |

---

## 1. Trace 是否上传：否

**判定：未上传（不是部分上传）。** `traceId=438d861f9312ebaa6d3db23e69636800` 只存在于本地 `~/.local/share/PonyAgent/sessions.db` 的 `session_turn_traces`；四类后端均无。

- Tempo（`10.43.51.135:3200`）：`/api/traces/438d…` → **404**；按 `session.id=session-b1e4d476` / TraceQL `langfuse.session.id` 检索 → `[]`（但同窗口有桌面端 trace 正常入库，inspectedTraces=327，排除后端故障）。
- Langfuse（ClickHouse）：`traces` / `observations` 中该 id 与 session 均 **0 行**（库内 494 条，当天 27 条 `pa_` 会话来自桌面端）。
- OTel 公网 edge：`bearertokenauth/public`；无 token POST → **401** `missing or empty authorization header`；错误 token → 401 `scheme or token does not match`。
- PonySentry（127.0.0.1:3000，DB `ponysentry`）：`public.traces` / `trace_turns` 对该会话 **0 行**；当天 pony-agent 唯一 issue（08:14Z `TurnExecutionFailed`）属于另一会话。
- 结构性原因：`crates/pony-agent-core/src/bin/tui.rs:8413-8465` 装配 OTel 是真实的，`agentTraceReporting=true` 也是开着的；但 `otel::resolve_config`（`agent/otel/mod.rs:553-660`）的 token 四源全空：
  1. `~/.config/pony-agent/settings.json` 无 `otelToken` / `ponysentryClientToken`；
  2. `secrets.json` 只有 ponyllm key；
  3. 进程环境无 `PONY_OTEL_TOKEN`；
  4. 本地 cargo 构建无 `option_env!(PONY_OTEL_BUILD_TOKEN)` 烧录（仅 `.github/workflows/release.yml:34/46` 注入）。
  于是命中 `otel/mod.rs:919-928` 的「无鉴权 token → 公网 edge 401 → 本进程 trace 不会出现在 Tempo/Langfuse」告警。
- 日志铁证：`~/.local/state/pony-agent/logs/pony-agent.log` 在目标会话窗口（08:29–08:49Z）内共 19 条同告警，含 `2026-11-05T08:48:45.068Z`；历史有实锤 401：`otel::shutdown … Status(401,None) url:"https://otel.ponyjob.top/v1/traces"`（2026-11-02）。PonySentry 同样 `client_token` 为空 → fail-closed 零 egress（`agent/ponysentry/client.rs:64-82`）。
- 结论含义：**本地有 trace ≠ 远端有 trace**。本次诊断只能依赖本地 trace + turn_events + TUI runtime 日志；“从后端拉 trace 看断点”这条路当前不成立，本身就是可观测性缺陷（见 §6）。

---

## 2. tester / reviewer 为什么没有返回信息

### 2.1 事实链（来自运行期日志 + 本地 trace + DB）

1. `spawn_teammate` 返回 `ok`，member=`tester`/`active`；`send_message` 返回 `delivered:true / delivery_state:"driving" / mode:"queue" / pending_inbox_len:1 / processed:false`——回执本身不撒谎，语义正是“已入队/已开始驱动，不等于已处理”（`orchestration.rs:105-130`、`:611-675`）。
2. 驱动循环真实存在：`spawn_teammate_in_workspace`（`orchestration.rs:441-463`）→ `send_message_in_workspace`（`:542-580`）→ 后台 `drive_loop_in`（`:584-609`）→ `drive_teammate_turn_in`（`:894-1000`）。日志中队友 turn 真实启动 3 次（tester×2、reviewer×1）。
3. **致命点：队友 turn 的 provider 不是宿主会话的 provider。**
   - 宿主：`ponyllm / deepseek/deepseek-v4.1-flash`（TUI 日志 `turn:run ... requested=ponyllm`）。
   - 队友：`openai / gpt-4.1-mini`（`turn:run requested=openai provider=openai ... model=gpt-4.1-mini`，日志 `:62119`）。
   - 随后 `followup:error provider openai missing API key ... field: OPENAI_API_KEY`（`:62125`），`turn:failed`（`:62126`，text_len=17）。
   - 原因：`HostTurnEngine::run_turn_with_root`（`agent/subagent_runtime.rs:216-231`）用 `AgentRuntimeBuilder::new()` 另建 runtime，`TurnInput{provider_id:None, model_id:None}`（`:222-223`），未注入宿主 `provider_resolver`；builder 回落 `ProviderRegistryStore::new()`（`runtime/builder.rs:100-104`），其内置模板第一位是 `provider-openai / gpt-4.1-mini / OPENAI_API_KEY`（`agent/config.rs:1074-1102`），`resolve_selection(None,None)` 取默认（`:366-395`）。
   - 因此**任何宿主默认 provider 不是 openai 的用户，队友永远跑不起来**；这不是环境偶发，是设计缺口。
   - 队友首跳 `BatchExecute` 预校验失败 `child_not_allowed`（日志 `:62122`）只是把工具错误回喂模型（follow-up `:62124`），不是致死原因。
4. 错误回执只进 lead 内存收件箱：失败补偿 `orchestration.rs:946-960`、成功回执 `:982-997` 都只 `send_message(lead, …)`；lead 收件箱唯一读入口是模型主动调用 `drain_inbox`（`tools.rs:1789-1804`）。本会话 143 次 tool/call 中 `drain_inbox=0`，系统提示词团队策略段（`agent/context.rs:57-80`）也从未教模型使用 `drain_inbox` ⇒ 回执永久滞留内存，UI 无入口展示。
5. 次级事实修正：
   - `list_agents` 显示 `tester=inactive` 恰好说明引擎真的执行过并失败收口（`orchestration.rs:982-988`；无引擎才置 `Active`）。
   - `reviewer` 不在当时的 `list_agents` 里是**时序**：`list_agents`（seq 422）早于 reviewer 的 `spawn_teammate`（seq 463），之后没有再 list；不是丢失。
   - `team_task_*` 任务恒为 `pending / owner:null / revision:1`，因为队友从未活到 claim。
   - `wait_agent` 是纯桩且未暴露为工具：`orchestration.rs:851-879` 里 `let _timeout = …;` 后直接返回；`tools.rs` 无 wait_agent 分发分支。
   - 队友 turn 不落库、无独立可枚举会话（`subagent_runtime.rs:203` 生成 `subagent-{uuid}`，但独立 `SessionStore`；日志 `no trace row session=subagent-…`），因此当前也不存在“进入某个 teammate 会话”的对象。

### 2.2 本地 trace 的对应证据

- trace 共 427 条 timeline / 141 个 tool activities；团队工具均出现且父层 `status=done`：`spawn_teammate×2`、`team_task_create×2`、`team_task_list`、`list_agents`、`send_message`。
- **没有任何 teammate 子 span**：`orchestration.rs` 中 `otel|trace|session_id` 零命中；队友 turn 也不产生独立 trace。
- trace 尾部 `phase=completed`、`error=NULL`、`fallback_reason=NULL`；最终 assistant 正文却写「tester / reviewer 仍在异步收敛」——**父 trace 把“未完成”记录成了“完成”**，且该句正成为下一轮追问断流的输入来源（见 §3）。

---

## 3. 用户追问为什么没有任何后续

### 3.1 复现与事实

- 时间线：
  - 首轮：`turn-7ee56f27…`，727 条事件，`turn/end=completed`（16:49），trace 有记录。
  - 追问 1：16:54:11「检查tester和reviewer的进展」→ **6 条事件，无 turn/end，无 trace**。
  - 追问 2：16:55:30「继续」→ **6 条事件，无 turn/end，无 trace**。
  - tui-repro 17:10:50 在 `session-b1e4d476` 副本上再次追问「继续，把 tester 和 reviewer 的收据补齐」：12/42/72/110/150 秒画面不变，进程 futex 等待、无外连——**稳定 3/3 复现**。
- 追问后的唯一新增 UI：`✓ 读取 (Read) · 收集路径 / 的上下文。`，随后永久静默；无 assistant 正文、无错误提示、无审批弹窗、无 `turn/end`。

### 3.2 根因链（五步，均有 file:line）

1. **planner 在模型之前用启发式接管了追问**：`plan_turn` 先跑 `LocalTurnPlanner::preflight_decision`（`runtime/turn_prep.rs:163-207`）；若 `Some`，直接当作首跳决策（`:203-205`），`provider_decision_with_mode` 根本不会调用。
2. **追问被翻译成“读路径 /”**：`infer_local_tool_call` 的兜底无条件分支 `planner.rs:352-399`；`referenced_path` 来自 `explicit_path.or_else(|| infer_last_referenced_path(history))`（`:236`），后者从历史消息倒序取第一个“像路径”的 token（`:535-540`），而 `looks_like_path` 对**含 `/` 即真**（`:791-813`，`:802`）。
3. **`/` 的来源是上一轮 assistant 正文里的「tester / reviewer」**：该会话最后一条 assistant 消息首行为「## 修复已完成（tester / reviewer 仍在异步收敛）」；`extract_path_candidates`（`:583-600`）把 ` / ` 这个分隔符本身切成候选 `/`，`looks_like_path("/")==true` ⇒ `referenced_path=Some("/")` ⇒ 兜底分支生成 `workspace_gather_context`（display=Read，`description:"收集路径 `/` 的上下文。"`，`path:"/"`，`limit:40,lineCount:80`，与 trace 逐字段吻合）。
   - 这说明：**模型自己上一轮写的“tester / reviewer”把下一轮追问坑死了**，形成闭环因果。
   - 修正 Lead 初始假说：不是 Developer `Session label: graph=state-machine-v1 / session=…`；`planner_history` 只取 `session_context.recent_history`（`agent/context.rs:221-223`），其来源 `session.history` 仅 user/assistant（`agent/session/store.rs:450/456`），Developer label 不进 planner 历史。
4. **工作区外读 → Approval → `control_outcome_pending`**：`governed_executor.rs:236-283` 对工作区外路径返回 `ApprovalRequired`；`dispatcher.rs:1248-1330` 落 `PendingControlRequest`（kind=Approval）；legacy 适配层在 `tools.rs:997-1013`（`:1008`）把它伪装成 `status:"error" / code:"control_outcome_pending"`。
5. **挂起请求匹配不回来 → 裸 suspended**：`turn_stream.rs:539-545` / `turn_sync.rs:167-173` 检测到 pending 后调用 `suspend_turn_for_control_outcome`（`turn_control.rs:222-249`）；但匹配器 `stream_support.rs:657-675` 第一分支要求 `call_id` 相等，planner 造的 `ToolCall.call_id=None`；兜底分支**只接受 `Interaction`（Ask）**，Approval 被结构性排除 ⇒ `pending_request=None` ⇒ 只发裸 `turn:suspended`（无 `turn:ask-suspend-stream` 日志），TUI 侧 `tui.rs:2945-2949` → `:8161-8166` 只置“等待用户输入确认...”，`check_pending_asks` 无请求可弹；且 `turn_control.rs:270-286` 对 Approval 无恢复/重放实现。
   - TUI 实际表现与日志/DB 完全一致：**无正文、无错误、无弹窗、永不终态**。这就是“没听见”的直接原因。

---

## 4. TUI 的 agent team 视图：当前不存在

| 用户期望 | 现状 | 证据 |
|---|---|---|
| 展示角色 roster（tester/reviewer/…） | 无 | `AVAILABLE_COMMANDS` 11 条无 `/agents`（`tui.rs:715-728`）；全文件 `list_agents/spawn_teammate/Teammate` 命中=0 |
| 展示会话/状态（running/inactive） | 无 | 无 `TeammateStatus` 渲染；无 inbox 面板 |
| 进入具体 teammate/subagent 会话 | 无 | `/resume` 只列主会话（47 条，`subagent-*`=0）；队友 turn 不落库，无可进入对象 |
| 返回主会话 | 部分（仅主会话间） | `/resume`、`reset_session_scope` 存在；teammate 层级不存在，因为从未能进入 |
| Team 任务看板 | **名不副实** | `/tasks` 文案“Team 任务看板”（`tui.rs:721`），实际数据源是 `todo::todo_list`（`tui.rs:482-506`）；`tui_tasks_from_todos` 把 `owner/blocked_by/write_scopes` 写死为空（`:462-481`）；`UiEvent::TaskBoardUpdated` 全 crate 无发送点（`:352` + `:8155`）；`team_task_list()` 在 tui.rs 零命中 |

- 数据面其实已具备，纯展示层可复用（无需新造存储）：`list_agents()`（`orchestration.rs:515-540`）、`inbox_pending_len()`（`:756-765`）、`is_driving()`（`:184-194`）、`team_task_list()`（`:1003-1016`）；TUI 与 orchestration 同进程，可直接调。
- 但“进入 teammate 会话/返回主会话”不是纯 UI：当前 teammate turn 用独立 `SessionStore` 且不落库（`subagent_runtime.rs:216`；日志 `no trace row session=subagent-…`），必须先把子会话持久化（parent_session_id + role + status + history + trace），否则 UI 无对象可进。
- 实际截图/捕获：`/tmp/dsh-pony-tui-diag/01-main-screen.txt`、`02-slash-menu.txt`、`03-tasks-panel.txt`、`05-resume-picker.txt`；复现：`14~18`、`22`。

---

## 5. 退出乱码修复与交付质量

- **功能上已修复**（本次二进制 16:47:59 已含未提交改动）：`/quit`、Ctrl-C、SIGTERM 三路径均终端还原 + 打印 `继续该会话请运行：pony resume session-<8hex>`；PTY 原始字节顺序正确：kitty pop（`<1u`+`<u`）→ `>4;0m` → 五级鼠标关 → bracketed paste 关 → `?1049l` → `?25h` → 提示；退出后 `echo` 输入正常；`pony resume session-2d72fe33` 能恢复历史（含 workspace 绑定）；kitty 模拟下 push `>9u` 与 pop 成对，退出时延 0.03s。
- 但交付质量有问题：
  - 变更未提交（`tui.rs` +478/-188，`docs/decisions/README.md`，新增 `tests/tui_terminal_restore.rs`、ADR 0062），当前 `git status` 仍脏。
  - ADR 0062 的“验收”引用的 `tests/tui_resume_cli.rs`、`tools/tui-audit/check_exit_resume.py`、`.dev-team/wave-exit-resume-red/green.md` **全部不存在**（本应由 tester 产出，而 tester 被 §2 缺陷卡死）；`Status: implemented` 属超前，建议降级为 `proposed/partially implemented` 或补齐收据后再置。
  - 首轮 assistant 把“teammate 未收敛”写成“修复已完成”，并在 trace 中留下 `phase=completed`；这属于**假完成叙事**，与 dev-team 纪律“未拿到 tester/reviewer 收据不得宣称全绿”冲突。

---

## 6. 本地 trace 本身暴露的观测缺口

- trace 只覆盖首轮（1 条 `session_turn_traces` 行）；两次追问 turn 无 trace、无 `turn/end`、无 `normalized_turns` 行 ⇒ **挂起/断流类故障在 trace 里完全不可见**，只能靠 runtime 日志与 DB turn_events 事后取证。
- trace 中应有 1×`agent.turn` + 142×`llm.chat` + 141×`tool.call` + `governance.decide`；**队友/子代理零 span**（无子 trace、无 parent→child 关系），团队调用只是父 trace 内的普通 tool span。
- 远端不可见（§1）叠加本地缺子 span，使“团队为什么没回执”在观测面几乎不可诊断；这属于本次故障长期未被发现的结构性原因之一。

---

## 7. 评估（严重度 / 影响 / 修复方向）

| 项 | 评估 |
|---|---|
| P0-1 Agent Team 不可用 | 影响：任何非 openai 默认 provider 的用户，tester/reviewer 永远失败；且失败不可见。修复面：`subagent_runtime.rs` 注入父 provider 快照/`ProviderResolver`；`orchestration.rs` 失败回执增加可见通道；lead inbox 自动注入或 TUI 面板。工作量：中（~1-2 天 + 测试）。 |
| P0-2 追问断流 | 影响：用户任何“继续/检查进展”都可能被劫持，且永不终态；无恢复入口。修复面：收紧 `looks_like_path`（拒绝 `/` 等退化路径）、给 planner 兜底分支加意图门、`preflight` 只在明确意图时短路；`match_pending_control_request` 支持无 call_id/Approval；`Approval` 增加重放闭环；无 pending 时至少返回可诊断错误而非裸 suspended。工作量：中（planner 改动小，但挂起/恢复语义涉及 runtime，需 B 级）。 |
| P1 TUI 团队视图 | 影响：用户明确期望的“角色/会话/状态 + 进入子会话 + 返回主会话”完全缺失。建议：先持久化子会话（parent_session_id/role/status/history/trace），再在 TUI 增加 `/agents` roster（复用 list_agents/inbox_pending_len/is_driving）+ `/tasks` 接真实 `team_task_list`；Enter 进入子会话、Esc 返回。工作量：大（跨 session 模型、UI、trace 契约），建议独立立项（ADR）。 |
| P1 可观测性 | 影响：无远端 trace，事故只能本地取证。修复：给 TUI/CLI 注入 `PONY_OTEL_TOKEN`（或 settings.otelToken）与 PonySentry token；CI 已有 secret 注入，本地/发布路径需补齐；同时把 teammate turn 落为子 trace（parent_span_id），suspended 也应有 trace 或至少结构化事件。工作量：小-中（配置 + 少量埋点）。 |
| P2 `/tasks` 名不副实 | 影响：用户以为看到团队看板，实际是 todo；建议改文案或接真实数据，避免“假能力暴露”。工作量：小。 |
| P2 ADR/交付质量 | 影响：未提交改动 + 不存在的验收物 + `implemented` 超前；建议补齐测试或降级 ADR，按项目纪律提交。工作量：小。 |
| 已修复项 | 退出乱码/`pony resume` 在当前工作树中有效（三路径 + PTY 验证），但未提交、无独立验收；建议保留改动并补 tester 收据后再提交。 |

---

## 8. 建议的修复顺序（本次不执行）

1. **P0-2 先止血**：让追问不再被 planner 劫持（`looks_like_path("/")=false` + 兜底分支加意图门 + preflight 短路条件），并给“无 pending 的 suspended”一个可见错误/终态；补 red 测试固定当前缺陷行为。
2. **P0-1 团队可用**：subagent runtime 继承父 provider/model；失败回执进入可见通道（lead inbox 自动注入或 TUI 面板）；补 e2e（tester/reviewer 真实返回）。
3. **P1 团队视图**：持久化 teammate 子会话（parent/role/status/history/trace）→ TUI `/agents` roster + 进入/返回；`/tasks` 接真实 `team_task_list`。
4. **P1 可观测性**：注入 OTel/PonySentry token；teammate turn 打子 span；suspended 也留痕。
5. **P2 收口**：ADR 0062 降级或补验收；提交当前工作树改动；清理诊断产物/证据目录（按项目纪律归档）。

---

## 9. 未验证 / 边界

- Langfuse/PonySentry HTTP API 无 admin key，改用 ClickHouse/Postgres 直查；未核验其 Web 聚合视图。
- collector 容器无 `sh/cat`，配置取自 ConfigMap；edge 限流（120rpm/IP、16MiB）未实测（本次失败在 401 鉴权阶段）。
- Windows 分支、真机 kitty/WezTerm 物理窗口、无 API key/断网边界、窄终端布局未验证。
- `Approval` 挂起在桌面端（非 TUI）是否可用未验证；本报告只对 TUI 链路下结论。
- 未跑全量构建/测试；TUI 二进制 mtime 16:47:59 已含未提交改动，未重建。
