# 修复方案（待批准 · 本轮不执行）

> 对象：`/home/dm/pony-agent`；依据：`session-b1e4d476` 诊断（[00-DIAGNOSIS.md](00-DIAGNOSIS.md)）。
> 本文只给问题与方案，**不执行任何代码/配置/集群写操作**；批准后按 §9 波次落地。

---

## 1. 问题清单与优先级

| ID | 问题 | 证据锚点 | 级别 |
|---|---|---|---|
| P1 | 本地 TUI/CLI 的 trace 与 issue 通道未接入（无 OTel/PonySentry token） | `trace.md` §3/§4；`otel/mod.rs:919-928`；`ponysentry/client.rs:64-82`；`pony-agent.log` 19 条告警 | P0（先做） |
| P2 | 子代理不继承宿主 provider/model，且无自定义入口；队友 turn 必落到 `openai/gpt-4.1-mini` 缺 key | `subagent_runtime.rs:216-231`；`config.rs:1074-1102`；日志 `:62119/62125` | P0 |
| P3 | planner 在模型前劫持追问，历史里的 ` / ` 变成 `Read(path:"/")`；工作区外 Approval 匹配不到 → 裸 suspended、永不终态 | `planner.rs:352-399/802`；`turn_prep.rs:203-205`；`stream_support.rs:657-675`；`turn_sync.rs:167-173` | P0 |
| P4 | 团队能力不可见：无 roster/状态/子会话；`/tasks` 是 todo 假看板；回执只进 lead 内存收件箱 | `tui.rs:715-728/482-506/5829`；`orchestration.rs:946-997`；`context.rs:57-80` | P1 |
| P5 | 观测完整性：teammate 无子 span/子 trace；suspended 无 trace；失败回执不可见 | `trace.md` §5；日志 `no trace row session=subagent-…` | P1 |
| P6 | 交付质量：665 行未提交改动；ADR 0062 验收物不存在；`implemented` 超前；无独立测试收据 | `team-audit.md` §D | P2 |
| P7 | `/tasks` 名不副实；`TaskBoardUpdated` 死变体；`wait_agent` 是桩且未暴露；`team_task` 无人 claim | `team-audit.md` §A.3/§C.1.1 | P2 |
| P8 | SSE 解析器出现大量 `provider:sse-tool-call-missing-name` 告警（可能导致工具调用丢失/错拼） | `tui-runtime.log:87834` 等 | P2（需专项确认） |

---

## 2. 方案一：本地接入 trace 与 issue（P0，先做）

### 2.1 目标
本机 Pony TUI/CLI 的一次真实 turn 必须：
1. 在 Tempo 可查到 `agent.turn` trace（HTTP 200）；
2. 在 Langfuse（ClickHouse `traces`/`observations`）能看到语义轨；
3. 在本地 PonySentry 能看到 issue/事件，且 trace_id 回填正确；
4. 日志不再出现「未配置鉴权 token」告警。

### 2.2 凭证来源（只读核对，不写入）
- **OTel（trace）**：设备级 bearer token，存于 `observability/otel-public-ingest-tokens` 的 `tokens.txt`（每行一枚 `otl_<32hex>`）。
  - 推荐：由 infra 侧 `cluster-infra/scripts/observability/provision-otel-token.sh --add devserver-pony-tui` 签发**专用设备 token**（明文仅打印一次；可审计/可撤销）。
  - 最小路径：直接取现有 `tokens.txt` 中一枚（不产生集群变更，但无法区分设备，撤销会误伤其他客户端）。
- **PonySentry（issue）**：`pony-sentry-secrets` 的 `CLIENT_TOKEN`；ingest 头为 `X-Client-Token`，路径 `/api/v1/ingest`。
  - 本机服务：`http://127.0.0.1:3000`（`pony-sentry-server` 正在跑）；生产默认 `https://sentry.ponyjob.top`。
