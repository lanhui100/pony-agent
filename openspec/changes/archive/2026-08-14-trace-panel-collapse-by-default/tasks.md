# Tasks

- [ ] `HomeSidebar.vue`：`activePanel` 默认值 `"trace"` → `""`。
- [ ] `HomeTracePanel.vue`：`collapsible-body` 改为 `v-if="open"` 懒挂载（toggle header 常驻）。
- [ ] `HomeSidebar.vue`：`liveTraceTurn` 折叠态不 stamp `updatedAt: Date.now()`（消除缓存必 miss）。
- [ ] 前端单测：默认折叠（body 未挂载）、toggle 后挂载渲染、折叠时 trace 更新无 body DOM、展开后渲染最新数据、liveTraceTurn 折叠态不 stamp。
- [ ] 更新既有 `HomeSidebar.spec.ts` 中 trace 相关用例（先展开再断言）。
- [ ] `npm run test:unit`、`npm run build` 通过。

## Validation Notes

- 3 路对抗审核（2026-08-14）已采纳：P0-1 v-if 卸载删展开入口 → header 常驻 + body 懒挂载；P0-2 "无 trace 计算"不可测 → 验收改为"body 未挂载"，父组件计数链归 PA-086；P3 措辞统一；顺手修复 liveTraceTurn stamp（HomeSidebar.vue:123）。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。