# Wave-4 冻结契约 — 可观测性断链修复与端到端链路治理

本契约用于约束与指导 Wave-4 可观测性治理（OTel 配置供给、Issue 与 Trace 互跳、traceparent 级联透传、结构化日志与退出 flush）。

---

## C1 OTel 配置供给（修 P0-1）

### 契约条款
1. **现网事实**：端点与 token 仅来自进程 env（`OTEL_EXPORTER_OTLP_ENDPOINT` / `PONY_OTEL_TOKEN`），默认端点是集群内网 DNS，发布流水线不注入，导致桌面端永不导出。
2. **来源优先级**：OTel 配置解析必须严格遵循 **settings 持久化 > 进程 env > 内置默认**。
3. **Settings 字段契约**（camelCase，持久化于 `settings.json`）：
   - `otelEndpoint`: `string`，用户配置的 OTLP 端点 URL；
   - `otelToken`: `string`，鉴权 Bearer/Header Token；
   - `otelServiceName`: `string`，服务名，默认值为 `"pony-agent"`；
   - `otelSamplingRatio`: `number` (0.0 .. 1.0)，采样率，默认值为 `1.0`。
4. **内置默认端点**：桌面端默认端点变更为公网 `https://otel.ponyjob.top`；集群内直连仍可经由环境变量覆盖。
5. **凭据安全与零 egress**：
   - `otelToken` 仅作 HTTP 请求头注入（如 `Authorization: Bearer <token>`），严禁落盘到任何日志、面包屑或调试回显。
   - `otelToken` 为空串或未配置时，等同未配置（触发零 egress，不发起外部导出请求）。

### 优先级
P0

### 拒绝条件
1. 传入明文 `http://` 协议的非本地回环端点（非 `127.0.0.1`、`localhost`、`::1`）且携带有效 token 时，`init` 必须直接返回 `Err` 拒绝初始化（保留现有安全防线）。
2. `otelSamplingRatio` 不在 `[0.0, 1.0]` 区间内时拒绝采纳。

### 验收判据
1. 当 `settings.json` 显式配置 `otelEndpoint` 与 `otelToken` 时，其生效权高于系统环境变量。
2. 无环境变量且无 settings 时，回退到公网默认端点 `https://otel.ponyjob.top`。
3. 测试套件或调试日志中全局断言 token 不以明文出现。

---

## C2 Issue ↔ Trace 互跳（修 P0-2）

### 契约条款
1. **Payload 字段扩展**：PonySentry ingest payload 与 IPC 数据传输契约在前端和 Rust **双侧**显式声明以下字段：
   - `event_id`: 32 位小写十六进制字符串（无连字符）；
   - `trace_id`: 32 位小写十六进制字符串（OTel Trace ID）。
2. **双侧反序列化保全**：Rust 侧反序列化结构体必须包含 `event_id: Option<String>` 与 `trace_id: Option<String>`，禁止在 serde 解析时静默丢弃。
3. **真实 Trace 绑定**：
   - 取值为「当前活跃 turn 的 OTel trace_id」；
   - 当前环境若无活跃 trace/turn，值必须为 `None`（或 null），**严禁使用任何伪造占位值**（如全 0 或 dummy-trace-id）。

### 优先级
P0

### 拒绝条件
1. `event_id` 不符合 32 位十六进制正则 `^[0-9a-f]{32}$`（带连字符或长度非 32 时视为非法）。
2. 在无活跃 trace 上下文时人为注入非 null 的假 trace_id。
3. Rust 端接收到 IPC payload 之后将 `trace_id` 擦除或丢弃。

### 验收判据
1. 异常触发时生成的 PonySentry issue 详情中包含有效的 `trace_id`。
2. 依据该 `trace_id` 可在 Tempo/Langfuse 检索到同 ID 的完整链路 span。

---

## C3 traceparent 透传（修 P1）

