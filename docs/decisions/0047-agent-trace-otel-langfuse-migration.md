# ADR 0047：Agent Trace 迁移至 OpenTelemetry → Langfuse（PonySentry Trace 下线 + 全局上报开关）

Status: implemented

## 背景

`pony-agent` 的 Agent 运行链路此前走自研私有上报：`ponysentry::AgentTracePayload` 经 `POST /api/v1/traces` 直送 PonySentry，由 `turn_flow::emit_event` 在 Turn 终态静默触发（`maybe_report_turn_trace`），并在观测页提供单会话手动上传（`upload_session_trace`）。

三项事实推动本次变更（2026-10-24）：

1. **PonySentry 的 Trace 能力已下线**，语义数据（思考树、工具调用透视、多轮成本）必须迁到 Langfuse；issue/错误上报仍留在 PonySentry。
2. **可观测性底座已就位但本项目未接入**：集群 OTel Collector 双轨 fan-out（Tempo 档案轨 + Langfuse 语义轨），本项目 `opentelemetry|otlp` 全库 0 命中，巡检 `tracing` 维度 0/5（`T1~T5` 全 MISSING）。
3. **隐私缺口**：桌面端此前默认具备上报能力（构建期烧录 token，见 ADR 0037），用户既无开关也无告知。

## 决策

1. **协议**：引入 `opentelemetry / opentelemetry_sdk / opentelemetry-otlp = 0.30.0`（`http-proto` + `reqwest-client` + `trace`，sdk 追加 `rt-tokio`），经 OTLP/HTTP-protobuf 直送 Collector，由 Collector 侧 `basicauth/langfuse` 完成 Langfuse 鉴权——**应用不持有 Langfuse 密钥**。依赖预演见 `.dev-team/wave43-dep-probe.md`（`http-proto` 会连带引入 `tonic` 的 codegen feature，未启用 gRPC 传输层，可接受）。
2. **模块**：新增 `crates/pony-agent-core/src/agent/otel/`（`mod.rs` + `log.rs`）。**不引入 `tracing` 生态**（core 至今零日志框架，只有 231 处裸 `eprintln!`），改用 OTel API 显式建 span，避免全局 subscriber 与 lib 侧冲突。
3. **门禁与默认关闭**：全局 `AtomicBool` + 自定义 `ShouldSample`（`GatedSampler`）。关闭时直接返回 `SamplingDecision::Drop` ⇒ **零 span 分配、零网络出口**（导出器层再设一道 gate 兜底）。Provider 进程启动装配一次，开关只翻转 flag，不重建 provider。关闭路径 `shutdown()` 幂等可重入，`RunEvent::Exit` 时 `shutdown_with_timeout(3s)` 释放批处理线程与导出 runtime。
4. **语义映射**（属性键即 Langfuse 语义轨路由谓词：命中 `gen_ai.system` 或任意 `langfuse.*` 即被复制到语义轨）：
   - `agent.turn`（root，Drop 收尾）：`langfuse.session.id` / `langfuse.trace.name` / `gen_ai.operation.name=agent.turn` / `agent.turn.id|sequence|phase` / 可选 `langfuse.user.id`；
   - `llm.chat`：`gen_ai.system`（provider kind）/ `gen_ai.request.model` 与 `langfuse.observation.model.name`（同值双写，消除口径歧义）/ `gen_ai.usage.input_tokens|output_tokens`；
   - `tool.call`：`gen_ai.tool.name` / `langfuse.observation.type=tool` / `agent.tool.status|duration_ms|error`。
   - **缺失值一律不输出该键**（不写空串占位，避免 Langfuse 出现空 observation）。
5. **挂载点（零公共签名变更）**：turn root 落在 `runtime/turn_stream.rs` 与 `runtime/turn_sync.rs` 的顶层入口；`llm.chat` 落在 `turn_flow.rs` 的 4 个 provider 调用缝（`provider_decision{,_stream}` / `provider_followup{,_stream}`）——**不放** `provider/mod.rs` 的 `post_*`，因为那里只拿得到裸 `Response/Value`，token usage 在 SSE/JSON 解析之后才产生，下移会导致 span 覆盖不到用量；`tool.call` 只记顶层活动（`parent_activity_id.is_none()`），防 span 基数爆炸。
6. **PonySentry Trace 全量下线**（**破坏性变更**）：删除 `AgentTracePayload` / `TurnTraceItem` / `ToolCallTraceItem` / `EvalStatus`、`capture_agent_trace` / `send_agent_trace_direct` / `maybe_report_turn_trace` / `build_*_trace_payload(s)`、`client::send_trace_direct_http` 与 trace 队列分支；`ponysentry/mod.rs` 由 517 行收敛至 79 行，仅保留 issue/panic 面（`IngestPayload`/`capture_error`/`capture_payload`/`capture_panic`/`add_breadcrumb`/`install_panic_hook`/`is_enabled`）。
   **兼容性证据**：服务端 trace 能力已下线（用户决策 2026-10-24）；`AgentTracePayload` 无第三方消费方（仓库内仅 ponysentry 自身与其测试引用）。
