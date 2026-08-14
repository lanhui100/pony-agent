# Tasks

- [ ] 新增 `src/lib/runtime/trace-projection.ts`：`ProjectedTurn` / `TraceProjection` / 签名化 memo / 单一发布入口（非响应式模块层）。
- [ ] `runtime.ts`：9 条 `traceTimeline` 写路径收敛到发布入口（diff 逐条确认）：throttled update、terminal 事件、恢复、rollback、checkpoint load、会话切换、过滤。
- [ ] `runtime.ts`：throttle 回调携带 `{sessionId, turnId, generation}` 校验，不跨代。
- [ ] `HomeSidebar.vue`：状态计数链改读投影层。
- [ ] `HomeTracePanel.vue`：turns 渲染读投影行；删除 turnTimelineCache 全量计算，改签名化 memo。
- [ ] `HomeWorkspace.vue` / `WorkspaceTurnItem.vue`：工具归因读投影行（保留 toolActivities/capabilityInvocation）。
- [ ] 前端单测：9 条写路径一致性矩阵、引用相等断言（未受影响 turn）、删除立即逐出、会话切换原子清空、throttle 不跨 generation、消费投影后行为不变。
- [ ] 节流参数实测记录（200ms 基线评估，调整需证据）。
- [ ] 更新受波及的既有测试（HomeWorkspace.spec.ts trace 顺序断言等）。
- [ ] `npm run test:unit`、`npm run build` 通过。

## Validation Notes

- 3 路对抗审核（2026-08-14）已采纳：P0 9 条写路径收敛单一入口；P1 投影 DTO 字段列全（title/phase/providerCallRecords/buildContextObservation/toolActivities/fallback/token/duration）；P1 revision≠增量渲染 → 签名化 memo + 不可变逐 turn 引用；P3 投影放非响应式模块层；P1 引用语义（活跃别名/历史派生）；P2 移除重复 activeTimeline，快照含 sessionId/generation/orderedTurnIds/byTurnId/revision。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。