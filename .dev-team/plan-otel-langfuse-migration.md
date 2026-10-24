# 实施总计划：Agent Trace 从 PonySentry 迁移到 OpenTelemetry → Langfuse

> 分级：**C 级**（跨模块 + 协议变更 + 数据源迁移 + 涉及外部网络出口 + 隐私开关）
> 触发依据：命中 blast-keywords（`thread` / `http` / `fs` / `lock`）+ 外部网络 IO + 跨 5+ 模块。
> 目标锚点：`goal-d51579b9-14b6-45b2-8b77-8ab0d7a64283`

## 1. 交付目标（Objective）

1. **协议改造**：Rust core 接入 `opentelemetry-otlp`，把 Turn / Tool / LLM 调用映射为 OTel span 树，经集群 `otel-collector` 双轨 fan-out（Tempo 档案轨 + Langfuse 语义轨）。
2. **数据源迁移**：PonySentry 的 **trace** 能力全量下线，语义数据只进 Langfuse；**issue/error 上报保持不动**。
3. **隐私开关**：桌面端「配置 → 通用」新增 **全局 Agent Trace 上报** 开关，默认 **关闭**；关闭时零 span 分配、零网络出口。
4. **不冲突原则**：观测页「上传本会话 Trace」按钮保持独立显式动作，不受全局开关约束（用户点击即视为逐次授权），但其后端上传目标同样改为 Langfuse。

### Non-Goals（明确不做）

- 不改动 issue/错误上报链路（`ponysentry::capture_error` / panic hook / 前端 `reportError`）。
- 不做 trace 数据本地存储格式改造（`trace_persistence` 落盘契约不变）。
- 不引入 Prometheus metrics 暴露（`/metrics` 端点另立 backlog，本计划只做 tracing）。
- 不做 Loki 日志采集接入（infra 侧另立，本计划只要求新日志面输出带 `trace_id` 的 JSON）。
- 不迁移 `EvalStatus` 评估标注工作流（PonySentry 侧 trace 下线后随之退役；如需保留语义，另立 backlog 映射到 Langfuse score）。

## 2. 事实基线（侦察结论，含阻塞项）

| # | 事实 | 证据 |
| :--- | :--- | :--- |
| F1 | 自动上报钩子存在但**未提交**：`emit_event` 终态静默调用 `maybe_report_turn_trace` | `git diff crates/pony-agent-core/src/agent/turn_flow.rs`（+2 行，工作区未提交） |
| F2 | 手动上传入口：`upload_session_trace` → `build_session_trace_payloads_sharded` → `send_agent_trace_direct`（PonySentry HTTP） | `crates/pony-agent-core/src/agent/control_plane/query_commands.rs:57-73` |
| F3 | Trace 数据模型：`AgentTracePayload` / `TurnTraceItem` / `ToolCallTraceItem` / `EvalStatus` | `crates/pony-agent-core/src/agent/ponysentry/models.rs:41-123` |
| F4 | 上报门禁：`is_enabled() && client_token.is_some()`；无 token 即零 egress | `crates/pony-agent-core/src/agent/ponysentry/mod.rs:98-104`、`config.rs:27-48` |
| F5 | 桌面端设置链路已就绪：`load_app_settings` / `save_app_settings`，落盘 `config_dir/pony-agent/settings.json` | `src-tauri/src/lib.rs:585-591`、`crates/pony-agent-core/src/agent/app_settings.rs:95-132` |
| F6 | 前端开关组件与 store 现成：`Switch.vue` + `useSettingsStore.saveSettings` | `src/components/config/ConfigGeneralSection.vue:19,344`、`src/stores/settings.ts:43-62` |
| F7 | core 依赖：`reqwest 0.12`(native-tls) / `tokio 1` / 无 `tracing` / 无任何 `opentelemetry*` | `crates/pony-agent-core/Cargo.toml`、`Cargo.lock` |
| F8 | 本机 `cargo 1.91.1` 可用，crates.io 可达（HTTP 200），可拉取 opentelemetry 家族 | `curl https://index.crates.io/config.json → 200` |
| F9 | **阻塞项**：`otel-collector` 仅 ClusterIP（4317/4318/8888），**无公网入口**；公网 host 白名单只有 `sentry.ponyjob.top` 的 `/api/v1/ingest`、`/healthz` | `references/contract.json → observed.services.observability`、`observed.pony_sentry.public_paths` |
| F10 | Langfuse 鉴权由 collector 侧 `basicauth/langfuse` 完成，**应用不持有 Langfuse 密钥** | `references/contract.json → observed.otel.exporters / extensions` |
| F11 | 语义轨路由谓词：span 带 `gen_ai.system` 或任意 `langfuse.*` 属性即被复制到 Langfuse | `references/contract.json → dimensions.tracing.routing` |