### 契约条款
1. **接口暴露**：`otel` 模块必须公开暴露上下文查询函数：
   - `current_span_id() -> Option<String>`：返回 16 位小写十六进制 span ID；
   - `current_traceparent() -> Option<String>`：返回标准 W3C 格式 `00-<trace_id>-<span_id>-01`。
2. **全面出网注入**：**所有** outbound `reqwest` 出网请求（包含 provider 大模型出网、PonySentry HTTP ingest）在存在活跃 span 时，必须设置 HTTP 请求头 `traceparent`。
3. **真实性与 Fail-Closed**：
   - 无活跃 span 时，不注入 `traceparent` 请求头（严禁伪造）。
   - 注入函数必须严格校验 trace_id 与 span_id 的长度与十六进制合法性，校验失败则静默不注入（Fail-Closed，绝不注入脏头）。

### 优先级
P1

### 拒绝条件
1. 在无活跃 span/trace 上下文时生成或发送 `traceparent` 请求头。
2. 格式不合规（如版本号非 `00`、长度错误、含大写或非法字符）的 traceparent 头部被发送出网。

### 验收判据
1. 出网 HTTP 流量（Wire 层面）包含符合 W3C 规范的 `traceparent: 00-[0-9a-f]{32}-[0-9a-f]{16}-01`。
2. 无活跃 turn 时发出的请求 headers 中无 `traceparent` 字段。

---

## C4 日志（修 P1）

### 契约条款
1. **Trace 关联绑定**：`log_json` 结构化日志中的 `trace_id` 字段在存在活跃 turn 时必须写入真实的 32 位十六进制 trace_id，不再恒定为 `null`；无活跃 turn 时显式为 `null`。
2. **桌面端文件落盘**：
   - 桌面端运行时必须将日志同步追加落盘至 `<app_data_dir>/logs/pony-agent.log`；
   - 实行单文件追加与 512KiB (`524288` 字节) 轮转上限约束；
   - 文件写入异常时降级为仅控制台/忽略，禁止阻塞或导致主链路 panic。
3. **脱敏保护**：所有经由 `log_json` 输出的日志内容，在落盘或输出前必须经由 `ponysentry::sanitize`（或等价零信任脱敏函数）清洗敏感凭据。

### 优先级
P1

### 拒绝条件
1. 在存在活跃 turn 时，`log_json` 输出的 `trace_id` 字段仍为 `null`。
2. 日志单文件体积超过 512KiB 且未执行截断轮转。
3. 未经过脱敏处理的明文 secret（如 API key / Token）出现在日志输出中。

### 验收判据
1. 观察到的结构化日志 JSON 中，turn 周期内所有输出项均具有统一的非空 `trace_id`。
2. 磁盘日志文件在体积达到 512KiB 时成功触发轮转且文件读写正常。

---

## C5 退出 flush（修 P1）

### 契约条款
1. **退出拦截与强制刷盘**：在运行时接收到 `RunEvent::ExitRequested` 以及 `RunEvent::Exit` 信号时，必须触发 OTel exporter 的 `force_flush`。
2. **超时约束**：退出时执行的 `force_flush` 必须在硬超时上限 **≤ 3000ms** 内返回，严禁因对端网络悬挂导致进程退出卡死。
3. **幂等性保障**：多次接收到退出事件时，flush 操作必须保持幂等，不得重复初始化或产生死锁/panic。

### 优先级
P1

### 拒绝条件
1. 退出 flush 操作阻塞超过 3000ms。
2. 重复触发 `Exit` 事件时抛出 panic 或发生死锁。

### 验收判据
1. 模拟网络黑洞或慢速端点时，触发退出事件并在 3000ms 容差范围内完成进程优雅退出。
2. 正常退出时，缓冲区中尚未发出的 spans 能够在退出前成功投递到 OTLP 收集器。

---

# Wave-4.1 v2 契约修订（回应 L3-A / L3-B 审查裁决）

> 本节为 Wave-4.1 v2 增补与裁决修订段，保留前文 v1 原文作为历史基线。凡本节与 v1 存在差异之处，一律以本节 v2 裁决为准。

