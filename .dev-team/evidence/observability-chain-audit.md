# pony-agent 可观测性链路完整性审计报告

- 审计时间：2026-10-25（Asia/Shanghai）
- 审计方法：**契约对齐（`observability-onboarding` skill `check-drift.sh` Exit 0）+ 静态链路逐环核查 + 集群活体动态探针**
- 活体探针说明：使用 `otel-public-ingest-tokens` 中已签发的设备 token，向公网入口真实投递一条合成 span（`service.name=pony-agent-chain-audit`，带 `gen_ai.system`/`langfuse.session.id`），再回查 Tempo / Langfuse。
- 结论口径：`CONNECTED` = 有物理证据证明数据到达；`BROKEN` = 代码或配置证明链路断开；`NA` = 结构上不适用；`UNKNOWN` = 缺凭据/无历史流量无法判定。

---

## 0. 一句话结论

**集群底座（入口→Tempo/Langfuse）物理连通且语义分流正确；但客户端侧存在 3 个 P0 级断链，导致 pony-agent 自身在发布形态下「trace 从不上送、trace_id 永不入库、桌面端日志不进 Loki」。**

最强证据：活体探针投递前，Tempo 的 `service.name` 标签值集合为**空**（`{"tagValues":[],"metrics":{}}`）——即该集群自部署以来**从未收到过任何 pony-agent span**；探针投递后才有首个 `desktop-client`。

---

## 1. 活体验证收据（物理证据）

| 环 | 探针动作 | 结果 | 判定 |
| :--- | :--- | :--- | :--- |
| 公网 OTLP 入口 | `POST https://otel.ponyjob.top/v1/traces` + `Authorization: Bearer <device token>` | `http=200`，`{"partialSuccess":{}}` | ✅ 鉴权与接收正常 |
| Edge 改写/打标 | 查 Tempo 标签 | `service.name=desktop-client`、`client.service.name=pony-agent-chain-audit`、`ingest.source=public` | ✅ 改写与归因符合契约 |
| Tempo 落地 | `tcount 'client.service.name="pony-agent-chain-audit"'` | `1` | ✅ 落 Tempo |
| Langfuse 语义轨 | `GET /api/public/traces` | `id=5b8efff7…`、`name=agent.turn`、`sessionId=audit-session-1`、`metadata.attributes.pony.trace.route=genai` | ✅ 落 Langfuse 且语义路由正确 |
| 入口鉴权边界 | 无 token 请求 | `401 {"code":16,"message":"missing or empty authorization header"}` | ✅ fail-closed |
| 投递前基线 | 查 Tempo 全部 service name | 空集合 | ❌ **应用从未上报过** |

---

## 2. 四维逐环判定

### 2.1 Tracing（trace → OTLP → Tempo / Langfuse）

