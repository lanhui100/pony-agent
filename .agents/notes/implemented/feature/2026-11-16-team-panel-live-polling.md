# Agent Note: 团队面板按引用计数轮询刷新

Status: implemented

## Problem

桌面端「队友花名册」与「任务看板」两个面板（`src/components/team/TeamRosterPanel.vue` /
`TeamTaskBoardPanel.vue`）的数据是**拉取式**的：后端只有两个只读 IPC
（`team_get_roster` / `team_get_tasks`，`src-tauri/src/lib.rs`），没有事件流。面板此前只在
挂载时 `fetchTeamOverview()` 一次（`TeamRosterPanel.vue` 的挂载处还留着「B098/B105 UI 状态恢复
未实现」的注释），于是打开面板那一瞬间之后数据静止：队友状态变化、任务被认领/完成都不会反映，
用户必须关掉面板重开才能看到真实进度。这与 `.agents/notes/implemented/architecture/2026-11-11-agent-team-collaboration-and-task-board.md`
里「看板是运行时真实状态的可见面」的定位冲突——看得见的只是挂载那一刻的快照。

## Decision

1. 轮询状态收敛到 store（`src/stores/team.ts`）：新增 `startPolling()` / `stopPolling()`，
   间隔常量为模块级 `TEAM_POLL_INTERVAL_MS = 2_000`。
2. 定时器**模块级单例 + 引用计数**：`teamPollTimer` 只在引用计数 0→1 时创建，归零时 `clearInterval`；
   两个面板同时挂载也只有一份 timer、一份 IPC 频率。
3. 两个面板以 `onMounted` / `onUnmounted` 成对调用（挂载即 `fetchTeamOverview()` + `startPolling()`，
   卸载 `stopPolling()`）；组件不自持任何 timer 变量。
4. 轮询只是**重复调用既有只读 IPC**：不新增后端命令、不新增事件通道、不改持久化格式；
   非 Tauri 环境由 `fetchTeamOverview()` 内的 `isTauriAvailable()` 早退保持静默。

## Alternatives considered

- **后端事件推送（Tauri `emit` + 前端 `listen`）**：语义最正（变更即推送、空转零成本），但要后端在
  `team_tasks` / 队友状态写入点检测变更并新增事件名契约，跨 Rust↔TS 的契约面与本次诉求（面板不再是
  死数据）相比收益不足；留作后续若轮询出现可测的 IO 压力时的替换路径。
- **各组件内自持 `setInterval`**：实现最短，但两面板同时可见时会产生两份定时器与双倍 IPC，
  且卸载遗漏即 silent leak（面板是覆盖层，反复开关会累积）；与「单一刷新源」的低耦合目标相反。
- **手动刷新按钮 / 仅在面板重新打开时拉取**：不新增任何后台活动，但用户诉求正是「拖任务时能实时
  看到队友在干什么」，手动刷新把实时性交还给用户，等于不回应该诉求。

## Consequences

- 面板挂载期每 2s 两次本地只读 IPC（`team_get_roster` / `team_get_tasks`），无网络出口、
  无 LLM 调用；面板全部卸载即完全停止（引用计数归零）。
- 已知取舍：2s 是「跟手」与 IPC 频率的折中，写在单一常量处；要调只改一处。
- 替换路径明确：后端若提供变更事件流，删除 `startPolling` / `stopPolling` 与两处 `onMounted` 调用，
  改为订阅即可，store 对外只暴露 `fetchTeamOverview()`。
