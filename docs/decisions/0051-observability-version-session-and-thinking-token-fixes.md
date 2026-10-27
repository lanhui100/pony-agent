# ADR 0051：可观测性上报三项修正（版本单一来源、治理 span 会话归属、思考/缓存 token 出口）

Status: implemented

## 背景

2026-10-27 的真实会话审计（[observability-audit-2026-10-27](../audit/observability-audit-2026-10-27.md)）
在四维归属全绿的前提下，暴露三处「值错/缺项」：

1. **版本归因错位**：OTel resource `service.version` 与 PonySentry `release` 取 core crate 的
   `CARGO_PKG_VERSION`（当次 trace 报 `0.1.135`），与安装包发布线（`0.1.162`）不是同一计数线；
   Langfuse `trace.version` 与 PonySentry `last_release` 因此无法与 GitHub Release 对齐。
2. **治理 span 缺会话归属**：`governance.decide` 只写 `gen_ai.operation.name`，同一 trace 9 个 span
   只有 8 个命中 `langfuse.session.id` 过滤；治理/配额决策在语义轨完全不可见。
3. **思考与缓存命中 token 无出口**：`TokenUsage` 已解析 `reasoning_tokens` 与
   `cache_hit_input_tokens`，但 `llm.chat` 属性表只有 input/output，导致 high 思考档会话的用量/成本
   口径系统性偏小。

## 候选方案

- **版本：在 `OtelConfig` 增加 `service_version` 字段 / 由宿主写进程 env**：前者破坏所有既有
  `OtelConfig` 结构体字面量（含冻结测试），后者在 Tauri 多线程启动后调用 `set_var` 语义不安全、
  且与「配置显式传入」的既有装配风格不一致。**否决**。改为新增
  `init_with_service_version(config, Option<&str>)`（additive），`init(config)` 等价传 `None`。
- **PonySentry 版本：新增 settings 键 / 新增公开 `with_release` 构造器**：settings 通道是用户配置面，
  版本不是用户可调项；构造器方案无需改 settings 结构、可直接单测。**采纳** `with_release`。
- **思考/缓存属性名：自定义 `gen_ai.usage.reasoning_tokens|cached_input_tokens` / OTel semconv
  标准名**：自定义名不在 semconv registry，Langfuse 只把 `gen_ai.usage.*` 原样落成自定义 usage detail、
  不做去重；标准名 `gen_ai.usage.reasoning.output_tokens` / `gen_ai.usage.cache_read.input_tokens`
  会被 Langfuse ingest 正确归并（input 减 cache_read、output 减 reasoning）。**采纳标准名**。
- **缓存键全协议上报 / 仅 OpenAI 协议族上报**：Anthropic 的 `input_tokens` 不含 cache_read，
  照报会被 Langfuse 减成偏小值。**采纳协议门控**（openai-completions / openai-responses 上报，
  anthropic 与未知标签 fail-closed 不上报），登记为已知限制。
- **根 `agent.turn` 聚合 / 只报子 span**：只报子 span 会让整轮成本需跨 span 拼装；根 span usage
  聚合键不经过 `turn_span_attributes` 纯函数，不破坏 AT2 的 7 键冻结集。**采纳根聚合**（两个固定键）。

## 决策

1. **版本单一来源**：`otel::init_with_service_version` 的 `service_version` 三级优先
   （显式入参 > env `PONY_OTEL_SERVICE_VERSION` > `CARGO_PKG_VERSION`）；显式入参与
   `PonySentryConfig::with_release` 走同一合法化规则（trim / 拒控制字符 / ≤64 字节 / 不合规视为未设置），
   env 通道保持既有 trim-only 语义。Tauri 装配点以 `app.package_info().version` 注入两条轨。
2. **治理 span 会话归属**：`start_governance_decide_span` 复用 `push_active_session_attribute`
   （无活跃 turn/空 session 不输出键），会话过滤 9/9，治理决策进入 Langfuse 语义轨。
3. **思考/缓存 token 出口**：新增纯函数 `llm_usage_detail_attributes`（有值才写、`Some(0)` 有效、
   `u64→i64` 饱和到 `i64::MAX`），`LlmSpanGuard::record_usage_details` 在 `Drop` 写子 span，
   `TurnTraceGuard::drop` 写整轮聚合；`turn_flow.rs` 4 个 provider 调用缝接线，cache 键协议门控。

## 影响

- **契约**：additive 公共 API 3 项（`init_with_service_version`、`record_usage_details`、
  `with_release`）+ 1 个纯映射函数 `llm_usage_detail_attributes`；`init(config)`、`OtelConfig` 字段、
  `LlmSpanInput` 形状不变；无新依赖、无新 env、无新网络出口。
- **宿主**：`src-tauri` 装配点改走 `init_with_service_version(..., Some(app 版本))`；
  `load_app_settings` 增加 `AppHandle` 参数并以 `with_release` 注入 PonySentry release。
  前端 `invoke("load_app_settings")` 无参调用不受影响（Tauri 自动注入）。
- **已知限制**：① Anthropic 语义下 cache 键不上报（follow-up：需要同时把 cache 计入 input 口径）；
  ② CLI/TUI 仍报 core crate 版本（无安装包版本可注入）；③ 未知协议标签 cache fail-closed；
  ④ 显式注入与 env 的 trim 差异（env 保留原值）。
- **测试**：新增冻结验收 `crates/pony-agent-core/tests/observability_reporting_fixes_test.rs`
  （11 例，含 L2-AT 对抗与测试侧最小 protobuf 解码器）；OT-8e 静态扫描授权接受新装配入口一处；
  既有 AT2 7 键冻结集与 wave49 根 usage 断言不被打破。绿相期发现并修复测试缺陷 TD-1
  （`l2at_concurrent_init` 的 `Barrier` 参与方不匹配死锁，红相被 panic 短路掩盖）；
  残留风险：该例若实现回退为 panic，主线程可能 hang 而非快速失败（已登记待后续根治）。
- **文档**：[agent-trace-pipeline](../architecture/agent-trace-pipeline.md) §2.1 span 语义同步；
  [observability.yaml](../observability.yaml) 追加 wave-51 归属登记（四维归属不变）。
