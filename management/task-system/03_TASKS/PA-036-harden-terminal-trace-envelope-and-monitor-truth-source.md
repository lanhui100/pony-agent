# PA-036 收紧 terminal trace envelope 与 monitor 真相源

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- [add-terminal-trace-envelope-and-monitor-truth-source](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-terminal-trace-envelope-and-monitor-truth-source)

## Delta Spec
- [terminal-trace-envelope-and-monitor-truth-source/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-terminal-trace-envelope-and-monitor-truth-source/specs/terminal-trace-envelope-and-monitor-truth-source/spec.md)

## Canonical Spec
- 交付后沉淀到 `openspec/specs/` 的 terminal trace envelope / monitor truth-source canonical spec

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把 terminal lifecycle event metadata、persisted trace envelope 与 monitor/control-plane 聚合继续收紧成单一真相源，避免 sync / stream 路径、reload 后读面与 failed/cancelled evidence 继续出现“能看到 trace 但指标口径不完全可信”的灰区。

## 输出
- sync / stream terminal trace envelope 的统一持久化口径
- completed / failed / cancelled persisted trace 的 canonical terminal metadata
- monitor / control-plane 对 terminal trace 的统一聚合真相源
- failed / cancelled persisted trace 对既有 evidence 字段的保真验证
- Rust / 前端 / reload 验证矩阵

## 范围边界
- 本卡只补 terminal envelope parity 与 truth-source verification
- 本卡不新增 lifecycle event contract，不扩 hooks evidence model，不重写 monitor 聚合维度
- terminal envelope 的 `eventType` 只复用 [PA-031](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-031-define-turn-lifecycle-and-event-contract.md) 已定义的 canonical terminal lifecycle event 集合

## 验收标准
- sync 与 stream 产出的 terminal persisted trace 都有统一的 `eventId / eventType / eventVersion / sequence / emittedAtMs`
- monitor / control-plane 聚合不再需要区分“stream trace”与“sync trace”去做指标兜底
- failed / cancelled trace reload 后仍能保留 terminal metadata，且不会丢失既有 provider/tool/hook evidence 字段
- 当前端读取到缺失 terminal envelope 的 persisted trace 时，只能展示 non-canonical/raw trace 信息，不得产出 completed / failed / cancelled canonical metrics
- Rust/前端测试覆盖 completed、failed、cancelled、reload 与 monitor drilldown 五类路径

## 当前进展
- 已完成前置契约：
  - [PA-031](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-031-define-turn-lifecycle-and-event-contract.md)
  - [PA-032](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-032-stabilize-trace-persistence-and-recovery-contract.md)
  - [PA-035](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-035-integrate-runtime-hook-dispatch-on-stable-boundaries.md)
- 立项时缺口：
  - streamed graph-run 路径已有 terminal trace annotation 回写，但 sync `run_turn()` persisted trace 仍缺同等粒度 terminal envelope
  - monitor/control-plane 已能聚合 hook trace records，但“terminal envelope 一致性”尚未被独立任务卡接住
  - failed/cancelled drilldown 仍需要确认 persisted trace 在 terminal metadata 收紧后不会丢失既有 provider/tool/hook evidence 字段
  - terminal envelope 缺失时，前端 read-plane 仍需要验证不会把 raw trace 推导成 canonical terminal metrics

- 当前已完成实现：
  - `turn_flow.rs` 已新增 `TurnEventEnvelope` 与 `build_terminal_turn_event_envelope(...)`
  - `runtime.rs` 已为 sync completed / failed terminal path 补齐 terminal envelope 注回 persisted trace 与 `TurnResult`
  - 已补 3 条 sync terminal exact 测试，直接断言 `eventId / eventType / eventVersion / sequence / emittedAtMs`
  - `control_plane.rs` 已把 monitor / drilldown canonical metrics 收紧为只统计带 terminal envelope 的 persisted trace
  - `ModelMonitorPage.vue` 已为缺失 terminal envelope 的 trace 增加 raw-evidence 明示，不再把该类 trace 混入 canonical metrics 语义
  - 已新增后端与前端回归：
    - raw trace 不参与 monitor canonical metrics 聚合
    - raw trace 仍保留在 drilldown 中作为原始证据可见
    - monitor UI 会对 raw trace 展示显式提示
  - `session.rs` 已补 file-backend roundtrip 回归：
    - failed terminal trace reload 后保留 `eventId / eventType / eventVersion / sequence / emittedAtMs`
    - cancelled terminal trace reload 后保留同等 terminal envelope
    - failed / cancelled reload 后不丢已有 `provider_call_records / tool_activities / hook_trace_records`
  - `control_plane.rs` 已补 mixed sync / stream truth-source 组合验证：
    - sync-like 与 stream-like persisted trace 会被同一套 canonical aggregation 口径统计
    - monitor 不再需要依据来源类型分叉 terminal metadata 逻辑
  - `control_plane.rs` 已补 failed / cancelled session drilldown evidence 保真验证：
    - failed trace 下钻会保留 terminal envelope、provider call record、tool activity、blocked hook evidence
    - cancelled trace 下钻会保留 terminal envelope、provider call record、tool activity、non-blocking hook evidence
- 前端 read-plane 已补 terminal envelope 消费回归：
  - `runtime-store` 只会从带 canonical terminal envelope 的 trace 恢复 `failed / cancelled` terminal runtime phase
  - 缺失 terminal envelope 的 raw trace 仍会保留在历史里，但不会被 store 恢复成 canonical terminal state
  - `ModelMonitorPage` 已持续对 raw trace 展示 non-canonical 提示
  - `PA-036` closeout 判断已明确：
    - sync `run_turn()` 运行时语义只产出 `completed / failed`
    - `cancelled` 只存在于 streamed turn + execution control 路径
    - 因此 `2.2` 已按真实能力边界收窄为“sync failed 与 streamed cancelled”，不是遗漏实现

## 下一步动作
- 把已完成的 terminal truth-source 约束继续作为 monitor/read-plane 的默认真相源
- 后续如要新增 sync cancel 语义，应作为新的 runtime/cancellation change 单独立卡，不回灌 `PA-036`
- 如需归档对应 OpenSpec change，按单独归档流程执行

## 当前卡点
- 暂无；本卡已完成关闭

## 断点续跑提示
继续前先看：
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src-tauri/src/agent/session.rs`
- `src/components/ModelMonitorPage.vue`
- `tests/runtime-store.spec.ts`