## 3. 架构决策

### D1 接入形态：直接用 OTel API，不引入 `tracing` 生态
core 现在零日志框架（裸 `eprintln!`），引入 `tracing` + `tracing-opentelemetry` 需要全局 subscriber 与 lib 侧冲突处理，收益不足。改用 `opentelemetry` API 显式建 span（`tracer.start(...)`），新增 `agent/telemetry/otel.rs`（或 `agent/otel/mod.rs`）作为唯一装配点。
新增依赖：`opentelemetry` + `opentelemetry_sdk` + `opentelemetry-otlp`（features: `http-proto`, `reqwest-client`, `trace`），版本与 `reqwest 0.12` 对齐（`opentelemetry-otlp 0.30.x`）。

### D2 开关门禁：AtomicBool + 自定义 Sampler（关时 Drop）
- 全局 `static TRACE_REPORTING_ENABLED: AtomicBool = false`。
- 自定义 `GatedSampler` 实现 `Sampler`：flag 为 false 时返回 `SamplingResult::Drop` ⇒ **不分配 span、不排队、不导出**（严格零 egress，且开销可忽略）。
- 导出器层再加一道 gate（防止 flag 在 span 生命周期内翻转）。
- Provider 在进程启动时**常驻初始化一次**（Batch 间隔 5s），开关只翻转 flag，不做运行时重建（避免 global provider 替换语义陷阱）。
- 关闭路径：Tauri 退出时 `provider.shutdown()`（满足 NFR `resource_release` / `worker_thread_shutdown`）。

### D3 端点与认证：由 env 决定，代码不硬编码
- `OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_SERVICE_NAME` / `OTEL_TRACES_SAMPLER*` 走标准环境变量；缺省回落到 `http://otel-collector.observability.svc.cluster.local:4318`。
- 桌面端 release 指向**公网 OTLP 端点**（见 §8 未决项 U1）；该端点必须带 token 头（`PONY_OTEL_TOKEN`，走 headers 注入，不写死）。
- Langfuse 密钥永不下发到客户端。

### D4 语义映射（OTel GenAI 约定 + Langfuse 属性）
| Span | 名称 | 关键属性 |
| :--- | :--- | :--- |
| Turn root | `agent.turn` | `langfuse.session.id`=`session_id`、`langfuse.user.id`、`langfuse.trace.name`、`langfuse.trace.tags`、`gen_ai.operation.name`=`agent.turn` |
| LLM 调用 | `llm.chat` | `gen_ai.system`(provider kind)、`gen_ai.request.model`、`gen_ai.usage.input_tokens`/`output_tokens`、`langfuse.observation.model.name` |
| 工具调用 | `tool.call` | `gen_ai.tool.name`、`langfuse.observation.type`=`tool`、`langfuse.observation.input`/`output`、error status |
- 父子：`agent.turn` 为 root，`llm.chat` / `tool.call` 为其子 span（同 trace_id）。
- `trace_id` 回填到新日志面（`trace_id` 字段）与后续 issue 上报的 `tags.trace_id`，实现 Loki ↔ Tempo ↔ Langfuse 互跳。
- token/耗时字段优先取 `TurnTraceRecord` 已有统计，缺省标 `stats_incomplete` 语义用 span attribute 表达。

### D5 手动上传迁移
`upload_session_trace` 改为：读取会话持久化 `TurnTraceRecord` → 重放为 OTel span 树 → **强制导出**（`force_export` 旁路 gate，实现为设置 `GatedSampler` 的单次 override 或使用独立 `SimpleSpanProcessor` 直发），返回 turn 数契约不变（`Result<usize, String>`）。前端文案把「PonySentry」改为「Langfuse」。