## R1 [C2 修订] 仅允许「导出态」trace_id 绑定 Issue（对应 L3-A BLOCKING）

### 契约条款与概念裁决
1. **概念正交定义**：
   - `exported_trace_id()`：仅在当前存在真实 SDK root span，且状态满足 `is_valid() && is_sampled()` 时返回 32 位十六进制字符串；未配置导出、门禁关闭、未采样或非真实导出 span 时恒为 `None`。
   - `current_trace_id()`：当前 turn 的相关性 ID（Correlation ID，十六进制 32 位）。用于本地日志串联与跨服务 `traceparent` 上下文传播，**不保证已向 Tempo 导出**。
2. **Issue 绑定约束**：
   - `IngestPayload.trace_id` **只能**取 `exported_trace_id()`；取不到时必须显式为 `None`（前端传递 null / undefined，Rust 反序列化为 `None`）。
   - **绝对禁止**使用本地相关性 ID 填充 `issue.trace_id`，以杜绝 Issue 指向 Tempo 中根本不存在的虚假 trace 导致排障断链。

### 优先级
P0

### 拒绝条件
1. 在 OTel 门禁关闭、非导出态或未采样时，向 PonySentry Issue 注入任何非 None 的 `trace_id`。
2. 使用 `current_trace_id()` 或伪造占位串填充 `IngestPayload.trace_id`。

### 验收判据
1. 产生包含 `trace_id` 的 Issue 时，该 ID 在 Tempo 中**100% 具备可检索的真实链路**。
2. 当 OTel 上报门禁关闭或无真实导出时，PonySentry 产生的 Issue 详情中 `trace_id` 必定为 null/None。

---

## R2 [C2 前端修订] 前端上报数据源归一（对应 L3-B BLOCKING-1）

### 契约条款
1. **下发与消费契约**：
   - 前端**唯一且法定**的活跃 trace 来源为 Rust 下发的 `TurnTraceRecord.traceId`（DTO 属性命名为 camelCase `traceId`）。
   - 前端所有 6 处 `reportError` 错误上报入口（以及 beforeSend 拦截管线）仅能读取当前活跃 turn 的 `TurnTraceRecord.traceId`。
   - 若 `TurnTraceRecord.traceId` 为空或当前无活跃 turn，前端上报时不得传递 `trace_id` 字段（保持 undefined / 序列化时省略或 null）。

### 优先级
P0

### 拒绝条件
1. 前端自行生成随机 UUID 或通过其他前端本地变量填充 `reportError` 的 `trace_id`。
2. 在 `TurnTraceRecord.traceId` 存在的情况下，前端 `reportError` 丢失该关联。

### 验收判据
1. 单元测试覆盖：终态 turn 携带有效 `traceId` 时，触发 `reportError` 生成的 payload 中 `trace_id` 严格等于该值。
2. 单元测试覆盖：无活跃 turn 或 `traceId` 未下发时，`reportError` payload 中无 `trace_id`。

---

## R3 [新增 C6] Secret 全生命周期防护与脱敏契约（对应 L3-B BLOCKING-2）

### 契约条款
1. **Secret 绝缘面**：
   - `otelToken`（以及所有类凭据敏感信息）绝对禁止出现在：标准/文件日志、面包屑（Breadcrumbs）、IPC 错误上报的 `extra`/`tags`、Rust 类型 `Debug`/`Display` 输出、遥测公开 payload 中。
2. **前后端统一脱敏正则**：
   - 脱敏匹配规则从原有「严格前缀/完全匹配」全面升级为**键名子串不区分大小写匹配**：
     ```regex
     (?i)(token|secret|password|credential|authorization|api_key|apikey)
     ```
   - 前端（TypeScript）与 Rust 核心层必须使用语义完全等价的子串脱敏正则。凡键名命中该正则的字段，其值在序列化/落盘/上报前一律替换为 `"[REDACTED]"`（或既有 `"[REDACTED_SECRET]"`）。
