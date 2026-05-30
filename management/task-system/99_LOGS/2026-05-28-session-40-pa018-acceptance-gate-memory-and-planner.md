# 2026-05-28 Session 40 PA-018 Acceptance Gate Memory And Planner

## 本轮目标

- 继续推进 `PA-018`
- 再补一条显式、保守、可审计的 `LongTermMemory` 稳定事实来源
- 让这条新事实进入 graph / planner 的结构化消费链
- 同步回写任务文档与验证证据

## 本轮改动

- 更新：
  - `src-tauri/src/agent/session.rs`
  - `src-tauri/src/agent/graph.rs`
  - `src-tauri/src/agent/planner.rs`
  - `src/types/runtime.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `LongTermMemory` 新增第七条保守写入来源：
  - `project_workflow.acceptance_gate`
- 新规则只在用户消息里出现明确交付门槛表达时落地，例如：
  - `建立验收标准`
  - `验收审计`
  - `acceptance criteria`
  - `closeout audit`
- 新事实沿用单槽 identity：
  - 同类记录按 `kind` 去重更新
  - 不会因为重复表达而无限追加
- `GraphTurnHandoff` 新增结构化字段：
  - `acceptance_focus`
- `GraphEngine.build_turn_handoff()` 现在会从 retrieval `LongTermMemory.entries` 中提取：
  - `project_workflow.acceptance_gate -> acceptance_focus`
- `DefaultGraphPlanner` 的 continue 摘要现在会显示：
  - `验收要求：...`
- 前端共享类型 `src/types/runtime.ts` 已同步 `acceptanceFocus`，避免 graph/runtime contract 再次分叉

## 验证

已通过：

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- src-tauri/src/agent/session.rs src-tauri/src/agent/graph.rs src-tauri/src/agent/planner.rs
npm exec vue-tsc -- --noEmit
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_acceptance_gate_into_long_term_memory -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_does_not_extract_acceptance_gate_from_incidental_acceptance_mentions -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture
```

结果：

- `vue-tsc` 通过
- 新增两条 `session` memory 单测通过
- `graph::tests` 共 `11` 条通过
- `planner::tests` 共 `15` 条通过

## 当前结果

- `PA-018` 在 `A. 结构边界` 上又前进了一步，因为 `LongTermMemory` 不再只有显式偏好、显式 note 和当前任务焦点，也开始覆盖显式交付门槛事实
- `PA-018` 在 `C. runtime 接入` 上也多了一条更深的结构化消费证据，因为 `acceptance gate` 已进入 `GraphTurnHandoff` 与 planner continue 摘要
- 当前仍不能宣布 `PA-018` 完成交付，因为 capability / bridge 层与更广范围的稳定事实来源仍未收口

## 下一步动作

1. 继续找出 capability / bridge 层里仍默认读取原始 `session/run/checkpoint` 的入口
2. 继续扩展 `LongTermMemory` 的其他显式、保守、可审计稳定事实来源
3. 接近收口时，再做一轮正式 closeout audit，判断是否足以把 `PA-018` 从 `In Progress` 切到 `Done`

## 当前卡点

- `acceptance gate` 已进入 memory / graph / planner，但还没有形成更大范围的 capability 或 UI 默认消费链
- 稳定事实来源虽然更丰富了，但仍不足以证明 `LongTermMemory` 已覆盖更完整的项目级事实面