| 环 | 判定 | 结论与证据 |
| :--- | :--- | :--- |
| SDK/Provider 生命周期 | CONNECTED | `otel::init(from_env())` 于 `src-tauri/src/lib.rs:1236`，`shutdown()` 于 `:1354`（Exit 分支）；测试 `ot8_exit_path_releases_otel_provider` 守护 |
| **全局门禁默认值** | **BROKEN（P0）** | `settings.agentTraceReporting` 默认 **false**（`.env.example` §4 明示，配置页开关 `ConfigGeneralSection.vue:389`）；门禁关 ⇒ `GatedSampler` 直接 Drop，**零 span 分配零 egress** |
| **端点来源** | **BROKEN（P0）** | 端点**只能**来自进程环境变量 `OTEL_EXPORTER_OTLP_ENDPOINT`（`otel/mod.rs:107`）；缺省值 = 集群内 DNS `http://otel-collector.observability.svc.cluster.local:4318`（`:98`）。桌面端在用户机器上无此 env ⇒ 即使打开开关也发往**不可解析的集群内地址** |
| **鉴权注入** | **BROKEN（P0）** | `PONY_OTEL_TOKEN`（`:116`）同样只来自 env，无 settings/无 UI、无构建期注入。发布流水线 `.github/workflows/release.yml:25-28` **只注入 PonySentry token，完全没有 `OTEL_*` / `PONY_OTEL_TOKEN`** ⇒ 发布产物 = 无 token + 内网端点 |
| 采样器 / 队列 | CONNECTED | 默认 1.0 全采；批处理队列 2048 / 单批 512 / 超时 3000ms；`pony.otel.forced` 强制采样仅作用于手动上传重放 span |
| 导出器协议 | CONNECTED | OTLP/HTTP + `HttpBinary`，`traces_endpoint()` 自动补 `/v1/traces`；`authorization_header_value` 幂等组装 `Bearer`（避免 `Bearer Bearer`） |
| Span 树父子挂载 | CONNECTED | `ACTIVE_TURNS`（按 `tokio::task::Id`）+ `THREAD_TURNS`（线程栈兜底）；`agent.turn`/`llm.chat`/`tool.call` 命名冻结并有测试锁定 |
| 语义属性 | PARTIAL | `gen_ai.*` 键齐备、活体探针已证明 `gen_ai.system` 能路由进 Langfuse；但 `langfuse.session.id` **只挂在 `agent.turn` root 上**，`llm.chat` / `tool.call` 子 span 完全没有 ⇒ Langfuse 会话/观察树归集存在退化风险；`langfuse.user.id` 三个调用点全传 `None`（死键） |
| 退出 flush | BROKEN（P1） | 窗口关闭被拦截只隐藏（`lib.rs:1342-1347`），托盘强退 / SIGKILL 不 flush ⇒ 批处理 5000ms 窗口内的 span 直接丢失；`init` 失败无重试，provider 永久缺席 |
| 死命令 | BROKEN（P2） | `record_stream_debug_metrics` 仅存内存 + `println!`，且前端已无调用者（`src-tauri/src/lib.rs:1059-1066,1325`） |
| **W3C traceparent 透传** | **BROKEN（P1）** | `W3cTraceContext` / `inject_traceparent_header`（`otel/mod.rs`）**全仓无任何调用点**——只存在于定义与测试。⇒ 调用外部 API / 后端接口不会带 `traceparent`，跨服务因果链无法续接 |
| 生成速度属性（本轮新增） | CONNECTED | `gen_ai.speed.tokens_per_second` 已在 `turn_flow.rs` 4 个 provider 调用点接线，并有 wire 级断言 + 变异对照证明 |

### 2.2 Issue（PonySentry）

| 环 | 判定 | 结论与证据 |
| :--- | :--- | :--- |
| 前端三网（error / rejection / Vue errorHandler） | CONNECTED | `src/main.ts:24/45/65`，均在 `mount`(:92) 之前注册 |
| Vue 错误边界 | BROKEN（P2） | 全仓 `onErrorCaptured` 0 命中；错误仍被 errorHandler 兜住（采集不断），但无局部降级、整棵子树白屏，上下文仅 `componentTag` |
| **beforeSend 注册** | **BROKEN（P2）** | `setBeforeSend` 仅有测试调用点，生产默认 `null`（`telemetry.ts:63`）⇒ 清洗/裁剪钩子默认不生效（网络噪音由 `isNetworkNoise` 另行生效） |
| 桌面传输（IPC→队列→POST） | CONNECTED | `reportError` → `ponysentry_capture`（`src-tauri/src/lib.rs:618`）→ 有界队列 1024、满即丢、独立 worker 线程 |
| panic hook | CONNECTED | `src-tauri/src/main.rs:5` 安装，携带 `tags.project` |
| **event_id 幂等键** | **BROKEN（P1）** | 前端生成（`telemetry.ts:307`）但 Rust payload 结构无该字段（`ponysentry/models.rs:26-36`），IPC 反序列化静默丢弃 ⇒ 桌面端服务端去重退化为纯 fingerprint |
| **trace_id 回填（Tempo↔Sentry 互跳）** | **BROKEN（P0）** | `trace_id` 是可选参数，仓内 6 个 `reportError` 调用点**无一传入**；Rust 侧无字段；`TurnTraceGuard::trace_id()` 生产调用点 0。⇒ Issue 与 Trace 无法互跳 |
| 指纹质量（fingerprint/culprit） | BROKEN（P1） | 客户端不送 `fingerprint`/`culprit`；`capture_error` 的 `stacktrace` 恒 `None`、panic 仅单帧且 `lineno=None` |
| 前端 fetch 兜底路径 | BROKEN（P1） | `INGEST_URL` 默认 `""` ⇒ 早退零 fetch；发布流水线只注入 `VITE_PONYSENTRY_CLIENT_TOKEN`，**不注入 `VITE_PONYSENTRY_INGEST_URL`** ⇒ 浏览器部署与 IPC 失败降级均 100% 落空 |
| 重试契约 | BROKEN（P2） | 两侧均无重试/backoff，与 `docs/architecture/ponysentry-integration-report.md:179` 承诺不符 |
| 实测落地 | CONNECTED（部分） | PonySentry 中 `project=pony-agent` 共 **9 条** issue（`TurnExecutionFailed` / `IpcCommandError` 系列）⇒ 后端+IPC 主链真实可用 |

