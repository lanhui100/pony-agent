# Wave43 · Lead 裁决书（S1~S6 + 写域分权）

> 裁决人：Lead（一票裁决）。生效即冻结，波内不得就地修改；如需变更走契约演进门禁。

## A1（S1）模块路径：改走 `agent/otel/`

- 事实：`crates/pony-agent-core/src/agent/telemetry.rs` **已存在**（34KB，`agent/mod.rs:56 pub mod telemetry;`）。Rust 不允许 `telemetry.rs` 与 `telemetry/` 目录并存。
- 裁决：新增 **`crates/pony-agent-core/src/agent/otel/`** 目录（`mod.rs` + `log.rs`）；`crates/pony-agent-core/src/agent/mod.rs` 增加一行 `pub mod otel;`（该行纳入 task-2 写域）。
- `agent/telemetry.rs` **保持原样不动**，不重构、不搬移。

## A2（S2）纯属性映射 seam（禁止测试做 wire 字节扫描）

在 `agent/otel/mod.rs` 冻结以下中性类型与纯函数（**不泄漏 OTel 类型**，Executor 内部转换成 `opentelemetry::Value`）：

```rust
pub enum AttrValue { Str(String), I64(i64), F64(f64), Bool(bool) }

pub struct TurnSpanInput {
    pub session_id: String,
    pub user_id: Option<String>,
    pub turn_id: String,
    pub sequence: u64,
    pub phase: String,
}
pub struct LlmSpanInput {
    pub provider_kind: String,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub operation: String,
}
pub struct ToolSpanInput {
    pub tool_name: String,
    pub status: String,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

pub fn turn_span_attributes(input: &TurnSpanInput) -> Vec<(String, AttrValue)>;
pub fn llm_span_attributes(input: &LlmSpanInput) -> Vec<(String, AttrValue)>;
pub fn tool_span_attributes(input: &ToolSpanInput) -> Vec<(String, AttrValue)>;
```

属性键契约（冻结）：
- turn：`langfuse.session.id`、`langfuse.trace.name`、`gen_ai.operation.name`="agent.turn"、`agent.turn.id`、`agent.turn.sequence`、`agent.turn.phase`、`langfuse.user.id`（有则带）
- llm：`gen_ai.system`、`gen_ai.request.model` **与** `langfuse.observation.model.name`（同值双写，避免口径歧义）、`gen_ai.usage.input_tokens`、`gen_ai.usage.output_tokens`
- tool：`gen_ai.tool.name`、`langfuse.observation.type`="tool"、`agent.tool.status`、`agent.tool.duration_ms`、`agent.tool.error`（有则带）
- 缺失值一律**不输出该键**（禁止写空串占位，避免 Langfuse 侧出现空 observation）

## A3（S3）flush 与可重入

```rust
pub fn force_flush(timeout_ms: u64) -> Result<(), String>;
```
契约：`shutdown()` 之后允许再次 `init()`（全局 provider 可重建）；`is_reporting_enabled()` 在未 init 时返回 false；`force_flush` 未 init 时返回 `Err`。三个行为都必须写进 doc 注释，Test Agent 据此断言。

## A4（S4）手动导出 core 级 seam

```rust
pub fn force_export_session_trace(
    session_id: &str,
    traces: &[crate::agent::session::TurnTraceRecord],
) -> Result<usize, String>;
```
契约：**旁路全局 gate**（显式用户动作）；返回成功导出的 turn 数；**不得**修改 `is_reporting_enabled()` 的末态（翻转后必须复原）。`HostControlPlane::upload_session_trace` 仅做转调，保持 `Result<usize, String>`。

## A5（S5）前端 testid 冻结

- `config-agent-trace-reporting-switch`（Switch 根元素）
- `config-agent-trace-privacy-notice`（说明文案容器）
- `telemetry-upload-independent-note`（观测页「与全局开关互不影响」提示）

## A6（S6）**驳回**「需加 rename_all」，以证据为准

- `AppSettings` 与 `AppSettingsStorage` **均已** `#[serde(rename_all = "camelCase")]`：`crates/pony-agent-core/src/agent/app_settings.rs:57-60`（AppSettings）、`:68-75`（AppSettingsStorage）。
- 因此 `agent_trace_reporting` 的落盘键**天然为** `agentTraceReporting`，与 D7 一致，**不需要 ADR**。
- OT-8 断言按此修正：raw JSON 键为 `agentTraceReporting`；legacy 容错指「老 settings.json 无该键 → 读回 false」。

## A7 写域分权（测试文件归 Test Agent 独占）

- Test Agent 独占：`crates/pony-agent-core/tests/**`、`tests/**`。
- task-3 写域**移除** `crates/pony-agent-core/tests/otel_`；Executor 不得写入任何 `tests/` 路径。
- Executor 允许在 `src/**` 内写 `#[cfg(test)]` 实现级单测（不构成验收门禁），但**不得**触碰 `tests/` 目录。
- D6（PonySentry trace 下线）击穿的 3 个存量测试文件（`ponysentry_trace_test.rs` / `trace_auto_report_test.rs` / `ponysentry_manual_upload_test.rs`）由 **Test Agent** 按新契约重写或删除；`ponysentry_credential_gate*.rs` / `ponysentry_telemetry_test.rs` 只 import 保留面，**保留不动**（作为 OT-7 保留面活体证明）。

## A8 采样率

D4/plan §8 U2：桌面端默认 `sampling_ratio = 1.0`（用户显式授权 + 单机低频），可由 env `OTEL_TRACES_SAMPLER_ARG` 覆盖。Test Agent 不对概率采样做断言（只断言配置透传）。
