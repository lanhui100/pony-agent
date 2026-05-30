# Pony Agent Dashboard

## 项目状态

- 项目：`Pony Agent`
- 类型：学习模式重构项目
- 当前主线：`Phase 4 / Graph Runtime Mainline`
- 当前阶段：`Mainline Stabilizing`
- 总体状态：`In Progress`

## 当前主线结论

- `PA-013 / PA-014 / PA-017 / PA-019 / PA-023` 已完成并有代码落地
- 当前下一条主线仍然是 `PA-018`
- `PA-024` 已正式立项，但仍处于占位页阶段，不是当前主线第一优先级

## 当前重点

1. `PA-018` 已正式启动
   当前已经落地第一阶段 retrieval contract：`TurnContext / SessionContext / RunState / LongTermMemory / TranscriptContext / RetrievedContextState`
2. `runtime` 已开始消费 retrieval 结果
   `build_request()`、planner preflight、session summary 生成、图片召回判断与 graph handoff 已开始消费结构化 retrieval 结果
3. `LongTermMemory` 已从占位 contract 前进到最小真实边界
   `session.rs` 中已经有独立存储字段、显式语言/风格/文件引用/任务系统同步/变更范围偏好写入策略、显式当前任务焦点写入策略、显式 note 写入策略与最小审计字段，retrieval 会返回 `empty / available` 的真实状态
4. `planner` 已开始消费结构化 handoff facts
   continue 摘要已开始使用 `active_task_focus`、`last_referenced_file` 和 `long_term_memory_entry_count`，不再只复述 session summary
5. `host inspection` 已开始暴露 retrieval 视图
   inspection 响应现在可以直接返回 `RetrievedContextState`，宿主侧观察入口不必再自己拼 session/run 原件
6. `host retrieval` 已有原生查询面
   宿主层现在可以直接加载 `RetrievedContextState`，不必总是先走 inspection 复合响应
7. 前端 `runtime store` 已开始默认加载 retrieval 视图
   `loadSessionState()` 会默认加载 `RetrievedContextState`，并在 completed / failed / cancelled 后刷新 retrieval 状态，给后续 UI / capability 消费提供稳定入口
8. 前端 `runtime store` 主提交流程已开始优先消费 retrieval runState
   `submitTurn()` 在决定 `start / continue / resume_graph_run_stream` 时，会先读取 `retrievedContext.runState`；如果本地 retrieval facts 不足，会先通过宿主原生 `load_retrieved_context` 刷新一次 retrieval 视图，只有这层仍不足时才回退到 `inspect_host`
9. 前端 `HomeSidebar` run 面板也已开始优先消费 retrieval runState
   当前会先用 `retrievedContext.runState` 构造最小 run 视图；如果本地 retrieval facts 不足，会先刷新宿主原生 retrieval，再决定是否回退到 `inspect_host`
10. 前端 `HomeSidebar` 已有真实 retrieval 消费点
   当前只在右侧 observability 区做增量整合：保留原有 `状态 / Tools / Trace` 主体结构，并把 retrieval 以紧凑 summary 的形式并入 `Trace` 面板
11. 前端左栏 / 中栏已按边界回收 retrieval 常驻表达
   `HomeSessionSidebar` 不再承载“当前上下文”卡片，`HomeWorkspace` 也不再常驻 `Retrieved Context` 顶部条，避免导航区和对话主舞台混入 observability 信息
12. 前端 run 视图 contract 已开始跟进 `GraphTurnHandoff.active_task_focus`
   `HomeSidebar` 的 trace-run 组头仍可消费 `lastHandoff.activeTaskFocus`，由 retrieval 派生的最小 run 也会补上这条结构化事实
14. 前端 graph/retrieval contract 已开始收敛到共享类型
   `src/types/runtime.ts` 已导出 `GraphTurnHandoff / GraphRun / GraphRunCheckpoint / HostInspectionSnapshot` 等共享类型，`runtime store`、`HomeSidebar` 与 `HomeWorkspace` 已开始复用，减少局部类型漂移
15. 前端 retrieval run-state 映射也已开始收敛到共享 helper
   `normalizeGraphRunPhase()` 与 `deriveGraphRunFromRunState()` 已收进 `src/types/runtime.ts`，`HomeSidebar` 与 `HomeWorkspace` 不再各自手写一份最小 run 派生逻辑