### 2.3 Logs（→ Loki）

| 环 | 判定 | 结论与证据 |
| :--- | :--- | :--- |
| 输出面 | CONNECTED | `log_json` 输出 stdout 单行 JSON（`otel/log.rs:17-33`），键固定 `level/message/ts/trace_id` |
| **trace_id 实填** | **BROKEN（P1）** | 全部 3 个调用点均传 `None`（`otel/mod.rs:438`、`:1470`、`src-tauri/src/lib.rs:1240`）⇒ 恒 `null`。`Loki↔Tempo` 互跳的唯一桥**未接通** |
| **桌面端日志落地/上送** | **BROKEN（P1）** | 无 `tauri-plugin-log`、无文件 appender；`windows_subsystem="windows"` ⇒ 无控制台可采；OTLP 只开 trace 特性、无 logs 信号；全仓无 loki/promtail ⇒ **桌面端日志既不上送也无法事后取证** |
| 等级控制 | BROKEN（P2） | 无 `RUST_LOG`/`EnvFilter`；`log_json` 无 level 过滤 |
| 脱敏覆盖 | BROKEN（P2） | `log_json` 本身不过 `sanitize`，`lib.rs:1240` 的 `{err}` 与 `mod.rs:438` 的 `{raw_ratio}` 明文直出 |
| Loki 现状 | NA/UNKNOWN | 审计脚本 `L3 MISSING` 因其查的是 k8s `namespace` 标签；桌面端无 pod ⇒ 结构上不适用于该维度 |

### 2.4 Metrics（→ Prometheus）

| 环 | 判定 | 结论与证据 |
| :--- | :--- | :--- |
| `/metrics` HTTP 暴露 | **NA** | 桌面端无常驻 HTTP server；源码 `/metrics` 字面量 0 命中；全部 `TcpListener` 在 `#[cfg(test)]` |
| 抓取目标（ServiceMonitor 等） | **NA** | 仓库无任何 k8s 清单；对桌面端不存在抓取面 |
| OTel Metrics 信号 | NA（可选） | 三件套只开 `trace` 特性，无 `MeterProvider` |
| 本地统计面 | CONNECTED | `ModelMonitorPage` / `TelemetryPage` / `HomeStatusPanel` 只读本地数据（SQLite + invoke），无对外导出 |

> ⚠️ **审计脚本误报更正**：`audit-project.sh` 的 M1/M2 判 PASS 是**文档误命中**——`g()`/`gh()` 扫全仓任意文件且正则 `/metrics` 命中 `management/task-system/01_TASK_BOARD.md:151` 的「trace/metrics 二级化」字样。桌面端 Metrics 维度应判 **NA**，不应是 PASS。

---

## 3. 断链清单（按严重度）

### P0（会让"整条链路看起来接了、实际零数据"）
1. **Tracing 发布态必然零导出**：门禁默认关 + 端点/token 仅来自 env + 发布流水线不注入 `OTEL_*`/`PONY_OTEL_TOKEN`。三因叠加 ⇒ 用户即使打开开关也发不出去。**证据**：`otel/mod.rs:98,107,116`、`.github/workflows/release.yml:25-28`、Tempo 投递前空标签集。
2. **Tempo↔Sentry 互跳断裂**：`trace_id` 从未被传入 Issue，Rust 侧无字段。**证据**：`telemetry.ts:286,308`；6 个调用点；`models.rs:26-36`。

