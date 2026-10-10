# 0025 将 Sentry 问题收集下沉至 Harness 宿主层与 Agent Trace 解耦

Status: proposed

## 背景

在前期实现中（参考 ADR 0024 及 `docs/architecture/ponysentry-integration-report.md`），Pony Agent 接入了 PonySentry 服务，承担了双重职责：
1. **Agent Trace 上报**：将智能体执行过程中的 Session/Turn Trace、Token 消耗、工具调用明细与评测状态（`EvalStatus`）上报至 `/api/v1/traces`，服务于模型行为调优、质量标注与评测回流。
2. **Sentry 异常与崩溃捕获**：在 Agent Core 内部（如 `turn_flow.rs`、`provider/mod.rs`、`turn_persist.rs`）直接侵入式调用 `ponysentry::capture_error`，将 Turn 失败、Watchdog 超时与 Provider 重试耗尽作为通用 Issue 上报。

这种设计带来了职责边界混淆和架构耦合问题：
- **概念混淆**：基础设施/宿主环境异常（Panic、OOM、网络不可达、进程被杀）与智能体业务语义失败（模型拒答、工具参数 Schema 校验不通过、Prompt 偏航）混在一起；
- **核心逻辑受污染**：Agent Core 是通用调度和执行内核，直接依赖特定的外部 Sentry 客户端违背了关注点分离原则；
- **观测责任倒置**：Sentry 作为问题与故障收集系统，其职责本质上是监控**执行容器/宿主（Harness / Shell）**的健康度，而 Agent 自身的运行轨迹应收敛为标准结构化 Trace 吐出。

## 决策

1. **职责严格划分**：
   - **Agent 层（业务语义与可观测性）**：
     - Agent Core 专注业务调度与 Trace 聚合，通过结构化事件总线（Event Envelope / Stream Event）向外吐出执行结果（包括失败原因与 Trace 记录）；
     - 移除 Agent Core 内部向通用 Sentry Issue 直接发送异常的主动耦合点（`turn_flow`、`provider`、`turn_persist` 内部的 `capture_error` 侵入调用）；
     - Agent 的 Trace 上报机制（`/api/v1/traces`）由专职的 Trace 聚合器负责，不干扰宿主故障收集。
   - **Harness / 宿主层（基础设施与故障兜底）**：
     - 宿主容器（Tauri 桌面外壳 / CLI / Harness Web）负责未捕获异常、进程级 Crash（Panic hook）、系统资源异常等基础设施级问题收集；
     - 宿主通过监听 Agent 吐出的顶层失败事件或捕获宿主与 Agent 之间的通信异常，决定是否将不可恢复的基础设施级严重故障上报至 Sentry。
2. **Agent 核心纯粹化**：
   - `pony_agent_core` 的 `ponysentry` 模块收敛为其核心的 Trace/Telemetry 导出器，逐步废弃核心内部零散的业务级 `capture_error` 侵入点，改由标准的错误返回和事件传播机制暴露给调用方。
3. **保持向后兼容**：
   - 保留 Tauri 壳层的 `ponysentry_capture` 与 Panic Hook 作为宿主层故障收集设施，确保桌面宿主崩溃防护不降级。

## 影响

- **代码影响**：清理 `turn_flow.rs`、`provider/mod.rs`、`runtime/turn_persist.rs` 中冗余的 `ponysentry::capture_error` 业务打点，强化 Trace 管道中的完整错误承载。
- **治理与验证**：更新相关架构文档与契约测试，确保 Agent Trace 上报与宿主 Sentry 收集职责明晰。
