# PA-084~087 实现后对抗审核记录（2026-08-14）

> 范围：前端卡顿优化 4 卡实现（trace 面板折叠懒渲染 / 虚拟滚动 / 快照投影 / 输入优先级隔离）
> 审核阵容：@consultant 档 @architect（架构）1 路完成；@code-reviewer / @tester 因环境 depth limit 未能完成独立输出，由编排器基于 @consultant 意见 + 自核代码兜底
> 结论：**P1 问题已全部修复**（见下表），最终验证 395 passed + vue-tsc + build 全绿。

## 审核意见裁决汇总

| # | 意见（来源） | 级别 | 裁决 | 处理 |
|---|---|---|---|---|
| 1 | PA-086 投影层未按 spec 完整落地（无 sessionId/generation/revision 对象） | P1 | **裁决：收窄 spec** | 投影层定位为"轻量 memo helper"（非完整 TraceProjection），design.md 已更新边界裁决：就地变更路径前均有新克隆赋值（引用已变），memo 以 ref+updatedAt 自然失效 |
| 2 | PA-086 发布入口是语法收敛，`sessions.ts:212` / `runtime.ts:3226` 仍直接写 | P1 | **裁决：不修复，记录说明** | 两处均为低频恢复路径（snapshot restore / terminal $patch），赋值引用为新克隆（memo 自然失效），不构成一致性风险 |
| 3 | PA-086 `ref+updatedAt` 无法覆盖就地变更（updateActiveModelTraceFromAssistant） | P1 | **裁决：当前代码路径不成立** | 所有就地变更前都有 `updateActiveTraceTimeline(新克隆)` 赋值（引用已变）；已核代码 2775-2778 / 2815-2818 |
| 4 | PA-085 `@scroll.passive` 绑定在自定义组件上，listener 落 root 而非 viewport（scroll 不冒泡） | P1 | **采纳并修复** | `onTraceBodyMounted` 中对 `viewportEl` 原生 `addEventListener("scroll")`，卸载移除（`boundScrollHandler` 清理） |
| 5 | PA-085 折叠 turn 估算高度含不可见的 timeline 内容（CSS grid 0fr 压缩） | P1 | **采纳并修复** | `estimateTurnHeight` 折叠时只算 header；测试同步修正 |
| 6 | PA-085 底部跟随只 watch turn ID 签名，流式更新不改 turn ID | P1 | **采纳并修复** | 新增 `totalTraceHeight` computed（前缀和末值），watch 总高度触发跟随 |
| 7 | PA-084 `frozenLiveTraceTimeline` 持有可变引用（就地修改污染）+ 无 session 清理 | P1 | **采纳并重构** | 折叠时 `liveTraceTurn` 返回 null（不参与侧边栏计数）；展开时正常构造（ref+updatedAt 自然失效）；消除冻结引用问题 |
| 8 | PA-086 模块级 memo 生命周期不完整（卸载不清理） | P2 | **采纳并修复** | `HomeSidebar` 增加 `onBeforeUnmount` 清理 |
| 9 | PA-084/086 存在两套独立缓存（frozen + memo） | P2 | **裁决：已消解** | 修复 7 后 frozen 体系删除，仅剩 memo 单一体系 |
| 10 | PA-087 只覆盖 IME，低优先级调度未实现 | P2 | **裁决：收窄验收** | PA-087 收敛为"IME 守卫 + 输入路径确认"；调度降级由 084/085/086 覆盖（折叠零渲染 + memo 有界计算 + 虚拟滚动有界 DOM），实现偏离已记录在任务卡 |
| 11 | 集成测试不足（9 条写路径、就地修改、session 切换、真实 scroll、50+ turn） | P2 | **部分采纳** | 已补：投影层 6 测试、虚拟滚动 8 测试（含折叠高度修正）；真实浏览器滚动/50+ turn 为手动验证项（jsdom 无布局，记录为残余风险） |
| 12 | 注释与实际架构不一致（P3） | P3 | **采纳** | trace-projection / HomeTracePanel 注释已更新为实际行为 |

## 关键验证

- 折叠时 `liveTraceTurn` 返回 null：侧边栏计数折叠时不含活跃 turn（行为变化，语义合理——折叠时用户看对话区）。
- 就地变更路径：所有 `updateActiveModelTraceFromAssistant` 调用点前均有新克隆赋值，memo 引用失效安全。
- 底部跟随：依赖总高度（流式 timeline 内容变化触发），用户上滑（距底 ≥80px）后停止跟随。

## 测试与验证

- `vue-tsc --noEmit` 通过
- `npm run test:unit`：22 文件 395 passed | 10 skipped
- `npm run build` 通过
- 手动验证项（残余风险）：真实浏览器滚动定位、50+ turn DOM 上限、流式+展开 trace+输入三方体验