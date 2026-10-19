# 0041 Agent Team 端到端协作体系与全双工任务板架构

Status: implemented

## 背景
Pony Agent 目前仅具备单 Agent 轮次流转能力，虽在 `orchestration.rs` 中预留了 `spawn_teammate`、`team_task_*` 等内存数据模型与空桩，但缺少 `wait_agent` 边沿唤醒机制、真正的自主队员执行循环（Autonomous Teammate Driver）、Tauri IPC 状态查询控制通道以及前端可视化 Team Roster 与 Task Board 面板。模型侧工具面此前也因未完整闭环而在 W25 中被摘除。

## 候选方案
1. **方案 A：纯前端多会话模拟方案**
   - *说明*：前端同时创建多个独立 Session，通过前端轮询模拟 Agent 之间的消息传递与协作。
   - *落选原因*：违背 Pony Agent 架构核心宪章“Rust 核心与前端渲染职责分离，调度与沙箱必须位于 Rust 核心”；前端轮询性能差且无法保证并发原子性与 CAS 强一致。
2. **方案 B：基于 DSH 规范的 Rust 全双工协作内核 + Tauri 事件总线 + 前端响应式看板（当前方案）**
   - *说明*：在 `pony-agent-core` 实现具备边沿通知（Tokio Notify / Condvar）的 `wait_agent`、`team_task_get`、完整的 CAS 任务板状态机与队员收件箱调度器；在 `src-tauri` 导出团队查询与控制的 IPC 命令；前端通过 Pinia `useTeamStore` 统一订阅与展示队员状态（Roster）与任务看板（Task Board）。
   - *入选原因*：完全对齐 DSH 与 Claude Code 架构规范，保持 Rust 强类型与无竞争并发安全，前端体验流畅。

## 决策
1. **Rust 后端架构**：
   - 补齐 `wait_agent`、`team_task_get` 及其配套参数模型。
   - 在 `orchestration.rs` 引入 `TeamActivityBus`（基于 Tokio Notify / 广播通道），在 member 状态变化、message 入队或 task 更新时触发边沿唤醒。
   - 补全 `team_task_update` 的 `set_dependencies`、`edit` 等动作。
   - 在 `src-tauri` 注册 `team_get_roster`、`team_get_tasks`、`team_claim_task` 等 IPC 命令。
2. **前端架构**：
   - 新增 `src/types/team.ts` 契约类型。
   - 新增 `src/stores/team.ts` Pinia Store，管理 `members` 与 `tasks`，提供轮询与事件同步。
   - 在前端界面（如状态栏或工作区抽屉）接入 `TeamRosterDrawer.vue` 与 `TeamTaskBoard.vue`。
3. **工具与安全门禁**：
   - 恢复工具层 `spawn_teammate`、`list_agents`、`send_message`、`wait_agent`、`team_task_*` 的完整集成测试与能力契约。

## 影响
- `crates/pony-agent-core` 形成具备边沿通知能力的 Agent Team 调度总线。
- 前端新增团队全景协同视图。
- 需同步通过原生 Rust 与前端全部门禁测试。
