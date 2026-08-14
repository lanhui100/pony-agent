# composer-input-priority-isolation

## Background

流式输出期间主线程被 trace 渲染/对话渲染占满时，`WorkspaceComposer` 的 keydown/click 事件排不上队，输入出现卡顿感。即使 PA-084/085/086 落地后，流式渲染本身仍有主线程占用，输入体验仍需兜底隔离。

参考：dsh 用视图分离（不同时渲染）解决；pony 保持同页布局，因此需要输入优先级隔离作为兜底。

3 路对抗审核（2026-08-14）确认的关键约束：

- **目标改写**：是"流式期间**输入事件及时响应**（打字/点击不卡、draft 立即更新）"，**不是**"流式期间发送第二条消息"（isSubmitting 时 Enter 明确不提交，属既有产品语义）。
- **"持久化 draft"场景不存在**（persistHistory 载荷无 draft，切换即清空）——删除该验收。
- **IME 修复**：Enter 无 `isComposing` 守卫，中文候选确认会误触发提交——本卡修复（属"输入事件正确性"而非"改变提交逻辑"）。
- **复用 `runLowPriorityTurnWork`**（utils.ts:137 已有 rIC+timeout+fallback），不新建调度工具。
- **模板只读 `presentedProjection`**（不订阅最新源快照），idle 调度才能真正延后渲染。

## Goals

- `WorkspaceComposer` 的输入事件处理不被 trace/对话渲染阻塞（打字/点击及时响应，draft 立即更新）。
- trace 渲染调度降级（展开时低优先级调度），不阻塞输入。
- IME 输入（中文）不受影响，候选确认不误触发送。

## Non-goals

- 不重构输入组件为 Web Worker。
- 不做视图分离（用户已确认保持侧边栏布局）。
- 不改变发送触发条件以外的语义（isSubmitting 提交门禁保持现状）。
- 不支持流式期间排队发送第二条消息（需另开队列语义 change）。

## Scope

- `src/components/chat/WorkspaceComposer.vue` / `HomeWorkspace.vue`：输入事件路径审计 + `isComposing` 守卫。
- `src/components/HomeTracePanel.vue`：展开态 trace 更新渲染用 `runLowPriorityTurnWork` 调度（合并高频更新）。
- `src/stores/runtime.ts`：draft 写入保持同步；`presentedProjection` 消费路径（依赖 PA-086 投影层）。
- 前端单测：顺序代理断言（draft 同步 + 渲染延后 + 最终一致）、IME 序列、调度合并、卸载清理。

## Risks

- `requestIdleCallback` 兼容性（jsdom 无此 API）：`runLowPriorityTurnWork` 已有 setTimeout fallback，测试中 mock 或走 fallback 分支。
- 调度降级导致展开面板更新延迟：最终一致性保证（不丢更新，只是延迟）。
- IME composition 事件（中文输入）不能受影响：`isComposing` 守卫 + composition 期间不做任何延迟处理。
- 调度 pending 中会话切换：同步清空并取消 pending 任务。

## Validation

- 前端 vitest：顺序代理断言（draft 同步更新 + 渲染延后执行 + 最终一致无丢更新）、IME compositionstart→input→Enter→compositionend 全序列、rIC fallback、调度合并（多 revision 一次渲染）、卸载清理。
- `npm run build` 通过。
- 手动验证：流式期间输入无卡顿；展开面板内容最终一致；中文输入候选确认不误发送。