### P1（数据质量/因果链缺口）
3. **W3C `traceparent` 不注入**：`inject_traceparent_header` / `W3cTraceContext` 只有定义与测试调用，生产零调用点；provider 出网请求只设鉴权头（`provider/mod.rs:1447-1456,1831-1840`）。且 otel 侧**未暴露 `span_id`**（只有 `trace_id`），即便接线也无法构造完整 `traceparent`——属"半成品 API"。
4. **日志 `trace_id` 恒 `None`**：`otel/mod.rs:438`、`:1470`、`lib.rs:1240` 三个调用点全传 `None`；`TurnTraceGuard::trace_id()` 生产零消费者 ⇒ Loki↔Tempo 互跳桥未接通。
5. **桌面端日志无落盘、无上送**：无 `tauri-plugin-log`、无文件 appender；release `windows_subsystem="windows"` 无控制台；OTLP 只开 trace 特性 ⇒ 崩溃现场无法事后取证。
6. **`event_id` 在桌面 IPC 路径被丢弃**：Rust `models.rs:26-36` 无该字段，IPC 反序列化静默忽略 ⇒ 幂等键在桌面端不存在。
7. **Issue 指纹质量不足**：不送 `fingerprint`/`culprit`；`capture_error` 的 `stacktrace` 恒 `None`、panic 仅单帧且 `lineno=None` ⇒ 服务端聚合退化。
8. **前端 fetch 兜底死路**：`VITE_PONYSENTRY_INGEST_URL` 未在发布流水线注入，`INGEST_URL` 默认 `""` 早退。
9. **退出不 flush**：托盘强退/SIGKILL 丢失 5000ms 批窗口内 span；`init` 失败无重试。
10. **`langfuse.session.id` 未下沉到子 span** ⇒ Langfuse 观察树按会话归集不稳。

### P2（工程卫生/一致性）
11. `beforeSend` 无生产注册点（清洗默认不生效）。
12. 无 Vue 错误边界（无局部降级）。
13. 日志无等级控制（无 `RUST_LOG`/`EnvFilter`）、`log_json` 未过脱敏（`lib.rs:1240` 的 `{err}` 明文）。
14. Issue 无重试/backoff，与 `docs/architecture/ponysentry-integration-report.md:179` 承诺不符。
15. `record_stream_debug_metrics` 为死命令（无调用者）。
16. `langfuse.user.id` 为死键（三处恒 `None`）。
17. 文档与实现不符两处：`docs/architecture/agent-trace-pipeline.md:50` 声称明文 HTTP 豁免「集群内主机」，实现只豁免 `localhost/127.0.0.1/::1`（`otel/mod.rs:251-262`）；`agent.turn.speed.tokens_per_second` 契约标 reserved 但 `record_turn_speed` 零调用。
18. 审计脚本 `audit-project.sh` 存在**文档误命中**（`g()`/`gh()` 未排除 docs）⇒ M1/M2 假 PASS，需修脚本而非修项目。

---

## 4. 建议修复顺序（最小闭包）

| 序 | 修哪里 | 动作 | 验收判据 |
| :--- | :--- | :--- | :--- |
| 1 | `otel/mod.rs` + 设置模型 + 发布流水线 | 端点/token 改为「settings 持久化 + env 覆盖」，发布注入 `PONY_OTEL_TOKEN` 与公网端点默认值 | 打包产物开开关后，Tempo 出现 `client.service.name=pony-agent` 的真实 trace |
| 2 | `otel/mod.rs` 公开 `current_trace_id()`；`telemetry.ts` + `ponysentry` 模型 | Issue 链路回填 `trace_id`；Rust payload 增 `event_id`/`trace_id` 字段 | PonySentry issue 详情含 trace_id 且能跳 Tempo |
| 3 | `otel/mod.rs` 公开 `span_id()`；`provider/mod.rs` 出网点 | 真正注入 `traceparent`（复用已有 `inject_traceparent_header`） | 出站请求 header 含 `traceparent`（wire 级断言） |
| 4 | `otel/log.rs` 调用点 + 桌面日志落盘 | `log_json` 传真实 trace_id；加文件 appender（轮转）用于事后取证 | 日志行 `trace_id` 非 null；本地日志文件可读 |
| 5 | `RunEvent::ExitRequested` / 托盘退出 | 退出前 `force_flush`（有界 3s） | 强退场景下末批 span 不丢 |
| 6 | `ponysentry/models.rs` | 补 `fingerprint`/`culprit`/`stacktrace` 填充 | 服务端返回无 `missing_culprit` warning |
| 7 | `main.ts` | 注册默认 `beforeSend`（脱敏 + 裁剪） | 单测断言默认 beforeSend 生效 |
| 8 | `observability-onboarding/scripts/audit-project.sh` | `g()`/`gh()` 排除 docs 与 management（改用 `gsrc()` 同款排除） | 对 pony-agent 巡检 M1/M2 判 NA 而非 PASS |

> Metrics 维度对桌面端判 **NA**（无抓取面），不建议为了"变绿"而伪造 `/metrics` 端点；如需运行指标，走 OTel Metrics 信号或本地面板导出。
