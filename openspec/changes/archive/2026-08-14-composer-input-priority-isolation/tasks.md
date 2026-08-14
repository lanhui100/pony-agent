# Tasks

- [ ] 审计 `WorkspaceComposer.vue` / `HomeWorkspace.vue` 输入事件路径（keydown/click/composition），确认全部同步无阻塞；发现异步延迟则改同步。
- [ ] `HomeWorkspace.vue`：`handleComposerKeydown` 补 `isComposing` / keyCode 229 守卫（修复中文候选确认误发送）。
- [ ] `HomeTracePanel.vue`：展开态 trace 更新渲染用 `runLowPriorityTurnWork` 调度（pending 标志合并 latest-wins）；模板只读 `presentedProjection`（依赖 PA-086 投影层）。
- [ ] 会话切换/组件卸载：清空 pending + 取消未执行任务。
- [ ] 确认 draft 写入/持久化路径无渲染延迟影响。
- [ ] 前端单测：顺序代理断言（draft 同步 + 渲染延后 + 最终一致）、IME 全序列（compositionstart→input→Enter→compositionend）、rIC fallback、调度合并、卸载清理。
- [ ] `npm run test:unit`、`npm run build` 通过；既有 composer 测试更新后全绿。

## Validation Notes

- 3 路对抗审核（2026-08-14）已采纳：P0 "流式期间发送新消息"不成立 → 目标改写为"输入事件及时响应"；P0 "持久化 draft"场景不存在 → 删除；P1 Non-goal 与 IME 修复冲突 → 加 isComposing 守卫 + Non-goal 改写；P1 idle 不能延后已有渲染 → 模板只读 presentedProjection；P3 重复造轮子 → 复用 runLowPriorityTurnWork；P2 fake timer 不能证明输入响应 → 顺序代理断言；P2 调度 pending 会话切换竞态 → 清空取消；P2 rIC 饿死 → timeout 兜底。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。