### D6 PonySentry trace 下线（破坏性变更，走契约单向门）
删除：`ponysentry::{capture_agent_trace, send_agent_trace_direct, maybe_report_turn_trace, build_agent_trace_from_event, build_session_aggregated_trace_payload, build_agent_trace_payloads_from_turns, build_session_trace_payloads_sharded}`、`AgentTracePayload/TurnTraceItem/ToolCallTraceItem/EvalStatus`、`client::send_trace_direct_http` 与 trace 发送队列分支。
保留：`IngestPayload` / `capture_error` / `capture_payload` / `capture_panic` / `add_breadcrumb` / `install_panic_hook`。
兼容性证据来源：PonySentry 服务端 trace 能力已下线（用户决策 2026-10-24）；`AgentTracePayload` 无第三方消费方（仓库内仅 ponysentry 自身与测试引用）。契约矩阵登记 `breaking`。

### D7 开关持久化与桥接
`AppSettings` 增加 `agent_trace_reporting: bool`（serde 默认 `false`，`camelCase`）；`AppSettingsStorage` 同步。新增 core 公共 API `set_agent_trace_reporting(enabled: bool)`，由 Tauri 侧在 `load_app_settings` 与 `save_app_settings` 时调用，保证「启动即按持久化值生效」。
前端：`src/types/settings.ts` 增字段、`DEFAULT_SETTINGS` 增 `agentTraceReporting: false`、`ConfigGeneralSection.vue` 增开关卡片 + 隐私告示文案（收集内容/不含密钥/可随时关闭）。

### D8 日志面（满足 NFR `logging.trace_id_required`）
新增最小 JSON 日志函数（`agent/telemetry/log.rs` 或等价）：输出单行 JSON，含 `level` / `message` / `ts` / `trace_id`；**只用于新增代码路径**，存量 `eprintln!` 按 `logging_legacy_eprintln_exempt` 豁免（PA-119 债务不变，不得借本次扩大豁免）。

## 4. Backlog 拆分（5 项，全部登记 dev-backlog）

| ID | 事项 | 依赖 | 写域（write_scopes） | 验收 |
| :--- | :--- | :--- | :--- | :--- |
| **B1** | infra：OTLP 公网入口 + token 鉴权 + `contract.json` 重录 | 无（并行） | cluster-infra 仓（另仓 PR） | `check-drift.sh` Exit 0；公网 `curl -X POST /v1/traces` 收 200 |
| **B2** | core：OTel SDK 装配 + GatedSampler + Turn/Tool/LLM span 映射 + trace_id 日志 | 无（先做） | `crates/pony-agent-core/src/agent/telemetry/**`、`crates/pony-agent-core/Cargo.toml`、`src-tauri/src/main.rs` | `cargo check/test` 绿；T3/T1/T2 静态断言转 PASS；单测断言 span 属性集 |
| **B3** | 迁移：PonySentry trace 下线 + `upload_session_trace` 改走 OTel 重放 | B2 | `crates/pony-agent-core/src/agent/ponysentry/**`、`control_plane/query_commands.rs`、`turn_flow.rs`、相关 tests | 手动上传端到端产生真实 Langfuse trace；旧 trace API 全删且无残留引用 |
| **B4** | 桌面端：全局上报开关（配置总页）+ 隐私告示 + 互不冲突提示 | B2（需 gate API） | `src/types/settings.ts`、`src/stores/settings.ts`、`src/components/config/ConfigGeneralSection.vue`、`app_settings.rs`、`src-tauri/src/lib.rs`、`src/components/telemetry/TelemetryPage.vue` | 前端单测 + agent-browser E2E：默认关、开启后落盘、Console Error=0、截图落盘 |
| **B5** | 收口：巡检四维 PASS + NFR 报告 + ADR + 文档索引 | B1,B3,B4 | `docs/**`、`.dev-team/**` | `audit-project.sh` Exit 0；`check-drift.sh` Exit 0；ADR 落盘 |

DAG：`B2 → {B3, B4}`；`B1 ∥ B2`；`B5` 收口依赖全部。

## 5. 波次编排

- **Wave 1**（并行 k=2）：B2（Executor A）+ B4 的**协议桩先行**（Protocol Agent 冻 `set_agent_trace_reporting` / `AppSettings` 新字段签名与类型）
- **Wave 2**（并行 k=2）：B3（Executor A）+ B4 实现（Executor B）——写域互斥（Rust core vs 前端 + settings）
- **Wave 3**：B5 收口 + Join Barrier 全量验证
- B1 全程并行（另一仓，不占本仓写域）

