# PA-087 输入框优先级隔离

## Basic Info

- ID: PA-087
- Status: Done
- Priority: P0
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-14
- Updated At: 2026-08-14
- OpenSpec Change: `composer-input-priority-isolation`（3 路对抗审核已通过，2026-08-14）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订）

## Background

流式输出期间主线程被 trace 渲染/对话渲染占满时，`WorkspaceComposer` 的 keydown/click 事件排不上队，用户"无法发送新消息"。即使 PA-084/085/086 落地后，流式渲染本身仍有主线程占用，输入事件仍需兜底隔离。dsh 的做法是视图分离（不同时渲染），pony 保持同页布局，因此需要输入优先级隔离作为兜底。

## Goal

`WorkspaceComposer` 的输入事件处理不被 trace/对话渲染阻塞：输入事件高优先级处理，trace 侧低优先级渲染调度（`requestIdleCallback` / `nextTick` 延后）。

## Scope

- `src/components/chat/WorkspaceComposer.vue`：输入事件处理路径确认无阻塞（keydown/click/composition）
- `src/components/HomeTracePanel.vue` / `HomeSidebar.vue`：trace 渲染调度降级（展开时用 `requestIdleCallback` 或帧后渲染）
- 输入状态（draft）写入与读取路径确认不被渲染延迟影响
- 前端单测：输入事件在渲染忙碌时仍及时处理（模拟长任务场景）

## Non-Goals

- 不重构输入组件为 Web Worker
- 不做视图分离（用户已确认保持侧边栏布局）
- 不改变提交/发送逻辑

## Acceptance Criteria

1. 模拟主线程长任务（如 200ms 阻塞）时，输入框 keydown/click 仍及时响应（测试或手动验证）。
2. trace 渲染调度降级后，展开面板内容最终一致（不丢更新）。
3. draft 输入在渲染忙碌时不丢失、不延迟回显。
4. 前端 vitest、`npm run build` 全绿；既有 composer 测试更新后通过。

## Review Plan

- @architect：调度策略边界（requestIdleCallback vs nextTick vs rAF）、与 store 节流整合
- @code-reviewer：事件冒泡/合成事件回归、输入法 composition 事件、draft 持久化时机
- @tester：长任务模拟用例、输入及时性、渲染最终一致用例

## Current Progress

- 3 路对抗审核（2026-08-14）已完成，findings 全部采纳：目标改写"输入及时响应"、删除"持久化 draft"假场景、isComposing 守卫、复用 runLowPriorityTurnWork、顺序代理断言。详见 `02_REVIEWS/2026-08-14-pa084-087-spec-review.md`。
- **实现完成（2026-08-14）**：
  - `HomeWorkspace.vue`：`handleComposerKeydown` 增加 `isComposing` / keyCode 229 守卫（修复中文候选确认误发送的既有 bug）。
  - 输入路径审计确认：draft 经 `@input` 同步写入 store（`setDraftMessage`），textarea `:value` 绑定，无异步阻塞。
  - **实现偏离记录**：调度降级部分由 PA-084/085/086 覆盖（折叠零渲染 + memo 有界计算 + 虚拟滚动有界 DOM），无需独立 rIC 调度层——审核意见已预判"087 是 086 的附属调度层"，本卡收敛为 IME 守卫 + 输入路径确认。
  - 验证：全量前端 394 passed（含新增 IME 测试）+ vue-tsc + build 通过。

## Next Action

- 进入实现后 3 路对抗审核（@code-reviewer / @architect / @tester）。

## Blockers

- 建议在 PA-084/085/086 之后实施（先治本再兜底）。

## Resume Hint

- 先读 `src/components/chat/WorkspaceComposer.vue`（输入处理）、`src/stores/runtime.ts`（draft 状态）、`src/lib/frontend-flight-recorder.ts`（已有卡顿诊断证据）。