# Tasks: Add Checkpoint Message Controls And Bottom Menu

## 1. Spec Artifacts

- [x] 1.1 完成 `add-checkpoint-message-controls-and-bottom-menu` 的 proposal / design / spec 文档
- [x] 1.2 基于 `opencode / claude-opus-4-7` 完成一轮独立 spec 审核并回写修订

## 1.5. Runtime State Schema Extension

- [x] 1.5.1 明确消息级 checkpoint UI 所需的最小 runtime state schema
- [x] 1.5.2 为 `HistoryNode` 或等价 runtime read-plane 补齐稳定 `turnId` 映射字段
- [x] 1.5.3 为 checkpoint 能力判断补齐 `workspaceRollbackCapable` 或等价只读字段
- [x] 1.5.4 验证扩展后的 runtime state schema 不破坏现有 history-control 逻辑与测试

## 2. Runtime Read Model

- [x] 2.1 基于扩展后的 runtime state schema，实现稳定的 `turnId <-> nodeId` 映射合同
- [x] 2.2 在 runtime store 中基于既有 `HistoryNode[] / HistoryBranch[] / turnId mapping` 归一化 conversation checkpoint entry，统一供消息区、底部 picker、fork 弹层消费
- [x] 2.3 明确缺失映射时的降级语义：隐藏入口而不是猜测展示

## 3. Workspace UI

- [x] 3.1 在 `HomeWorkspace.vue` 为非最新 agent 消息增加两个纯图标 checkpoint 回退动作
- [x] 3.2 为存在 fork 的 checkpoint 增加 fork 摘要图标与弹层
- [x] 3.3 在 composer 底部 bar 增加 checkpoint picker trigger，并移除当前笨重主入口设计
- [x] 3.4 审计现有全局快捷键占用，确定 checkpoint picker 的最终快捷键
- [x] 3.5 增加打开 checkpoint picker 的快捷键，并在实现与测试中固定记录
- [x] 3.6 明确 sidebar 中保留、弱化与移除的 checkpoint 相关入口，避免双主入口并存

## 4. Verification

- [x] 4.1 为 `HomeWorkspace.spec.ts` 补消息级 checkpoint affordance / picker / shortcut 回归
- [x] 4.2 为 `runtime-store.spec.ts` 补 checkpoint read model 与映射降级场景
- [x] 4.3 补 fork 摘要列表与跳转派发测试
- [x] 4.4 验证 truth-source non-regression，确保 UI 仅消费既有 history-control 边界
- [x] 4.5 验证消息级 affordance 只调用既有 `checkoutHistoryNode / switchHistoryBranch` 等 action，不引入私有仲裁路径
