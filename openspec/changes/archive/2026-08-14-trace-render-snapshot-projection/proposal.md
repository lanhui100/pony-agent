# trace-render-snapshot-projection

## Background

store 已有 `scheduleThrottledTraceTimeline`（200ms 节流合并 timeline 更新），但 `HomeTracePanel` 的 computed（`turnTimelineCache`、`orderedTurnTraces` 等）在每次 `traceTimeline`/`turnTraceHistory` 变化时**全量重算**，且 `HomeSidebar`（状态计数链）、`HomeWorkspace`/`WorkspaceTurnItem` 也消费同一份 trace 数据做工具消息归因。

参考：dsh 的轨迹视图消费**预计算快照**（`trajectory-snapshot-builder.ts`），渲染输入稳定，不是每次事件全量重算。本卡把 trace 渲染输入改为"投影层 + 增量更新 + 签名化 memo"。

3 路对抗审核（2026-08-14）确认的关键约束：

- **9 条写 `traceTimeline` 路径必须收敛**（runtime.ts:401/530/793/901/1143/1274/2033/2049/2093+），否则快照必然失同步。
- **投影放非响应式模块层**（不进 Pinia state，避免 devtools 序列化与持久化泄漏）。
- **引用语义**：活跃 turn 别名同数组（`updateActiveModelTraceFromAssistant` 就地变更可见），历史 turn 派生轻量投影。
- **revision watcher ≠ 增量渲染**：组件用签名化 memo（引用级缓存），不依赖 revision 做渲染门控。

## Goals

- 流式期间 trace 相关计算不再全量重算（签名化 memo + 投影层）。
- 9 条 trace 写路径收敛为单一发布入口，快照与源数据强一致。
- trace 面板与对话区工具归因展示与现状一致。
- 节流参数调整有实测依据。

## Non-goals

- 不做虚拟滚动（PA-085）。
- 不改后端 trace 数据源。
- 不引入新的状态管理库。
- 不做输入框优先级隔离（PA-087）。

## Scope

- `src/lib/runtime/`：非响应式 trace 投影层（`ProjectedTurn` 派生、签名化 memo、单一发布入口）。
- `src/stores/runtime.ts`：9 条写路径收敛到发布入口；`turnTraceHistory` 增删重建。
- `src/components/HomeSidebar.vue` / `HomeTracePanel.vue` / `HomeWorkspace.vue` / `WorkspaceTurnItem.vue`：消费投影层，减少全量重算。
- 节流参数评估（200ms 基线，调整需实测证据）。

## Risks

- 快照失效/重建时机错误导致展示不一致：发布入口单一化 + 一致性矩阵测试兜底。
- 快照内存占用：投影只存渲染所需字段（历史 turn 轻量派生），活跃 turn 别名源数组。
- 与 PA-085 虚拟滚动叠加：085 消费投影层行数据（数据结构对齐）。
- 既有测试波及（HomeWorkspace.spec.ts ~13 个 trace 顺序断言）：消费端改读投影后更新受影响测试。

## Validation

- 前端 vitest：9 条写路径一致性矩阵、增量引用相等断言、会话切换原子清空、组件消费投影后行为不变。
- `npm run build` 通过。
- 手动验证：流式期间 trace 面板与对话区工具归因展示一致；节流参数调整有实测记录。