# trace-render-snapshot Specification

## Purpose

规范 trace 渲染快照投影：共享投影管线 `useTraceProjection` 以引用签名 memo 化 turn 投影，流式更新只重算受影响 turn；冻结守卫（`liveTurnEnabled`）切断折叠态与 store 可变数组的引用共享，避免陈旧 memo 命中。

## Requirements

### Requirement: Trace consumers SHALL share a signature-memoized projection pipeline

Trace consumers (sidebar status chain, trace panel / TraceInspector, workspace tool attribution) SHALL consume turn projections through the shared pipeline with signature-based memoization (reference + `updatedAt` signature), so streaming updates do NOT trigger wholesale recomputation.

#### Scenario: Bounded compute during streaming

- GIVEN streaming updates arrive
- WHEN a trace consumer renders
- THEN unchanged turn projections SHALL be served from cache (reference equality)
- AND only the affected turn SHALL be recomputed

#### Scenario: Memo invalidation correctness

- WHEN a turn's timeline changes
- THEN its memo entry SHALL invalidate (signature change)
- AND other turns' memo entries SHALL remain valid

### Requirement: Frozen state SHALL cut reference sharing via live-turn guard

When the trace surface is collapsed (or otherwise declares frozen state), the projection pipeline SHALL exclude the live turn (`liveTraceTurn` returns null), cutting reference sharing with the store's mutable timeline array so the memo cache does NOT return stale results polluted by in-place mutation.

#### Scenario: Guard closed excludes live turn

- GIVEN the trace surface is collapsed
- WHEN streaming mutates the store timeline in place
- THEN the projection SHALL NOT merge the live turn
- AND frozen consumers SHALL keep seeing the last settled snapshot

#### Scenario: Guard open resumes live merge

- GIVEN the trace surface is expanded
- WHEN streaming updates arrive
- THEN the live turn SHALL merge normally
- AND the memo SHALL invalidate naturally on reference change

### Requirement: Consumer parity through projection

Trace consumers SHALL render identical data through the projection as they did from raw store data.

#### Scenario: Sidebar status chain

- WHEN the sidebar computes status counts
- THEN the counts SHALL match the projection's derived data

#### Scenario: Workspace tool attribution

- WHEN the workspace renders tool attribution from trace
- THEN the attribution SHALL match the projection's tool activity data

## Implementation Notes

- As-built（与归档 delta 的偏离，显式声明）：
  - 归档 delta 写"All write paths SHALL converge on a single publication entry"；实现后审核已裁决降级为**轻量 memo helper**（ref + `updatedAt` 签名），低频恢复路径不强制收敛（引用变化时 memo 自然失效）。"单一发布入口全收敛"不是事实，未同步。
  - 共享管线为 `src/lib/runtime/useTraceProjection.ts`（PA-096 从 HomeSidebar 抽出）；`liveTurnEnabled` 是 PA-086 冻结守卫的参数化（HomeSidebar 恒 false，TraceInspector 以 `open` 为开关）。
  - 节流 interval 调优的证据记录要求（归档 delta "Throttle parameter evidence"）是过程约束而非长期行为，未同步。
- Retroactive sync: 本规范由 `openspec/changes/archive/2026-08-14-trace-render-snapshot-projection/specs/trace-render-snapshot/spec.md` 按 as-built 重写同步。