7. **手动上传迁移**：`HostControlPlane::upload_session_trace` 改为把持久化 `TurnTraceRecord` 重放为 OTel span 树并**强制导出**（`BypassGuard` RAII 旁路，用完即释放，不改变门禁末态），返回 `Result<usize, String>` 契约不变。该动作**不受全局开关约束**——用户点击即逐次授权，与全局开关互不冲突。
8. **桌面端开关与告知**：`AppSettings` 增 `agent_trace_reporting: bool`（落盘键 `agentTraceReporting`，缺省 `false`；`AppSettings` 与 `AppSettingsStorage` 均已 `rename_all = "camelCase"`）；配置页「通用」新增「诊断与遥测 → 全局 Agent Trace 上报」卡片与隐私说明；`load_app_settings` / `save_app_settings` 之后立刻同步门禁，保存失败回滚 UI 末态（隐私 fail-closed）。
9. **端点与令牌全部外置**：`OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_SERVICE_NAME` / `OTEL_TRACES_SAMPLER_ARG` / `PONY_OTEL_TOKEN`（→ `authorization` 头）。集群内缺省 `http://otel-collector.observability.svc.cluster.local:4318`；桌面端公网入口 `otel.ponyjob.top` **待 infra 侧单独实施**，交接单见 `.dev-team/handoff-otel-public-ingress.md`，上线时只需填 endpoint + token，**无需改代码**。
10. **NFR**：导出超时对齐 `external_call_timeout_ms = 3000`（L2-AT 黑洞用例实测抓住过 5000ms 的初版）；不重试（`max_attempts = 1`）；导出失败吞掉并降级 `log_json("warn", …, trace_id)`；新增日志面为单行 JSON（`level`/`message`/`ts`/`trace_id`），存量 `eprintln!` 按既有豁免不动、不扩大豁免面。

## 备选方案与取舍

- **引入 `tracing` + `tracing-opentelemetry`**：需要全局 subscriber，而 core 被 Tauri / CLI / TUI / headless 四宿主共用，subscriber 所有权与初始化时机难以统一；收益仅是少写几行 `start()`。放弃。
- **不接 OTel，继续自研 span 上报 Langfuse**：等于重造 OTLP 编解码与批量导出，且语义属性规范会与集群契约漂移。放弃。
- **把 `llm.chat` 挂在 `provider/mod.rs` 的 4 个 `post_*`**：见决策 5，覆盖不到 token 用量，且需要把 usage 回填到别处。放弃。
- **手动上传也服从全局开关**：与用户「两个动作互不冲突」的明确要求相反；点击本身即逐次授权。放弃。
- **开关只存前端 localStorage**：Rust 侧无法据此零 egress，且多宿主（CLI/TUI）不共享。改为设置文件 + core 门禁。
- **前端 `normalizeAppSettings` 只保留已知键**（原实现）：会让任何一次设置保存把 `tool_rules` 覆盖成 `[]`（隐私开关触发数据丢失）。改为以原始对象为基线透传未知键（见 `src/stores/settings.ts`）。

## 后果

- 代码：新增 `agent/otel/{mod,log}.rs`；改 `agent/{mod,app_settings,turn_flow}.rs`、`agent/runtime/{turn_stream,turn_sync}.rs`、`agent/control_plane/query_commands.rs`、`agent/ponysentry/{mod,models,client}.rs`、`src-tauri/src/{main,lib}.rs`、前端 4 文件；`Cargo.toml` +3 依赖（lock 净增 15 包，零版本抖动）。
- 用户可见：配置页出现「全局 Agent Trace 上报」开关（**默认关闭**）；观测页上传改称 Langfuse 并标注与全局开关互不影响。
- 测试：`crates/pony-agent-core/tests/otel_trace_migration_test.rs`（21 例，含 OT-7 全仓退役符号守卫、OT-8 设置契约与宿主桥接静态断言）、`otel_trace_adversarial.rs`（6 例：黑洞超时区间、超大/畸形 payload、Barrier 门禁翻转风暴）、`ponysentry_issue_resilience_test.rs`（2 例，补回被删测试里的队列抗雪崩覆盖）、前端 `tests/otel-reporting-switch.spec.ts`（4 例）。删除 `ponysentry_trace_test.rs`、`trace_auto_report_test.rs` 及孤儿副本，覆盖对照见 `.dev-team/evidence/wave43/deleted-tests-coverage-map.md`。
- **明确非目标**：公网 OTLP 入口（`otel.ponyjob.top`，infra 单独会话）；Prometheus `/metrics` 暴露；Loki 日志采集白名单；`EvalStatus` 评估标注工作流的 Langfuse score 映射；同步路径的 turn 级取消出口；graph/control_plane 离线构造路径的 span 埋点。
- 已知基线缺陷（**非本次因果**，另立 backlog）：`agent::config::tests::missing_capabilities_fields_use_stable_defaults` 在 HEAD 即红（同文件 `DEFAULT_CONTEXT_WINDOW_TOKENS = 1_000_000` 与断言的 `Some(256_000)` 自相矛盾）。
