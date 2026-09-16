# composer-input-priority Specification

## Purpose

规范 composer 输入优先级隔离：IME 组合期间 Enter 不误触发提交；draft 同步写入立即可读，不被渲染工作延迟；trace 重排工作以低优先级调度，不抢占输入响应。

## Requirements

### Requirement: IME composition SHALL NOT trigger submission

While the user is composing with an IME (e.g. Chinese input), Enter used for candidate confirmation SHALL NOT trigger turn submission.

#### Scenario: Enter during composition ignored

- GIVEN the user is composing with an IME
- WHEN Enter is pressed with `isComposing` true (or legacy `keyCode` 229)
- THEN composition SHALL proceed without dropped events
- AND submission SHALL NOT trigger

### Requirement: Composer draft SHALL update synchronously

The composer draft SHALL be written synchronously on input and readable immediately, regardless of ongoing rendering work.

#### Scenario: Draft during busy render

- GIVEN trace/conversation rendering work is in flight
- WHEN the user types
- THEN the draft SHALL update synchronously (no deferral)
- AND the input event handler SHALL NOT be deferred

### Requirement: Trace rearrangement work SHALL yield to input

Trace timeline rearrangement work SHALL be scheduled at low priority (deferred execution with idle-callback preference and timer fallback), so it does NOT block input handling. Deferral is acceptable; dropping updates is NOT.

#### Scenario: Deferred update eventually consistent

- GIVEN the trace panel is expanded during streaming
- WHEN trace data updates arrive
- THEN the panel SHALL eventually render the latest data
- AND no update SHALL be lost

#### Scenario: Session switch cancels pending work

- WHEN a session switch occurs while a low-priority render is pending
- THEN the pending task SHALL be cancelled
- AND the new session SHALL render its own data

## Implementation Notes

- As-built（与归档 delta 的偏离，显式声明）：
  - 归档 delta 的"Deferred trace rendering … Merged scheduling / Idle scheduling fallback"三 Scenario 描述的通用低优先级调度框架，实际由 084/085/086 的调度工作覆盖（`runLowPriorityTurnWork`：800ms 延迟 + `requestIdleCallback` timeout 2500ms + `setTimeout` 兜底，`src/lib/runtime/utils.ts`；trace 路径 `scheduleThrottledTraceTimeline` 经它执行）。本卡收敛为 **IME 守卫 + 输入路径确认**。
  - IME 守卫位于 `HomeWorkspace.vue` composer keydown（`event.isComposing || event.keyCode === 229`）；覆盖单测 `tests/HomeWorkspace.spec.ts`（IME 组合中 Enter 不提交）。
- Retroactive sync: 本规范由 `openspec/changes/archive/2026-08-14-composer-input-priority-isolation/specs/composer-input-priority/spec.md` 按 as-built 重写同步。
