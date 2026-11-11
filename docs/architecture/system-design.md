# System Design: Pony-Agent 技能系统对齐

Status: Implemented (Phase 0 Planning)

## 1. 架构概览
技能系统作为 `crates/pony-agent-core` 的子系统，遵循 Rust 核心逻辑与前端渲染职责分离原则。前端仅作状态展示，技能调度由后端控制。

## 2. 核心交付目标 (Goals) 与非目标 (Non-Goals)
- **Goals**: 架构同构化（收口至 Core）、运行时可观测性（四维标准）、配置化与热刷新。
- **Non-Goals**: 禁止重构业务逻辑、不涉及 UI 重写、暂不涉及分布式治理。

## 3. 调用策略四象限闸门 (The Quadrant Gate)
根据任务的风险与优先级评估路由路径：

| 优先级 \ 风险等级 | **低风险** | **高风险** |
| :--- | :--- | :--- |
| **高优先级** | **Quadrant I (Direct Code)**: 热点路径，硬编码，响应极速。 | **Quadrant II (Workflow)**: 复杂 DAG 工作流，强质量门禁。 |
| **低优先级** | **Quadrant III (Skill)**: 通用可复用技能。 | **Quadrant IV (Agent)**: 模糊意图，委派子 Agent。 |

## 4. 架构权衡结论
- **ISSUE-008 (目录预算裁剪)**: 采用“预检 -> 拦截 -> 报错”模式，Fail-Closed 保证安全性。
- **ISSUE-009 (热刷新)**: 采用“显式触发热刷新”模式，确保运行时状态确定性。

## 5. 稳定性门禁
- **执行域隔离**: 校验 `write_scopes`，静默越权立即阻断。
- **协议契约漂移**: 建立 `check-drift.sh` 机制，强制代码与架构文档元数据对齐。
- **饱和度熔断**: 超出并发上限强制熔断，保护资源。