16. 前端 `HomeWorkspace` graph run 刷新仍保持原实现基线
   本轮未再继续把 retrieval UI 扩到中栏；后续若需要让 workspace 消费 retrieval，只能在不改写对话主舞台结构的前提下做增量接入
17. `PA-018` 已形成独立正式验收审计文档
   当前正式审计载体为 `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`，后续 closeout 需要以它为主做逐项终审
18. `PA-020 / PA-021` 继续依赖 `PA-018`
   在 retrieval boundary 稳定前，不推进 MCP bridge 与 skills bridge 的正式接入
19. `PA-024` 继续保留在 backlog
   监控页已有导航和占位页，但真实 telemetry 聚合还未开始

## 当前代码证据

- retrieval contract 与默认实现：
  [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)
- long-term memory 最小存储边界：
  [session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- runtime 接入：
  [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- graph handoff 接入：
  [graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs)
- turn 规划载体：
  [turn_flow.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/turn_flow.rs)
- 架构边界文档：
  [context-state-subsystem.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/context-state-subsystem.md)

## 当前验证

本轮已验证：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run verify
```

结果：

- `context` 相关 `6` 个测试全部通过
- Rust `lib` 全量 `109` 个测试全部通过
- 前端 `51` 个测试、`vite build` 与 Rust `cargo check` 全部通过

注意：

- 本轮已经拥有 retrieval contract 的局部证据、Rust `lib` 全量证据和仓库级 `verify` 证据
- `PA-018` 最终收口前仍需继续扩展 `LongTermMemory` 的稳定事实写入来源，以及更深层 retrieval 消费链路
- 最新一轮前端定向验证也已再次通过：
  - `npm exec vitest run tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vue-tsc -- --noEmit`
  - `git diff --check -- src/components/HomeWorkspace.vue tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts`
- 最新一轮仓库级验证也已重新通过：
  - `npm run verify`
  - 当前结果为前端 `60` 个测试、`vite build` 与 Rust `cargo check` 全部通过
- 最新一轮 Rust memory / retrieval 定向验证也已通过：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture`
- 最新一轮 graph / planner retrieval 消费验证也已通过：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib`
  - 当前结果为 Rust `lib` 全量 `109` 个测试全部通过
- 最新一轮 retrieval UI active-task 消费验证也已通过：
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts`
  - `npm exec vue-tsc -- --noEmit`
  - 当前结果为这三组前端定向测试共 `26` 个测试全部通过
- 最新一轮 run-view contract 补齐验证也已通过：
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts`
  - `npm exec vue-tsc -- --noEmit`
  - `npm run build`
  - `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check`
  - 当前结果为前端构建与 Rust `cargo check` 全部通过
- 最新一轮共享 graph/retrieval 类型收敛验证也已通过：
  - `npm exec vue-tsc -- --noEmit`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts`
  - `npm run build`
  - 当前结果为 `44` 条定向测试全部通过，前端构建全部通过
- 最新一轮共享 run-state 映射收敛验证也已通过：
  - `npm exec vue-tsc -- --noEmit`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts`
  - `npm run build`
  - 当前结果为 `44` 条定向测试全部通过，前端构建全部通过
- 最新一轮 host retrieval 刷新后再 fallback 验证也已通过：
  - `npm exec vitest run tests/runtime-store.spec.ts tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts`
  - `npm exec vue-tsc -- --noEmit`
  - 当前结果为 `47` 条定向测试全部通过，前端类型检查通过

## 下一步最小动作

1. 继续把 runtime / graph 邻接边界切换到 `RetrievedContextState`
2. 继续把 `LongTermMemory` 从“显式用户偏好”扩展到更完整但仍可审计的稳定事实来源
3. 在 `PA-018` 边界稳定后，再推进 `PA-020 / PA-021`

## 关联入口

- 任务板：[01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- 当前任务卡：[PA-018](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md>)
- 正式验收审计：[PA-018 Acceptance Audit](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md>)
- 文档索引：[docs/INDEX.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/INDEX.md)
- 会话日志目录：[99_LOGS](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/99_LOGS)