- 现有 `~/.config/pony-agent/settings.json` 已具备 `otelToken` / `otelEndpoint` / `otelServiceName` / `ponysentryIngestUrl` / `ponysentryClientToken` 字段（`app_settings.rs:61-101`），无需新增 schema。

### 2.3 配置方式（二选一）
**A. settings（推荐，TUI/CLI 都生效）** —— 追加到 `~/.config/pony-agent/settings.json`（camelCase；`chmod 600`，不入库）：
```json
{
  "workspaceMode": "coding",
  "agentTraceReporting": true,
  "otelEndpoint": "https://otel.ponyjob.top",
  "otelServiceName": "pony-agent-dev",
  "otelToken": "otl_xxx",
  "ponysentryIngestUrl": "http://127.0.0.1:3000",
  "ponysentryClientToken": "ps_xxx"
}
```
**B. 进程 env（临时/脚本化）**：`PONY_OTEL_TOKEN`、`OTEL_SERVICE_NAME`、`OTEL_EXPORTER_OTLP_ENDPOINT`、`PONYSENTRY_CLIENT_TOKEN`、`PONYSENTRY_INGEST_URL`；TUI 的 `OtelConfig::from_settings` 会读四项白名单（`otel/mod.rs:682-687`），PonySentry `from_env` 读两变量（`ponysentry/config.rs:112-123`）。env 优先于内置缺省、settings 覆盖 env。

**安全约束**：token 只落 `settings.json`（600）或进程 env，不写仓库、不进日志/截图/命令历史；获取时用 `read -s`/管道，不 `echo` 明文；专用 device token 优于共享 token。
**可选（本地方便）**：若只想验证本地链路，可把 `otelEndpoint` 指向内部 collector `http://10.43.93.4:4318`（无鉴权）——但这不是生产同构路径，**不推荐**作为最终接入。

### 2.4 验收步骤（批准后执行）
1. 启动 TUI 做一次短 turn；`grep "未配置鉴权 token" ~/.local/state/pony-agent/logs/pony-agent.log` **无新增**。
2. 从 `sessions.db` 取该 turn 的 `traceId`；`curl http://10.43.51.135:3200/api/traces/<id>` → **200**。
3. `kubectl exec -n observability clickhouse-0 -- clickhouse-client -q "select count() from traces where id='<id>'"` → **≥1**（语义轨）；若为 0，检查 `gen_ai.system`/`langfuse.*` 属性路由。
4. 触发一次受控失败（dev 环境；建议用独立 `environment=dev` 的测试 issue，避免污染生产）→ 本地 PonySentry `public.traces`/`events` 出现该 trace_id；`X-Client-Token` 认证为 200。
5. 回滚：删除 settings 中两 token 或 unset env，重启 TUI，确认恢复 fail-closed 且不产生外连。

### 2.5 交付物
- 配置操作记录（不含明文 token）、验证命令输出、`docs/observability.yaml` 更新（若新增本地接入说明；四维归属仍按现有声明，不伪造 metrics）。
- 可选：为 `pony --doctor`/TUI 启动增加「observability: otelToken/ponysentryToken 已配置（布尔）」自检输出，帮助以后定位（仅布尔，不回显 token）。

**估时**：0.5 人日（含验证）。

---

## 3. 方案二：子代理默认继承宿主 provider/model，并支持自定义（P0）

### 3.1 设计原则
1. **默认继承**：`spawn_teammate` / `subagent` 不显式指定时，使用**调用时宿主会话已解析的** provider + model。
2. **显式覆盖**：工具参数支持 `provider_id` / `model_id`（可选），优先级：显式 > 宿主快照 > 内置默认（默认仅作最后兜底，且必须 fail-fast）。
3. **快照语义**：在 spawn 时固化（provider_id/model_id/base_url/auth 标识，不含明文 key）；后续 `send_message`/驱动使用该快照；用户在宿主切换模型不追溯已派生队友。可在 `send_message` 增加可选 `provider_id/model_id` 覆盖（P2 再做）。
4. **失败前置**：spawn 前解析不到凭证/模型 → 立即返回 `spawn_failed`（含可读原因），不得返回 `ok` 后再异步失败。
5. **不共享可变全局**：不使用全局 `static` 保存“当前宿主 selection”（多会话/多窗口会串）；通过 `ToolExecutionContext` 显式传递快照到后台驱动。

