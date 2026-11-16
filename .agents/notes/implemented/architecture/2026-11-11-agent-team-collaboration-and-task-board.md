# Agent Note: Agent Team 端到端协作体系与全双工任务看板架构

Status: implemented

## Problem

在单智能体模式下，复杂的长链任务无法有效并行分解，易产生上下文爆炸与注意力涣散：

1. **协作内核缺失**：早期仅有单 Agent 轮次流转，缺乏智能体间全双工通知、队友生命周期驱动（Autonomous Teammate Driver）以及跨成员收件箱（Inbox）；
2. **任务缺乏无竞争防踩踏机制**：多智能体同时认领或变更同一任务时，缺乏原子性的版本控制（CAS）机制，导致状态丢失或竞态条件；
3. **协作状态缺乏全景可见性**：前端界面缺乏响应式的团队成员花名册（Roster）与共享任务看板（Task Board），用户无法感知多智能体内部协同拓扑。

## Decision

1. **Rust 全双工协作内核**：
   - 核心层实现自主队员驱动循环与进程内**活动边沿**（`TeamActivity`：`Mutex<u64>` 版本号 + `Condvar`，任一边沿 bump 版本并 `notify_all`），边沿覆盖成员登记/状态变化、消息入队/出队/回填与任务看板流转；
   - `wait_agent` 是**真等待**：记录基线版本后阻塞，边沿返回 `timed_out=false`，超时返回 `timed_out=true`；无活跃同伴（非 lead 且 `Running`/`Provisioning`）快速失败 `no-active-peer`。对外超时缺省 30s、下界 10s、上界 1h（对齐 DSH `waitForChange` 有界区间）；
   - 交付 `spawn_teammate`、`send_message`、`wait_agent`、`interrupt_agent`、`drain_inbox` 等标准工具集；单条团队消息上限 64 KiB；
   - **派生约束**：只有 Team Lead 可 `spawn_teammate`（工具层从 `ToolExecutionContext.session_id` 反查成员身份，未登记会话按 Lead 代理，与 `drain_inbox` 同源）；同名成员拒绝再派生（绝不静默覆盖），非 lead 成员数上限 16（防递归派生拖死进程）；
   - 保持严格职责边界：调度与沙箱执行位于 Rust 核心层，前端仅做状态展示与交互。
2. **CAS 乐观锁共享任务看板**：
   - 共享任务看板支持创建、认领（claim）、完成（complete）、释放（release）、编辑（edit）、依赖设置（set_dependencies）与删除流转；
   - 每次任务状态流转均校验 `expected_revision` 版本号，杜绝并发踩踏；`deleted` 为终态，不得被 claim/complete 复活；删除仍有依赖者的任务必须拒绝；
   - **授权**：任务当前 owner 或 Team Lead 才能变更；队友只能为自己认领（身份来源同 Decision 1）；
   - **状态机与就绪**：claim 仅限 `pending` 且依赖全部 `completed`；complete/release 仅限 `in_progress`；
   - **约束校验**：subject 非空且 ≤200 字符、description ≤16384 字符、每团队活跃任务 ≤256；写域拒绝绝对路径与 `..` 并去重保序；依赖拒绝自引用/重复/缺失并做环检测（缺失依赖视为未就绪，绝不默认放行）。
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
- **落地对齐（2026-11-15，对标审计 `docs/analysis/agent-team-audit-20261115.md` / DSH `de5d733a18`）**：
  - P0-1「`wait_agent` 空实现返回假信号」已收口：改为真实边沿等待，并抽出可测原语（`wait_on_team_activity` 用局部实例，单测不碰进程级全局状态）；
  - P0-3「伪授权」已收口：补齐调用者授权、状态机、依赖图校验与容量预算，并把工具层 `team_task_create`/`team_task_update` 接到 `TeamCaller`（此前一路按 Lead 放行，任何队友可改任意任务）；
  - 事实订正：边沿机制是 `Condvar` + 版本号（进程内同步原语），**不是** Tokio `Notify`/广播通道；`wait_agent` 在同步工具线程上阻塞等待，符合当前工具面同步执行模型；
  - P0-2「队友跨轮失忆」已收口：`register_teammate` 预分配并登记固定会话 id（`team-<name>-<8hex>`），驱动改用 `run_turn_request` 复用该 id（宿主 `HostTurnEngine` 已注入共享会话存储）；同一登记同时修复「队友 turn 内身份被误判为 Lead」的授权漏斗；
  - P0-5「任何成员可派生/中断」已收口：内核 `spawn_teammate`/`interrupt_agent_as` 强制 Lead-only，工具层经 `TeamCaller::from_session` 传真实调用者；
  - 工具面：`wait_agent` 保持内核真等待，模型可见入口合并为 `list_agents(wait_ms)`（避免新增工具名导致 8 处工具面清单与 schema hash 漂移）；
  - `team_history.rs` 空桩已删除（生产零调用点，仅自引用测试）；恢复条件：需要跨会话团队历史投影时按事件溯源重建，不得恢复 `todo!()` 桩；
  - 未收口（后续任务）：团队状态仍为进程级全局 `static Mutex`（无会话隔离、无持久化、重启即失）；前端团队面板仍是一次性拉取（无订阅/轮询）；`write_scopes` 无重叠告警（仅记录）。
