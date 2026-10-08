# 0024 PonySentry Agent Trace 上报契约与生命周期状态机

Status: implemented

## 背景

当前 Pony Agent 已具备本地 Session/Turn Trace 持久化与遥测采集能力，并在 PonySentry 中支持了基本的错误事件（`IngestPayload`）上报。然而，为了支撑高价值 Agent 运行样本的闭环治理、评测数据集萃取与模型行为对齐优化，系统需要将完整的 Agent Trace 异步上报至集中式分析平台（PonySentry）。

面临的核心问题包括：
1. **统一 Trace 数据契约缺失**：本地 `TurnTraceRecord` 偏向本地会话与调试投影，缺乏面向质量评估、标注追踪的标准化上报结构。
2. **生命周期状态流转未定义**：上报后的 Trace 样本缺乏明确的评测流转状态定义（如未经审查、分流优劣、加入评测集、模型优化回流等）。
3. **性能与安全非功能性基线 (NFR) 需硬化**：Trace 上报必须在零感知（非阻塞、有界队列、超时截断）下进行，并严格执行零信任凭据与个人主目录脱敏。

## 候选方案

- **方案 A：基于轻量独立 Trace 模型与异步缓冲队列上报（选定）**
  - **设计**：在 `pony-agent-core::agent::ponysentry` 中定义专用的 `AgentTracePayload`、`TurnTraceItem` 与 `EvalStatus` 状态机。复用 PonySentry 客户端的有界非阻塞异步通道（Capacity 1024, HTTP 超时 <= 3000ms），在上报前强制经过零信任脱敏管道。
  - **优势**：解耦本地 SQLite 持久化格式与远程上报传输协议；传输体积精简；完全保证用户主进程零延迟（Fire-and-Forget）。
  - **劣势**：需要维护本地 Trace 到上报 Payload 的轻量映射层。

- **方案 B：直接将本地完整 SQLite 会话 Snapshot JSON 上报（不选）**
  - **设计**：将 `SessionSnapshot` 完整序列化并通过现有 Ingest 接口上传至 Extra 字段。
  - **落选原因**：Payload 严重膨胀（单会话可达数 MB 到数十 MB），容易击穿网络配额与服务器请求体限制；包含大量本地特有的游标、草稿状态，冗余信息过多，违反最小权限与传输紧凑性原则。

- **方案 C：同步 blocking 上报并在错误时重试（不选）**
  - **设计**：在 Turn 结束时发起同步 HTTP POST 调用上报 Trace。
  - **落选原因**：严重破坏用户交互流畅度，若网络抖动或远端异常将造成本地 UI 冻结达数秒，违反桌面客户端 NFR 可靠性底线。

## 决策

1. **确立 Agent Trace 数据模型契约**：
   - 核心负载定义为 `AgentTracePayload`，包含 `session_id`, `run_id`, `turn_id`, `environment`, `release`, `eval_status`, `turns: Vec<TurnTraceItem>`, 聚合 Token 消耗、耗时及 `reported_at_ms`。
   - Turn 级别明细定义为 `TurnTraceItem`，包含模型、提供方、Token 消耗（含缓存命中 `cache_hit_tokens`）、耗时及结构化工具调用摘要 `tool_calls: Vec<ToolCallTraceItem>`。
2. **定义生命周期状态机 (`EvalStatus`)**：
   - 状态集合严格冻结为：
     - `unreviewed`（初始态：待审查）
     - `triage_good`（正向高质量样本）
     - `triage_bad`（负向缺陷/偏航样本）
     - `eval_dataset`（已入选评测金标集）
     - `optimized`（已完成针对性优化/Prompt 改进/微调）
     - `wontfix`（已知局限/无法处理/暂不解决）
   - 命名采用 snake_case serde 序列化，与前后端及 PonySentry 数据库列完全对齐。
3. **扩展 PonySentry 接口契约**：
   - PonySentry 服务端扩展 `/api/v1/traces` (或 `/api/v1/agent/traces`) 端点，接收 `AgentTracePayload`。
   - 客户端沿用 `X-Client-Token` 认证头，支持 512KB 单包上限防护。
4. **冻结 NFR 基准规范 (`.dev-team/nfr-baseline.json`)**：
   - **外部调用超时**：`external_call_timeout_ms <= 3000`。
   - **队列有界性**：`bounded_queue` 容量严格 `<= 1024`，满载静默丢弃最新/最旧帧，杜绝 OOM。
   - **零信任脱敏**：全链路必须经过 `sanitize` / `sanitize_json`，彻底抹除 Bearer/Basic/API Key 凭据与操作系统 User Home 绝对路径。
   - **日志结构化**：格式强制统一为结构化 JSON 并包含 Trace/Turn 关联标识。

## 影响

- **代码模块**：在 `crates/pony-agent-core/src/agent/ponysentry/models.rs` 与 `mod.rs` 中导出 `AgentTracePayload`、`TurnTraceItem`、`ToolCallTraceItem` 与 `EvalStatus`。
- **治理与测试**：测试专家可针对上述数据模型进行红相/契约断言，验证其在极端字段缺省、反序列化容错与状态机流转下的鲁棒性。