隔离：k=2 且共享 cargo 构建缓存 ⇒ 采用**共享工作区 k=1 串行保底**，除非 Executor B 只改前端（前端与 Rust 构建不竞争，可轻量隔离并行）。

## 6. 门禁与物理收据

1. **存量回归基线**：开工前 `cargo test --lib` + `npm run test:unit` 全量跑一遍，落 `.dev-team/regression-baseline-wave43.json`。
2. **红相锚定**：L2-T（每 backlog 一组功能契约）+ L2-AT（≥2 类破坏：OTLP 端点超时/不可达注入、并发开启关闭开关竞态），
   前端任务再加 `agent-browser` Console Error=0 + 截图。
3. **NFR 基准**：`.dev-team/nfr-baseline-wave43.json`——`external_call_timeout_ms ≤ 3000`（OTLP 导出超时）、`retry.max_attempts = 1`、`logging.format=json + trace_id`、`resource_release` 含 `otel_provider_shutdown`。
4. **L3-P 六问**：必须覆盖「超时/日志/资源释放/并发幂等/配置模板 `.env.example`/契约兼容性（D6 破坏性删除须附证据）」。
5. **收口**：`audit-project.sh` 四维 PASS + `check-drift.sh` Exit 0 + `npm run verify` Exit 0。
6. **提交纪律**：工作区已有 4 处未提交改动（F1 等），一律 `git add -p` 定向暂存，禁止 `git add .`。

## 7. 团队编制（Roster ≤ 8）

| 角色 | 数量 | 说明 |
| :--- | :--- | :--- |
| Lead | 1 | 契约冻结/裁决/收口/审计 |
| Protocol Agent | 1 | 空桩 + NFR 基准（L2-P） |
| Test Agent | 1 | L2-T 验收测试（首步加载 `test-expert`） |
| Executor | 2 | A：Rust core（B2/B3）；B：桌面端开关（B4） |
| L2-AT / L3 / L3-P / Explorer | Subagent 池 | 按需挂载，不占常驻槽位 |

## 8. 未决项（需用户裁决，阻塞 B1 与 B4 的 release 路径）

- **U1【已裁决 2026-10-24】公网 OTLP 转输方式**：仍走 `otel.ponyjob.top`，但**不在本波次实施**。
  本波次只负责「接口与配置面就绪」（端点/令牌全 env 化，代码零硬编码），基础设施侧交接单见
  [`.dev-team/handoff-otel-public-ingress.md`](handoff-otel-public-ingress.md)；B1 从本波次移出，
  不阻塞 B2/B3/B4 交付，上线时仅需填 `OTEL_EXPORTER_OTLP_ENDPOINT=https://otel.ponyjob.top`
  与 `PONY_OTEL_TOKEN`。原始三选一记录保留如下（供下一会话参考）：
  - (a) infra PR 新增 `otel.ponyjob.top` → collector:4318，Traefik + token 中间件（推荐，与既有无状态入口同构）；
  - (b) 复用 `sentry.ponyjob.top` 增加 `/v1/traces` 转发到 collector（改动小，但需 PonySentry 承担代理职责）；
  - (c) 本期只支持集群内/局域网端点，桌面端开关先做出来但默认无可用端点（可先交付，infra 后续补）。
- **U2 采样率**：契约建议 `parentbased_traceidratio=0.1` 起；桌面端远端上报建议 `1.0`（用户已显式授权、单机低频）。默认取 `1.0`，可 env 覆盖。
- **U3 手动上传是否受开关约束**：按「用户点击即逐次授权」处理，**不受**全局开关约束（与用户「不冲突」表述一致）。

## 9. 风险

| 风险 | 缓解 |
| :--- | :--- |
| opentelemetry 版本与既有 tokio/reqwest 冲突 | Wave 0 先做依赖预演（单独 `cargo check` 一次），锁定可用版本再进编码 |
| 二进制体积/启动开销 | `default-features = false`，只开 `trace`；关闭开关时 GatedSampler Drop 使开销近零 |
| Langfuse 无数据但 Tempo 有（属性缺失） | 单测硬断言每个 span 至少带一个路由谓词属性 |
| 破坏性删除引发既有测试雪崩 | D6 删除与测试清理同 PR；用 `cargo test` 全量收口，禁止放宽断言 |
| 工作区在飞改动（F1）与本计划冲突 | 开工前先裁决：该 2 行为本次迁移的待删除对象，纳入 B3 一并处理，不单独提交 |
