# Agent Note: Agent Team 端到端协作体系与全双工任务看板架构

Status: implemented

## Problem

在单智能体模式下，复杂的长链任务无法有效并行分解，易产生上下文爆炸与注意力涣散：

1. **协作内核缺失**：早期仅有单 Agent 轮次流转，缺乏智能体间全双工通知、队友生命周期驱动（Autonomous Teammate Driver）以及跨成员收件箱（Inbox）；
2. **任务缺乏无竞争防踩踏机制**：多智能体同时认领或变更同一任务时，缺乏原子性的版本控制（CAS）机制，导致状态丢失或竞态条件；
3. **协作状态缺乏全景可见性**：前端界面缺乏响应式的团队成员花名册（Roster）与共享任务看板（Task Board），用户无法感知多智能体内部协同拓扑。

## Decision

1. **Rust 全双工协作内核**：
   - 核心层实现自主队员驱动循环、边沿通知总线（基于 Tokio Notify / 广播通道），支持智能体状态变化、消息入队与任务更新的亚毫秒级边沿唤醒；
   - 交付 `spawn_teammate`、`send_message`、`wait_agent`、`interrupt_agent`、`drain_inbox` 等标准工具集；
   - 保持严格职责边界：调度与沙箱执行位于 Rust 核心层，前端仅做状态展示与交互。
2. **CAS 乐观锁共享任务看板**：
   - 共享任务看板支持创建、认领（claim）、完成（complete）、释放（release）与删除流转；
   - 每次任务状态流转均校验 `expected_revision` 版本号，杜绝并发踩踏；
   - 支持写域（write_scopes）声明与依赖项（blocked_by）阻塞控制。
3. **响应式前端全景面板**：
   - 前端通过 Pinia `useTeamStore` 订阅团队事件并同步数据；
   - 提供 `TeamRosterDrawer.vue` 与 `TeamTaskBoard.vue` 组件，实时可视化队员状态与任务看板。
4. **历史溯源与接替**：
   - 本决策源自并完全接替历史决策 `docs/archive/decisions/0041-agent-team-end-to-end-architecture.md`。

## Alternatives considered

- **前端多标签页/多会话轮询模拟协作**：
  在前端开辟多个独立会话，由前端 JS 轮询并在会话间转发消息。
  *落选原因*：严重违背工程宪法“Rust 核心与前端渲染职责分离，调度与沙箱必须位于 Rust 核心”，且前端轮询时延高、可靠性差、无法保证原子性 CAS。
- **引入外部 Redis / 消息队列系统**：
  在本地部署轻量 Redis 或 MQTT 承担消息中枢。
  *落选原因*：引入外部依赖与安装复杂度，破坏桌面应用的即装即用体验；基于 Tokio 原生内存总线性能更优且足够轻量。

## Consequences

- 团队协作能力下沉为内核基础资产，支持多角色（tester、reviewer、executor 等）并发求解；
- 任务看板具备强一致 CAS 保证；
- 桌面端具备完整的团队与任务看板可视化呈现。