### 3.2 代码改动面
- `crates/pony-agent-core/src/agent/tools.rs`
  - `ToolExecutionContext` 增加 `provider_selection: Option<ProviderSelectionSnapshot>`（或等价结构），由 runtime 在每次工具调用时填当前 turn 的 `ResolvedProviderSelection` 摘要。
  - `execute_spawn_teammate` / `execute_subagent` 从中取快照，传给 `spawn_teammate_in_workspace` / `subagent_in_workspace`。
- `crates/pony-agent-core/src/agent/orchestration.rs`
  - `SpawnTeammateArgs`、`SubagentArgs` 增加可选 `provider_id`、`model_id`（serde 默认 None，向后兼容）。
  - `TeammateMember` 增加 `provider: Option<String>`、`model: Option<String>`、`last_error: Option<String>`、`status: failed` 语义（`TeammateStatus` 已存在 `Failed`，当前驱动失败路径未写入；仅进程内，不序列化 key）。
  - `spawn_teammate_in_workspace(..., snapshot: Option<ProviderSnapshot>)`：先解析 selection（显式 > 快照 > 默认）；解析失败直接 `Err`；成功后写入 `TEAMMATE_PROVIDER_SNAPSHOTS`（内存 map，target → 非密钥 selection）。
  - `drive_teammate_turn_in` 从 map 取快照，传 `engine.run_turn_in(prompt, timeout, workspace_root, Some(snapshot))`。
- `crates/pony-agent-core/src/agent/subagent_runtime.rs`
  - `SubagentTurnEngine` trait 新增 `run_turn_with_selection`（或给 `run_turn_in` 加 `Option<ProviderSelectionSnapshot>` 参数；默认实现回落原 `run_turn`，保持兼容）。
  - `HostTurnEngine::run_turn_with_root`：`AgentRuntimeBuilder::new().workspace_root(root).provider_resolver(Box::new(StaticSelectionResolver(snapshot)))`；`TurnInput{provider_id: Some/None, model_id: …}` 与 resolver 二选一，建议用 resolver（key 解析在 core 内部）。
  - 无 snapshot（旧测试/无宿主）时保持现状并**记录 warn**，不要静默落到 openai 默认；生产路径必须在 spawn 时已有快照。
- `crates/pony-agent-core/src/agent/config.rs`
  - 提供 `ProviderSelectionSnapshot`（provider_id/model_id/base_url 标识/是否有 key；**不含 token 明文**）与 `StaticSelectionResolver`；`resolve_selection` 复用现有 `ProviderRegistryStore`，不要在 subagent 内新建默认 registry。
- 工具 schema / 系统提示：
  - `spawn_teammate`/`subagent` 的 JSON schema 增加两个可选字段；`context.rs` 团队策略段补一句「队友默认继承宿主 provider/model，可用 provider_id/model_id 覆盖」。

### 3.3 失败语义
- spawn 时 provider/model 不存在/无 key → `status:"error"`, `error.code:"spawn_failed"`，并在 `list_agents` 中不产生 Active 成员（或产生 Failed 成员并带 `last_error`）。
- 队友 turn 运行时失败（超时/401/限流）→ `teammate.status = Failed` + `last_error` 写入成员表；向 lead 发回执并在 TUI 可见（方案四）。
- `wait_agent`：后续实现真实等待（轮询成员状态 + Notify，遵守 timeout），并评估是否暴露为工具；本期至少保证 `list_agents` 的 `failed` 可判读。

