# Wave-43 黑盒契约矩阵：Agent Trace 迁移 OTel → Langfuse（L2-T 冻结稿）

- 作者：Test Agent（L2-T 独立测试者，唯一验收测试编写方）
- 基线 commit：`c457a65`（工作区含 4 处未提交改动，见 §0）
- 事实源：`.dev-team/plan-otel-langfuse-migration.md`（D1~D8）+ `/home/dm/.agents/skills/observability-onboarding/references/contract.json`（`dimensions.tracing` / `observed.otel` / `observed.pony_sentry`）
- 分级判定：**L3（工业级全防护）** —— 命中强制升级规则 1（并发开关翻转 + AtomicBool 门禁）、3（跨 core / tauri / 前端 5+ 模块）、4（公共契约破坏性删除 D6 + 外部网络出口）。L2-T 负责功能契约 + 组件契约；L2-AT 负责对抗注入（本文件只标注注入点，不实现）。

> 本文件是测试写域的**唯一依据**：Executor 只读，不得修改本文件或 `tests/otel_*`、`tests/otel-*.spec.ts`。

---

## 0. 事实锚点（侦察结论，断言直接挂这些行）

| # | 事实 | 证据（file:line / 命令） |
| :--- | :--- | :--- |
| A1 | `agent::telemetry` **已存在且是单文件模块**（34 KB，含 `TurnToolActivity` / `TurnTraceRecord` 相关类型），`agent/mod.rs:56` 为 `pub mod telemetry;` | `ls crates/pony-agent-core/src/agent/telemetry.rs`；`grep -n "pub mod telemetry" crates/pony-agent-core/src/agent/mod.rs` |
| A2 | 因此计划 D1 的 `agent/telemetry/otel.rs` **无法在不改 `telemetry.rs` 的前提下落地**（Rust 文件+目录布局要求父文件内声明 `pub mod otel;`），而 task-2 写域只给了 `crates/pony-agent-core/src/agent/telemetry/`。→ 阻塞项 **S1** | 同上 |
| A3 | 待删符号现存位置：`ponysentry/mod.rs`（`capture_agent_trace`、`send_agent_trace_direct`、`maybe_report_turn_trace`、`build_agent_trace_from_event`、`build_session_aggregated_trace_payload`…）、`ponysentry/models.rs`（`AgentTracePayload`/`TurnTraceItem`/`ToolCallTraceItem`/`EvalStatus`）、`ponysentry/client.rs:203,261,305,356`（`capture_agent_trace`、`send_trace_direct_http`、`/api/v1/traces`）| `grep -rn` |
| A4 | 自动上报钩子已在工作区未提交改动中接入：`turn_flow.rs:977` `maybe_report_turn_trace(&payload)` | `git diff crates/pony-agent-core/src/agent/turn_flow.rs` |
| A5 | 既有测试被 D6 直接击穿：`ponysentry_trace_test.rs`(675)、`trace_auto_report_test.rs`(395)、`ponysentry_manual_upload_test.rs`(665) 均 import 待删符号；`ponysentry_credential_gate.rs` / `ponysentry_telemetry_test.rs` **只 import 保留符号**（`IngestPayload`/`PonySentryClient`/`add_breadcrumb`…）→ 后者不动 | `grep -rln "AgentTracePayload\|send_agent_trace_direct\|maybe_report_turn_trace\|EvalStatus" crates/pony-agent-core/tests/` |
| A6 | `AppSettings` 有 `rename_all="camelCase"`，但 `AppSettingsStorage` **没有** rename_all（现落盘键为 `workspace_mode` / `tool_rules`）。D7 要求 JSON 键 `agentTraceReporting`，两者语义冲突 → 阻塞项 **S6** | `crates/pony-agent-core/src/agent/app_settings.rs:63-80,130-146` |
| A7 | `AppSettingsStore::with_path()` 已存在（可用临时路径做黑盒持久化测试，无 `tempfile` dev-dep，用 `std::env::temp_dir()+uuid`）| `app_settings.rs:107-110`；`Cargo.toml` dev-deps 仅 `zip` |
| A8 | 前端上传入口：`TelemetryPage.vue:56 uploadSessionTrace` → `safeInvoke("upload_session_trace",{sessionId})`，成功文案含 "PonySentry"（`TelemetryPage.vue:73,85`）；配置页现无 agent trace 开关（`ConfigGeneralSection.vue` 仅模式/更新卡片） | `grep -n` |
| A9 | 语义轨路由谓词（collector 侧，不可改）：`gen_ai.system` 存在 **或** 任意 `langfuse.*`；完整名单 31 项（含 `langfuse.session.id` / `langfuse.observation.type` / `langfuse.observation.model.name` / `pony.trace.route`）| `contract.json → observed.otel.routing_predicate_attributes` |
| A10 | collector 端点（集群内）：OTLP http `:4318`、grpc `:4317`；Langfuse 走 collector `otlphttp/langfuse`，**应用不持 Langfuse 密钥**；公网入口不存在（F9/U1 未决）| `contract.json → dimensions.tracing.endpoint_http`、`observed.otel.exporters` |
| A11 | 仓库既有验收测试惯例：本地 TCP egress 探针（非阻塞 accept + 固定轮询 + 硬截止，禁 sleep）+ 源码静态扫描断言（`read_repo_source`/`slice_after`）+ mutation-sensitive 断言 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:57-120`、`tests/telemetry-credential.spec.ts:1-30` |
| A12 | 前端测试惯例：`vi.hoisted` mock `@/lib/tauri`（`safeInvoke`/`isTauriAvailable`）+ `setActivePinia(createPinia())` + `data-testid` 断言 | `tests/TelemetryPage.spec.ts:1-60`、`tests/ConfigGeneralSectionUpdate.spec.ts:1-30` |

### 观测装置（本矩阵全部用例共用，物理收据载体）

**P1 本地 OTLP sink 探针**（Rust）：`TcpListener::bind("127.0.0.1:0")` + 非阻塞 accept + 10 ms 固定轮询 + 硬截止；捕获 `method / path / headers / raw body bytes`，回 `HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\n\r\n`（空 `ExportTraceServiceResponse` 合法），观测到首个请求后立即 break 并归还 `Vec<CapturedRequest>`。**禁止 `sleep` 盲等**。

**P2 wire 级字节扫描**（Rust）：`http/protobuf` 未压缩时，protobuf 的 `string` 字段按 UTF-8 原样落在 wire 字节里 → 断言 `body` 含 `agent.turn` / 属性键 / 属性值字面量即等价于「属性真的被导出」。前置断言：body 非 gzip（首两字节非 `0x1f 0x8b`），若出现 gzip 立即以显式信息失败（要求 D1 features 不含 gzip；否则须追加解压断言）。

**P3 静默黑洞 sink**（Rust）：accept 连接后**不回任何字节**，用于制造确定性的导出超时（验证 ≤3000 ms 上限）。与 P1 同构，仅响应策略不同。

**P4 源码静态断言**（Rust/TS）：`read_repo_source` + `slice_after` 窗口断言 / 递归目录扫描（跳过 `target`、`node_modules`、`.git`）。只用于**无法在进程内黑盒观测**的面（宿主生命周期、符号退役）。

---

## 1. 契约总表

| ID | 契约 | 输入（构造） | 期望可观测输出 | 断言方式 | 红相失败原语 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| OT-1 | 全局开关缺省关闭 ⇒ 零 span 导出、零 egress | 缺省 `init(endpoint=P1)`，不调 `set_reporting_enabled`；构造并结束 1 个 TurnScope；`shutdown()` | `is_reporting_enabled()==false`；P1 捕获 **0** 个请求（含 init/flush/shutdown 全窗口） | P1 计数断言 + 返回值断言 | `E0432/E0425`（`set_reporting_enabled`/`init` 不存在）；实现默认开 → `assert!(!is_reporting_enabled())` panic；GatedSampler 未 Drop/exporter 无 gate → `captured.len()==0` 断言失败 |
| OT-2 | 开启后 Turn 终态产生 `agent.turn` root span，带 `langfuse.session.id` | `init(endpoint=P1)` + `set_reporting_enabled(true)` + 干净 turn 终态（`session_id="sess-otel-ot2-<uuid>"`, `turn_id`, phase=`completed`）；结束 TurnScope；`force_flush()`/`shutdown()` | P1 收 1 个 `POST /v1/traces`；body 含 `agent.turn`、`langfuse.session.id`、session id 字面量、`gen_ai.operation.name` | P1 计数 + `path=="v1/traces"` + P2 四项字节断言 | span 名写成 `turn`/`agent_turn` → 字节断言失败；缺 `langfuse.session.id` → 失败；无 flush seam 致未导出 → 计数 0 失败 |
| OT-3 | LLM span 必带 `gen_ai.system` + 模型属性，满足语义轨路由谓词 | 一次 LLM 调用观测：provider kind=`anthropic`, model=`claude-sonnet-4-otel`, `TokenUsage{in=10,out=20}` | body 含 `llm.chat`（冻结名）、`gen_ai.system`+`anthropic`、(`gen_ai.request.model` 或 `gen_ai.model`)+模型名、`gen_ai.usage.input_tokens`/`output_tokens`；**且** 路由谓词交集非空 | P2 逐项断言 + 「body 内出现的 `langfuse.*`/`gen_ai.system` 键集合 ∩ 谓词名单 ≠ ∅」显式断言 | 只设 `langfuse.observation.model.name` 漏 `gen_ai.system` → 强断言红（谓词弱断言仍绿 ⇒ 必须双断言）；provider kind 落成 `openai` 常量 → 值断言失败 |
| OT-4 | 工具 span 必带 `gen_ai.tool.name` + `langfuse.observation.type=tool` | 一次工具观测：`name="bash"`, `status="success"`, `duration_ms=42` | body 含 `tool.call`、`gen_ai.tool.name`+`bash`、`langfuse.observation.type`+`tool` | P2 四项断言（键与值成对断言，防「键在值错」） | 缺 `langfuse.observation.type` → Langfuse 侧不识别为 tool 观察（谓词只保路由不保类型）⇒ 断言失败；值为 `span` → 失败 |
| OT-5 | 导出失败/不可达必须吞掉 + 降级 warn + 超时 ≤3000 ms，绝不阻断主链路 | 5a: endpoint=已关闭端口（bind 后 drop）；5b: endpoint=P3 静默黑洞 | 无 panic；调用返回（不 `Err` 冒泡到 turn 路径）；5a 耗时 < 500 ms；5b 耗时 ∈ [2000, 3250] ms；导出错误分支产出 warn JSON 日志（含 `trace_id`） | `Instant` 差值区间断言 + `catch_unwind`/返回类型断言；日志面用 P4 静态断言（`log_json("warn"` 且带 trace_id）+ 行为级「不 panic / 有界」 | 未吞错（`unwrap`/`expect`）→ panic 断言失败；未设 3000 ms 超时 → 5b 耗时 > 3250 ms 失败；**根本没发**（gate 误关）→ 5b 耗时 < 2000 ms 下界断言失败（防假通过）；`log_json` 无 warn 分支 → 5c 失败 |
| OT-6 | 手动上传在全局开关**关闭**时仍可导出（逐次授权），返回 turn 数契约不变 | 开关保持 false；`init(endpoint=P1)`；构造 2 条 `TurnTraceRecord`；调用手动导出 seam（S4） | `Ok(2)`；P1 收 ≥1 个 `POST /v1/traces` 且 body 含 session id + `agent.turn`；调用后 `is_reporting_enabled()` 仍 `false`（旁路**不翻转**全局开关） | 返回值 + P1 计数 + P2 字节 + 末态 flag 断言 | 旁路未实现 → 0 请求失败；旁路误走全局 gate → 0 请求失败；返回 0/Err → 数量断言失败；旁路实现顺手 `set_reporting_enabled(true)` → 末态断言失败（副作用检测） |
| OT-7 | PonySentry trace API 全下线；issue 上报路径必须仍可用 | 源码全量（`crates/pony-agent-core/src`、`src-tauri/src`、`crates/pony-agent-core/tests`、`src`） | 11 个待删符号 **0 命中**（清单见 §2.7）；保留面仍存在（8 个符号 + 3 处调用点） | P4 递归扫描双向断言 | 残留 `send_agent_trace_direct` → 命中列表非空失败；过度删除（`capture_error`/`IngestPayload` 被连带删）→ 保留断言失败；`src/lib/telemetry.ts` 的 `reportError` 被删 → 前端侧静态断言失败 |
| OT-8 | 设置持久化：缺省 false、写入读回一致、legacy 容错、启动同步 core 门禁 | `AppSettingsStore::with_path(tmp)`（4 个子用例） | 8a 无文件 → `false`；8b `save_view(true)` → `load_view()` 为 true 且 raw JSON 键冻结（S6）；8c legacy `{"workspaceMode":"work"}` → 不 panic 且 `false`；8d `load_app_settings`/`save_app_settings` 均调用 `set_agent_trace_reporting` | 行为断言（a/b/c）+ P4 静态断言（d） | 字段缺失 → `E0609 no field`；serde 缺 `#[serde(default)]` 且默认 true → 8a 红；From impl 未同步 → 8b 往返红；lib.rs 未桥接 → 8d 红 |
| UI-1 | 配置总页开关默认关、开启后落盘、有隐私告示文案 | `mount(ConfigGeneralSection)`，Tauri 分支与非 Tauri 分支各一 | 开关 `data-testid` 存在且默认 `aria-checked="false"`；点击后 `saveSettings` payload 含 `agentTraceReporting:true`；隐私告示元素存在且文案覆盖「收集内容 / 不含密钥 / 可随时关闭」三段语义 | testid + attributes + `safeInvoke` 入参深比较 + 关键词断言 | 未实现 UI → `wrapper.get()` 抛错；默认 true → `aria-checked` 红；未落盘（只改本地 state 不 invoke）→ 入参断言红；无隐私告示 → 红 |
| UI-2 | 观测页单会话上传不受全局开关影响 | `mount(TelemetryPage)`，settings `agentTraceReporting=false`，`sessionId="sess-x"`，mock upload→3 | 点击上传后 `safeInvoke("upload_session_trace",{sessionId:"sess-x"})` 被调用；展示含「3 轮」；文案**不含** `PonySentry`、含 `Langfuse`；reject 分支出现错误 toast | mock 调用入参 + 文本断言 + 分支断言 | 把上传入口包进全局开关 → 0 调用红；文案未迁移 → 含 `PonySentry` 断言红；`catch` 后无用户反馈 → toast 断言红 |
| S-1 | （补充，L3-P 配置模板）`.env.example` 补 `OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_SERVICE_NAME` / `PONY_OTEL_TOKEN`，且**不得**出现 Langfuse 密钥变量 | 读 `.env.example` | 必需变量齐备、无 `LANGFUSE_*KEY*` | 沿用 `tests/env-example.spec.ts` 惯例扩展 | 变量缺失 → 红；出现 Langfuse 密钥模板 → 安全红 |
| S-2 | （补充，NFR）采样率透传不概率断言 | `init` 时 sampling_ratio=1.0 与 env 覆写 0.1 | 配置回显（`config()` / 常量）等于注入值；**不做**「导出概率」断言（防 flaky） | 值回显断言 | 常量硬编码 1.0 覆盖 env → 回显红 |

---

## 2. 逐条契约详表

### 2.1 OT-1 关闭态零 egress（最高危，红线第一条）

- **Arrange**：`std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT")` 等（串行锁 `ENV_LOCK`，与既有测试同构）；启动 P1 探针（预算 1500 ms）；`init(OtelConfig{ endpoint: "http://127.0.0.1:<p1>", service_name: "pony-agent-test", sampling_ratio: 1.0, headers: vec![] })`。
- **Act**：`assert!(!is_reporting_enabled())` → 构造 `TurnScope`（session=`sess-otel-ot1-<uuid>`）→ 结束 → `shutdown()`。
- **Assert**：`p1_handle.join()` 返回 `requests.len() == 0`；失败信息必须打印实际捕获的 `method/path/len`，便于 Executor 定位。
- **物理收据**：探针线程最后输出 `[ot1] egress=0 budget_ms=1500`。
- **变异敏感性**：把 `GatedSampler` 改为 `AlwaysOn` / exporter 前 gate 删掉 ⇒ 必然 `POST /v1/traces` ⇒ 红。把默认 flag 改成 `true` ⇒ 第一条断言红。

### 2.2 OT-2 Turn root span

- **输入事件**：用 `TurnStreamEvent`（`session_id`/`turn_id`/`phase="completed"`/`input_tokens`/`output_tokens`），turn 终态走 `emit_event` 等价路径或 `TurnScope::new(session_id, turn_id, sequence)`（以冻结 stub 为准，见 S2/S3）。
- **断言字节项**（4 项，全命中才绿）：`"agent.turn"`、`"langfuse.session.id"`、`session_id` 字面量、`"gen_ai.operation.name"`。
- **路径断言**：捕获请求 `method == "POST"`、`path == "/v1/traces"`、`content-type` 含 `application/x-protobuf`。
- **root 性**：**黑盒不可靠**——`parent_span_id` 缺失/为 0 需 protobuf 结构解析。若冻结 stub 提供 `TurnScope::trace_id()/is_root()` 则升级为结构断言；否则登记为盲区 B4，不做假断言。
- **收据**：body 前 512 字节 hexdump 落 `.dev-team/evidence/wave43/ot2-body-prefix.txt`（可脱敏：无凭证）。

### 2.3 OT-3 LLM span 与语义轨路由

- **双断言**（缺一即视为放宽断言，禁止）：
  1. 强断言：`gen_ai.system` + `anthropic`、`gen_ai.request.model` + 模型名、`gen_ai.usage.input_tokens`/`output_tokens` 键存在。
  2. 谓词断言：从 body 中抽取出现的键名集合 `K`，断言 `K ∩ PREDICATE != ∅`，其中 `PREDICATE` 从 `contract.json → routing_predicate_attributes` 抄录为常量（31 项，含 `gen_ai.system`）。
- 若 `gen_ai.model`（旧名）出现而 `gen_ai.request.model` 缺失 ⇒ 断言消息必须显式指出「D4 契约为 `gen_ai.request.model`；如裁决改名，须先改契约再改断言」（禁止 Executor 单方改测试）。
- **值断言成对**：键存在但值为空串 ⇒ 红（`"gen_ai.system"` 后必须跟随 provider kind）。

### 2.4 OT-4 工具 span

- 断言字节项：`"tool.call"`、`"gen_ai.tool.name"`+`"bash"`、`"langfuse.observation.type"`+`"tool"`。
- 失败路径覆盖：`status="failed"` + `error` 观测 → span status 置 ERROR 的断言依赖 seam（S2 属性映射或 body 中 `error`/`status_message` 字节），在本轮仅断言「失败工具仍导出 span 且带 `gen_ai.tool.name`」，不虚断言 status 语义（登记盲区 B5）。

### 2.5 OT-5 失败吞掉与超时上限（L3 硬门禁）

| 子用例 | 注入 | 硬断言 |
| :--- | :--- | :--- |
| 5a 拒绝连接 | `endpoint = http://127.0.0.1:<随机 bind-then-drop 端口>` | 无 panic（`catch_unwind` 或直接调用返回）；`elapsed < 500 ms`；主链路（turn 终态构造/结束）仍成功 |
| 5b 接受不应答（黑洞） | P3 探针 accept 后挂起 | `2000 ms ≤ elapsed ≤ 3250 ms`（下界防「根本没发」假通过；上界 = `external_call_timeout_ms(3000) + 250 ms` 余量）；不 panic；`shutdown()` 不阻塞超过 3250 ms |
| 5c warn 降级 | 静态 | `otel` 导出错误分支包含 `log_json` 且 `level` 为 `"warn"`，且该调用携带 `trace_id`（`slice_after(source,"log_json",400)` 窗口内匹配） |

- 说明：stderr 进程内捕获不可靠，故 5c 用 P4；**不接受**「只有 5c 静态断言、无 5a/5b 行为断言」的降级。
- 若 `init` 不暴露可注入 http client，注入点即 `endpoint` 配置（D3 已保证 env/config 可注入）——这也是 L2-AT 的接缝（§3 AT-1）。

### 2.6 OT-6 手动上传旁路

- **必需 seam（S4）**：core 级 `force_export_session_trace(session_id, &[TurnTraceRecord]) -> Result<usize, String>`（或冻结等价签名）。**不接受**用 `HostControlPlane` 构造（依赖 workspace/sessions 存储，测试将退化为重型集成且不可控）。
- 断言：`Ok(2)`；P1 ≥1 请求；body 含 session id、`agent.turn`；`is_reporting_enabled()` 仍 false；再跑一次（幂等重放）仍 `Ok(2)` 且不 panic（重复上传允许产生 2 个 trace，只断言「不崩、数量契约不变」）。
- 前端侧同契约由 UI-2 覆盖（`safeInvoke("upload_session_trace")` 入参不变）。

### 2.7 OT-7 符号退役双向断言

**必须 0 命中（11 项）**，扫描根：`crates/pony-agent-core/src`、`src-tauri/src`、`crates/pony-agent-core/tests`、`src`：

1. `AgentTracePayload` 2. `TurnTraceItem` 3. `ToolCallTraceItem` 4. `EvalStatus` 5. `capture_agent_trace` 6. `send_agent_trace_direct` 7. `maybe_report_turn_trace` 8. `build_agent_trace_from_event` 9. `build_session_aggregated_trace_payload` 10. `build_agent_trace_payloads_from_turns` 11. `build_session_trace_payloads_sharded`
附加（同为零命中）：`send_trace_direct_http`、`/api/v1/traces`（PonySentry trace 路由残留）。

**必须仍存在（双向防过度删除）**：
- `crates/pony-agent-core/src/agent/ponysentry/mod.rs`：`pub struct IngestPayload` 的 re-export、`pub fn capture_error`、`pub fn capture_payload`、`pub fn capture_panic`、`pub fn add_breadcrumb`、`pub fn install_panic_hook`
- `src-tauri/src/lib.rs`：`agent::ponysentry::capture_error(`、`agent::ponysentry::capture_payload(`
- `src/lib/telemetry.ts`：`export async function reportError`；`src/stores/runtime.ts` 仍 import `reportError`
- 既有 `ponysentry_telemetry_test.rs` / `ponysentry_credential_gate.rs` 仍能编译（它们即保留面回归门）

**误删判定**：任何一条保留断言失败 → 直接判 Executor 过度删除，禁止以「新契约不需要」为由改测试。

### 2.8 OT-8 设置持久化

| 子项 | 步骤 | 断言 |
| :--- | :--- | :--- |
| 8a 缺省 | `with_path(<tmp>/settings.json)`（文件不存在）| `load_view().agent_trace_reporting == false` |
| 8b 往返 | `save_view(AppSettings{agent_trace_reporting: true, ..Default})` → `load_view()` | `== true`；raw JSON 解析后按键名（S6 冻结值）取到 `true` |
| 8c legacy 容错 | 写 `{"workspaceMode":"work","toolRules":[]}` 后 `load_view()` | 不 panic；`workspace_mode==Work`；`agent_trace_reporting==false` |
| 8d 启动同步 | P4：`src-tauri/src/lib.rs` 的 `fn load_app_settings` / `fn save_app_settings` 窗口内 | 两处均出现 `set_agent_trace_reporting(`；`load` 侧取 `settings.agent_trace_reporting`，`save` 侧取返回值字段 |
| 8e 退出释放 | P4：`src-tauri/src` 出现 `shutdown()` 且位于 `RunEvent::Exit`/`ExitRequested` 分支 | `nfr-baseline-wave43.json` 的 `resource_release.otel_provider_shutdown` 有代码锚点 |

- 清理：`afterEach` 删临时目录（谁创建谁清理）。

### 2.9 UI-1 / UI-2（Vitest，`tests/otel-reporting-switch.spec.ts`）

- 冻结 testid（S5）：`config-agent-trace-reporting-switch`、`config-agent-trace-privacy-notice`；上传按钮沿用既有 `telemetry-upload-trace`，反馈沿用 `telemetry-upload-feedback`、toast `telemetry-toast`。
- UI-1 三断言族：(1) 默认关（`aria-checked="false"` + store 默认 `false`）；(2) 点击后 `safeInvoke("save_app_settings")` 入参 `{ settings: expect.objectContaining({ agentTraceReporting: true }) }`；(3) 隐私告示文本含三段关键词（建议文案锚点：「收集」「密钥」「关闭」），并断言**不**承诺上传 prompt 明文（若文案含 "完整对话内容" 之类与 D7 描述冲突的词，直接红并上报 Lead 裁决）。
- UI-2 三分支：成功(3)/零(0)/reject。文案断言：全页不出现 `PonySentry`（`wrapper.text()` 不含），出现 `Langfuse`。
- 未实现前置：`normalizeAppSettings` 必须透传 `agentTraceReporting`（防 store 归一化丢字段导致开关回弹）——UI-1 追加断言：`loadSettings()` 返回 `{agentTraceReporting:true}` 后 store 中为 `true`（捕获 `normalizeAppSettings` 漏字段这个高风险点）。

---

## 3. L2-AT 对抗注入点（标注，非本角色执行）

| ID | 注入类别 | 接缝（seam） | 确定性要求 | 目标断言 |
| :--- | :--- | :--- | :--- | :--- |
| AT-1 | OTLP 端点超时穿透 | exporter endpoint（`OtelConfig.endpoint` / `OTEL_EXPORTER_OTLP_ENDPOINT`）+ P3 黑洞 sink | 硬截止 + 区间断言（`[2000,3250] ms`），**禁 sleep** | 超时不雪崩、不阻断 turn |
| AT-2 | 端点畸形响应 | P1 sink 改为返回 `200`+垃圾字节 / `500`+纯文本 / `413` | 固定响应序列（第 1 次 500、第 2 次 200），不断言时间 | 不 panic；重试次数 ≤ 1（`retry.max_attempts=1`）；总连接数 ≤ 2 |
| AT-3 | 并发开关翻转 | `set_reporting_enabled` + `TurnScope` 构造/结束 | `N=200` 固定迭代 + `Barrier(3)` 同步（writer A 置 true、writer B 置 false、reader C 建/销 span）+ `recv_timeout` 硬截止 join；**禁概率等待** | 无 panic/无死锁；终态 flag == 最后一次写入值；导出请求数 ∈ [0,N] 且不重复膨胀 |
| AT-4 | 畸形/超大 payload | 单 turn `tool_calls=1000`、`input_text=1 MB` | 固定规模 | 不 panic；导出 body 有界或分片；内存不失控 |
| AT-5 | 设置文件畸形 | `AppSettingsStore::with_path` 写入 `""` / `"{bad json"` / BOM 前缀 / `null` | 固定 4 样本 | `load_view()` 回落默认（`agent_trace_reporting=false`），不 panic，不覆盖原文件 |

> AT 执行者只读本表的「接缝」列；若实现未提供该接缝（例如 endpoint 不可注入、无 flush），AT 不可执行 → 判 Executor 未满足契约，不得改 AT 以求绿。

---

## 4. 供给侧冻结请求（**阻塞 Phase 2 红相编写，需 Lead 在 task-2 冻结时钉死**）

| ID | 请求 | 现状阻塞 | 建议裁决 |
| :--- | :--- | :--- | :--- |
| **S1** | 唯一化 OTel 模块路径 | `agent/telemetry.rs` 已存在（A1/A2），`agent/telemetry/otel.rs` 需在 `telemetry.rs` 内加 `pub mod otel;`，但该文件不在 task-2 写域 | 走 **`crates/pony-agent-core/src/agent/otel/`**（`agent/mod.rs` 加 `pub mod otel;`），把 `agent/mod.rs` 一行纳入 task-2 写域；或显式批准 task-2 改 `telemetry.rs` 一行。**必须在 stub 冻结前定**，否则我的测试 import 路径写不出（会直接编译红，但不是「契约红」而是「写域冲突」）。|
| **S2** | 纯映射函数（无网络/无全局态）：`turn_span_attributes(&TurnStreamEvent) -> Vec<(String,String)>`、`llm_span_attributes(provider,model,&TokenUsage) -> ...`、`tool_span_attributes(&TurnToolActivity) -> ...` | 当前 stub 只有 `init/shutdown/set_reporting_enabled/TurnScope`，OT-3/OT-4 的属性断言只能靠 wire 字节扫描（脆且无法断言「父子关系/属性类型」）| 纳入 task-2 stub。收益：确定性、零网络、编译期红相、可断言属性全集与值。|
| **S3** | flush 与重入：`force_flush() -> Result<(),String>`，且 `shutdown()` 后允许再次 `init()` | 全局 provider 若 `OnceLock` 化，跨用例无法隔离 → 全部用例被迫塞进 1 个函数，失去 AAA 结构 | 纳入 task-2 stub（`force_flush`），并明确 `init` 可重入语义 |
| **S4** | 手动导出 core 级 seam：`force_export_session_trace(session_id:&str, turns:&[TurnTraceRecord]) -> Result<usize,String>` | `upload_session_trace` 挂在 `HostControlPlane` 上（A8 重型依赖）| 纳入 task-2 stub 签名 |
| **S5** | 前端 testid 冻结 | 未实现，无法预先写断言 | `config-agent-trace-reporting-switch`、`config-agent-trace-privacy-notice` |
| **S6** | storage 落盘键冻结 | `AppSettingsStorage` 无 `rename_all`（A6），D7 要求 `agentTraceReporting` | 二选一并写进 stub 注释：(i) `AppSettingsStorage` 加 `rename_all="camelCase"`（则 `workspace_mode` 也改名，属存量落盘格式变更，需 ADR 说明）；(ii) 单字段 `#[serde(rename="agentTraceReporting")]`。我的 8b 断言按键名硬断言，键名不定则 8b 无法写。|

---

## 5. 存量测试清理清单（Test Agent 写域，必须与 D6 删除同 PR）

| 文件 | 处置 | 理由 |
| :--- | :--- | :--- |
| `crates/pony-agent-core/tests/ponysentry_trace_test.rs`（675 行） | **删除** | 100% 围绕待删 `AgentTracePayload`/`EvalStatus`；语义已被 OT-2/3/4 取代 |
| `crates/pony-agent-core/tests/trace_auto_report_test.rs`（395 行） | **删除** | 断言自动上报 + 待删纯函数；新语义为 OTel span（OT-2） |
| `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs`（665 行） | **重写为 OT-6 + 控制面形状断言** | 上传目标改为 Langfuse 重放；`send_agent_trace_direct` 断言必须消失 |
| `crates/pony-agent-core/tests/ponysentry_credential_gate.rs` | **保留不动** | 仅 import 保留符号（A5）|
| `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs` | **保留不动** | issue 上报回归门（OT-7 保留面的活体证明）|
| `src-tauri/src/bin/direct_turn_probe.rs`（工作区 +1 行） | **不动**（非测试写域；Lead 裁决归属）| 与本迁移无引用关系（grep 无 ponysentry 符号）|

`git add -p` 定向暂存；禁 `git add .`。

---

## 6. 覆盖盲区（诚实登记，禁止用假断言填平）

| ID | 盲区 | 原因 | 本轮处理 |
| :--- | :--- | :--- | :--- |
| B1 | **真实 Langfuse 落库**（Langfuse 侧真能看到 trace、父子树、成本）| 公网 OTLP 入口不存在（F9 / U1 未决），collector 仅 ClusterIP；应用不持 Langfuse 密钥（F10）| 仅断言 wire 级属性 + 路由谓词交集；真落库验收挂在 B1/B5（infra 交付后由 L3/E2E 补）|
| B2 | Tempo 侧查询 `service.name` 维度 | 同上，无入口 | 断言 body 含 `service.name`/`OTEL_SERVICE_NAME` 值（wire 级代理指标）|
| B3 | Tauri 宿主生命周期（启动 init、`RunEvent::Exit` shutdown）| 需真实 Tauri App 运行时，core 集成测试不可达 | P4 静态断言（8d/8e）+ tauri 冒烟由 B4/L3 负责 |
| B4 | root span 的 `parent_span_id` 缺失 / 全树 trace_id 一致性 | protobuf 结构解析需 prost dev-dep；stub 无 `trace_id()` seam | 若 S2 落地则升级为结构断言；否则不做断言（不虚报）|
| B5 | tool span 的 ERROR status 语义（`status=Error` / exception event）| 同上，wire 无稳定可读形式 | 只断言「失败工具仍导出且带 `gen_ai.tool.name`」|
| B6 | 「关闭态零 span 分配」（CPU/内存开销）| 黑盒只能观测零 egress；分配行为需 benchmark/分配器计数 | 只断言零 egress（OT-1）；开销类断言交 L3-P/NFR |
| B7 | warn 日志的进程内 stderr 捕获 | 无 tracing subscriber，`log_json` 直写 stderr，测试内捕获不可靠 | 5c 用 P4 静态 + 5a/5b 行为双轨 |
| B8 | 采样率概率行为（0.1 时确实只导出 10%）| 概率断言必 flaky，违测试规范 | S-2 只断言配置透传 |
| B9 | 前端真实渲染（Tauri 内 `safeInvoke` 真链路 / Console Error=0 / 截图）| vitest + jsdom 无 Tauri | UI-1/UI-2 只做组件契约；agent-browser E2E + 截图由 B4/L3 产出 |

---

## 7. 写域、红线与交付

**Test Agent 独占写域**：
- `crates/pony-agent-core/tests/otel_trace_migration_test.rs`（OT-1..OT-8 Rust 侧）
- `tests/otel-reporting-switch.spec.ts`（UI-1/UI-2 + OT-7 前端侧静态断言 + S-1 的 `.env.example` 扩展）
- `.dev-team/contract-matrix-wave43.md`（本文件）
- `.dev-team/evidence/wave43/**`（红相/绿相收据）
- §5 清单中的存量测试删除/重写

**红线（违一条即判测试无效）**：
1. 无断言测试（纯 `let _ = call()`）—— 零假测试律；
2. 吞异常放行（`catch { }` / `.ok()` 后无断言）；
3. `#[ignore]`、`it.skip`、`describe.skip` 跳过；
4. 放宽断言求绿（改 3000→6000、删 `gen_ai.system` 强断言、把 timeout 上界放到 10s）；
5. 用 `sleep` 概率等待替代 Barrier/硬截止轮询；
6. Executor 修改 `tests/otel_*` 或本文件。

**Phase 2 触发条件**：task-2 冻结完成 + S1/S2/S3/S4/S5/S6 全部裁决落盘。冻结前**不写**测试源码（避免路径/签名未定导致「假红」污染红相基线）。
