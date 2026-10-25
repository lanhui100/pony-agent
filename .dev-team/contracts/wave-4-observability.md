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