3. **配置文件权限与平台兼容**：
   - `settings.json` 在包含 `otelToken`（或其它凭据）落盘时，POSIX 系统下文件权限必须强制设置为 `0o600`（仅所有者读写）。
   - Windows 平台说明：Windows 下降级为依赖用户个人 AppData 目录的默认 ACL 隔离，捕获权限错误并保证平稳写入，禁止因文件权限不支持而 panic。

### 优先级
P0

### 拒绝条件
1. 任何形式的日志、面包屑或 Issue extra 中出现未掩码的 `otelToken` 原文。
2. POSIX 环境下写入含有 token 的 `settings.json` 时权限为 `0o644` 或更宽。

### 验收判据
1. 输入包含 `{ otelToken: "dummy-live-secret-value-12345" }` 的对象经过上报与日志链路后，输出必为 `[REDACTED]` 或 `[REDACTED_SECRET]`。
2. Rust 侧 `AppSettings` 及 `OtelSettingsSnapshot` 的 `Debug` 格式化输出中，token 字段均显示为 `"[REDACTED]"`。

---

## R4 [C3 / C5 收紧] Header 注入幂等与退出预算配额（对应 L3-A ADVISORY）

### 契约条款
1. **C3 注入语义与重复防范**：
   - 请求头注入必须采用 **`HeaderMap::insert` 语义**（覆盖式写入），严禁使用 `append` 导致请求中产生多条重复的 `traceparent` 头。
2. **C3 第三方 URL（WebFetch）注入裁决**：
   - **裁决结论**：保持对所有出网请求（含 WebFetch）注入 `traceparent`。
   - **架构取舍理由**：遵循 W3C TraceContext 规范，透传 traceparent 属于分布式追踪的标准最小信息披露，便于在跨系统协作及出网网关（如反向代理/代理集群）日志中联合检索定位；`traceparent` 仅含十六进制 ID 与采样位，绝不携带任何业务载荷或凭证，安全风险可控。
3. **C5 退出超时预算细化（三段合计 ≤ 3000ms）**：
   - 进程退出硬预算上限为 3000ms，按以下建议配额分配三阶段：
     - ① `RunEvent::ExitRequested`: 有界超时 ≤ 1000ms；
     - ② `RunEvent::Exit` flush: 有界超时 ≤ 1000ms；
     - ③ `shutdown()` 清理释放: 有界超时 ≤ 1000ms。
   - 三段操作必须各自及全局保持**绝对幂等**，重入调用直接短路返回，不得发生阻塞累加或死锁。

### 优先级
P1

### 拒绝条件
1. Outbound 请求头中出现两条或以上的 `traceparent`。
2. 退出总耗时超过 3000ms 导致进程无法正常终止。

### 验收判据
1. 网络抓包验证发往外部的 HTTP 请求中仅存在单条合法的 `traceparent`。
2. 进程退出路径在网络不可达时的单测实测总耗时 ≤ 3000ms。

---

## R5 [C4 语义澄清] 结构化日志 trace_id 契约定位

### 契约条款
1. **语义定位**：
   - 日志中的 `trace_id` 严格定义为「**本地执行相关性 ID**（Correlation ID）」，其生命周期绑定当前 turn。
   - **明确澄清**：结构化日志中的 `trace_id` 仅保证「同一 turn 内部所有日志事件的关联一致性」，**不承诺**该 trace_id 一定能在 Tempo/Langfuse 检索到（例如采样未命中、上报门禁关闭或纯离线模式运行时）。
2. **与 Issue trace_id 的边界隔离**：
   - Issue 的 `trace_id`：代表**导出态 trace**，必须在 Tempo 可查（见 R1）。
   - Log 的 `trace_id`：代表**本地逻辑流关联**，服务于本地控制台排查与 `pony-agent.log` 日志归集。

### 优先级
P1

### 验收判据
1. 在无外部导出（门禁关闭）状态下，turn 周期内的 `log_json` 依然正常输出统一的 32 位十六进制 `trace_id`。
2. 文档与代码注释中均已消除「日志 trace_id 必定可在远端 APM 检索」的误导性表述。