### 3.4 测试与验收（红相 → 绿相）
1. 单元：precedence（显式 > 宿主快照 > 默认）、无 key fail-fast、快照不序列化 secret。
2. 集成：`wave61_team_activation` 增加用例——宿主 `ponyllm/deepseek` 下 spawn，断言引擎收到的 provider 为 ponyllm；显式 `provider_id/model_id` 覆盖；无效 provider 返回 `spawn_failed`；旧 `StaticResolver` 路径不回归。
3. E2E（无真实网络）：用 fake `SubagentTurnEngine` 断言 `run_turn_with_selection` 参数；真实 TUI 手工验证一次 tester/reviewer 回执进入 lead 收件箱/TUI。
4. 验收：同一会话 spawn tester/reviewer → `list_agents` 不出现 `Failed`；`drain_inbox`/UI 能看到“tester finished”回执。

**估时**：2–3 人日（含测试）；影响面 core + TUI + 工具 schema。

---

## 4. 方案三：追问断流与 Approval 闭环（P0）

### 4.1 修复点
1. `planner.rs`：
   - `looks_like_path` 增加最小合法路径判定：拒绝 `/`、`\`、`.`、`..`、纯分隔符、长度<2、只有扩展名无主体；` / ` 不再算路径。
   - `infer_local_tool_call` 兜底分支加“显式意图门”：只有当前 user message 出现明确路径/文件/目录词（或“这个文件/该目录”等指代 + 上一条明确路径）才允许走 `workspace_gather_context`；`继续`、`检查进展`、纯追问一律返回 None。
   - 重新评估 `planner_can_follow_last_referenced_path_from_history` 用例（它正在冻结旧行为，需同改）。
2. `turn_prep.rs`：`preflight_decision` 只在明确工具意图时短路模型；对“续写/追问/确认”类消息强制走 provider。
3. `stream_support.rs:match_pending_control_request`：
   - 无 call_id 时，Approval 也按（session/turn/tool name）匹配；planner 生成的 ToolCall 补一个稳定合成 call_id（或 dispatch 时回填），保证可匹配。
4. `turn_control.rs`：为 Approval 实现恢复/重放路径（批准 → 重新执行原工具调用并回到 provider follow-up）；无法恢复时返回明确错误并结束 turn。
5. `turn_stream.rs`/`turn_sync.rs`：`pending_request=None` 时不得只发裸 `turn:suspended`；至少发 `turn:failed`（含 code/说明）或结构化 `suspended_without_request` 事件，避免用户静默等待。
6. TUI：对 `Approval` 显示审批弹窗；无请求时显示可读错误；补“继续/重试”入口。

### 4.2 验收
- 固定夹具：`history=[assistant("## 修复已完成（tester / reviewer 仍在异步收敛）")] + user("继续")` → planner 必须返回 None；模型被调用。
- 真实 TUI：追问「继续」后要么得到 assistant 回复，要么得到明确错误；不得出现“无终态”。
- 所有挂起轮次必须有 `turn/end` 或带 request_id 的终止事件；trace/DB 不再出现 6 条事件悬空。
- 红相测试覆盖：`looks_like_path("/")==false`、无请求 suspended 不允许、Approval 可重放。

**估时**：2 人日（planner 0.5 + approval 1 + TUI 0.5）。

---

## 5. 方案四：团队可见性与回执消费（P1）

### 5.1 数据/会话模型
1. 新增 `SessionKind`（main/teammate/subagent）与 `parent_session_id`、`agent_role` 字段（`SessionStore` schema + migration；旧会话默认 main）。
2. 队友 turn 必须写入**同一 SessionStore** 为子会话（不再用独立 `SessionStore::new()` 的孤儿栈），保存 history/turn trace；`HostTurnEngine` 接收 `parent_session_id` 并落库。
3. 为子会话生成 trace：`agent.turn` 子 span（`parent_span_id`=宿主 turn span，`agent.role`/`teammate.target`/`session.parent_id` 属性），tool span 保留。

### 5.2 TUI
1. 新增 `/agents`（或 Ctrl+G）面板：`list_agents()` + `inbox_pending_len` + `is_driving` + `last_error`，显示 name/role/status/failed reason；Enter 进入该 teammate 的子会话（若已落库），Esc 返回主会话。
2. `/tasks` 改接 `team_task_list()`（subject/status/owner/revision/write_scopes/blocked_by），不再用 todo 投影；todo 保留在 `/plan` 或单独面板，避免“Team 共享任务看板”名不副实。
3. Lead 收件箱：启动/每轮前自动 drain 并以 developer 消息注入摘要（限条数/字符），或在 TUI 顶部显示 inbox badge；系统提示词团队段补 `drain_inbox` 说明。
4. 任务认领：`spawn_teammate` 可带 `task_id`（可选）并让队友 prompt 自动 claim；或在 roster 显示未认领任务。

### 5.3 验收
- TUI 可看到 tester/reviewer 的 status、失败原因与回执；Enter 进入其会话并看到历史/工具调用，Esc 返回主会话。
- `/tasks` 显示 `team_task_*` 的真实 owner/revision，而不是 todo。
- 端到端：一次 dev-team 任务中，tester 失败时 Lead 下一轮能自动看到失败摘要（不依赖模型主动 drain）。

**估时**：4–6 人日（会话 schema + 子 trace + TUI 三块）；建议独立 ADR/backlog。

---

## 6. 方案五：观测完整性（P1）

1. **本地 token**（方案一）先行，解决“远端零 trace”。
2. **teammate 子 span**：在 `HostTurnEngine` 中注入 OTel context（parent span 的 traceparent/span id），子 turn 产生 `agent.turn` + `llm.chat` + `tool.call`；`service.name` 仍 `pony-agent`，新增 `agent.role=teammate`、`teammate.target`、`session.parent_id` 属性；不要复制宿主 provider 的密钥到属性。
3. **suspended/failed 留痕**：每个 turn 无论完成/失败/挂起，都要有 `turn` 级 trace 或至少 `turn.suspended` 结构事件（trace_id 可见），避免“永不终态且无 trace”。
4. **PonySentry**：本地 token 后，确认 `capture_turn_failure` 在 TUI 形态启用（已有 `enable_turn_failure_capture(true)`），issue 带 trace_id/release/environment；补一条 e2e 断言（dev 环境）。
5. **自检**：`pony --doctor`/TUI 启动日志输出 `otel: token=configured`、`ponysentry: token=configured`（布尔），不输出值；`/status` 可显示最近一次 export 成功时间（可选）。
6. 文档：`docs/observability.yaml` 增补本地接入/验证说明与 N-A 理由不变；不伪造 `/metrics`。

**估时**：1–2 人日（依赖方案一/二）。

---

## 7. 方案六：交付质量（P2）

1. **拆分提交**当前未提交 665 行：① terminal restore（含 `TERMINAL_INPUT_MODES_RESET`/guard/信号）；② `pony resume` CLI + hydrate；③ ADR/README。每步跑对应测试，避免“大杂烩”提交。
2. **ADR 0062**：在 `Status` 改为 `proposed/partially implemented`，或补齐 `tests/tui_resume_cli.rs`、`tools/tui-audit/check_exit_resume.py`、`.dev-team/wave-exit-resume-red/green.md` 后再置 implemented；验收章节必须指向真实存在的文件。
3. 独立测试：由 Test Agent 写 resume CLI/PTY 验收（不修改产品代码），Lead 冻结；覆盖 `/quit`、Ctrl-C、SIGTERM、空库 resume、未知 id、Windows 分支（能跑则跑）。
4. 首轮 assistant 的“修复已完成（tester/reviewer 仍在异步收敛）”这类**未收据即宣完成**应视为流程缺陷：在 dev-team prompt/审查中禁止；`turn` 收尾时若 required teammate 未 inactive，不得输出完成语。

**估时**：1–1.5 人日。

---

## 8. 方案七：其它（P2）

1. `/tasks` 命名与数据源统一（并入方案四）。
2. `wait_agent`：实现真实等待（`Condvar`/`Notify` + timeout）并决定是否暴露为工具；至少在 `list_agents` 显示 `failed` 后，dev-team prompt 不再依赖不存在的 wait。
3. `team_task` 认领：`spawn_teammate` 可选 `task_id`，自动 claim；否则保持“模型按提示 claim”，但 TUI 可见 pending/owner。
4. SSE 解析器：对 `provider:sse-tool-call-missing-name` 做专项复现（抓取原始 SSE chunk，确认是上游格式差异还是解析器丢字段）；若是解析器，按“缺 name 的事件应缓存拼接或丢弃并计数”，不得静默丢工具；补单测。**先诊断再定是否纳入本轮**。
5. `context.rs` 团队策略段补 `drain_inbox`、`wait_agent` 的真实能力边界，避免模型调用不存在/无效果的 API。

---

## 9. 实施波次（批准后）

| 波次 | 内容 | 依赖 | 门禁 |
|---|---|---|---|
| W0 | 本地 trace/issue 接入（方案一） | 无 | 真实 TUI turn → Tempo 200 + Langfuse ≥1 + PonySentry 事件；日志无告警 |
| W1 | planner/Approval 止血（方案三） | 无 | 红相夹具 + 真实追问有回复/明确错误；无 6 事件悬空 |
| W2 | 子代理 provider 继承 + 自定义（方案二） | W1（可并行） | spawn 预检失败可见；宿主 ponyllm 下队友真实回执；旧测试全绿 |
| W3 | 团队可见性 + 子会话/trace（方案四/五） | W2 | `/agents` 进入/返回、`/tasks` 真数据、teammate 子 span 可查 |
| W4 | 交付质量/清理（方案六/七） | W1–W3 | ADR 一致、提交原子化、测试收据、SSE 专项结论 |

**执行纪律**：每波先红相（Test Agent 独立写/冻结），再实现；Lead 只做契约/审查；B/C 级走 dev-team 门禁；每波物理收据（命令 Exit 0、TUI 截图、Tempo/PonySentry 查询原始输出）。

---

## 10. 风险与待决

1. **Token 归属与轮换**：本地用共享 token 会与桌面端混用，撤销会误伤；建议专用 device token（需 infra 变更批准）。PonySentry token 共享 `CLIENT_TOKEN`，是否需独立 dev token 由 owner 决定。
2. **子会话 schema 迁移**：`SessionStore` 新增 parent/kind 字段涉及 SQLite migration；需先做旧数据兼容/回填策略（旧行默认 main）。
3. **Approval 语义**：TUI 是否支持“批准后重放工具”需产品确认；若不支持，应改为 fail-closed 明确报错而不是挂起。
4. **自定义 provider 的 UX**：模型如何在工具参数里写 `provider_id`/`model_id` 需要提示词与 schema 同步；是否允许 teammate 使用不同 base_url/密钥（安全边界）需裁决。
5. **性能/并发**：队友继承宿主模型后，token 成本/并发（≤2）与限流策略需复核；`subagent_max_concurrent=2` 不变。
6. **观测数据量**：teammate 子 span 会让 Tempo 数据量上升，采样率默认 1.0，需要评估保留期/采样。

---

## 11. 批准请求

建议批准顺序：**W0（本地 trace/issue）→ W1（追问止血）→ W2（子代理 provider 继承）→ W3（团队视图）→ W4（交付质量）**。
本轮仅提交本方案；批准后我从 W0 开始执行，并在每波给物理收据。请确认：
1. 是否允许使用专用 device token（需 `provision-otel-token.sh --add`，集群写）？
2. PonySentry 使用本地 `127.0.0.1:3000` + 共享 `CLIENT_TOKEN` 是否可接受？
3. 是否同意 W3 的“子会话持久化 + TUI 进入/返回”作为独立 backlog（工作量大）？
