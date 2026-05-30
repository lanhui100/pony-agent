# Pony Agent Dashboard

## 项目状态

- 项目：`Pony Agent`
- 类型：学习模式重构项目
- 当前主线：`Phase 4 / Graph Runtime Mainline`
- 当前阶段：`Mainline Stabilizing`
- 总体状态：`In Progress`

## 当前主线结论

- `PA-018` 已完成并通过完成态验证
- retrieval 观测与 trace 展示语义已正式拆到 `PA-024`
- `Build Context` 与 cache-friendly prompt 边界已正式拆到 `PA-025`
- `PA-020 / PA-021` 现在不再被 `PA-018` 阻塞

## 当前重点

1. `PA-018` 已关闭
   已落地 `RetrievedContextState / ContextStateRetriever` contract、runtime / graph / planner / 宿主默认查询面的 retrieval-first 消费链路、`LongTermMemory` 独立边界与项目级稳定事实来源。
2. 下一条建议主线是 `PA-025`
   用户已明确强调 cache 命中与稳定前缀边界，后续应单独收口 `RetrievedContextState -> prompt/request` 映射与 `Build Context` 语义。
3. `PA-024` 继续作为 retrieval observability 的正式承接卡
   后续 trace 中 retrieval 的观测意义、展示结构和监控面不再回到 `PA-018`。
4. `PA-020 / PA-021` 已具备继续细化前提
   retrieval boundary 稳定后，后续 capability bridge 与 skills bridge 的读取边界已更明确。

## 当前代码证据

- retrieval contract 与默认实现：
  [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)
- long-term memory 独立边界与稳定事实来源：
  [session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- runtime 接入：
  [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- graph handoff 与 planner 收口：
  [graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs)
  [planner.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/planner.rs)
- 宿主 retrieval-first 读面：
  [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
  [lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)
- 架构边界文档：
  [context-state-subsystem.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/context-state-subsystem.md)

## 当前验证

本轮完成态验证：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run verify
```

结果：

- Rust `planner` 定向测试通过：`16 passed`
- Rust `graph` 定向测试通过：`11 passed`
- Rust `session` long-term memory 提取定向测试通过：`10 passed`
- Rust `lib` 全量通过：`120 passed`
- `npm run verify` 通过：前端单测、前端构建与 Rust `cargo check` 全部通过

## 下一步最小动作

1. 启动 `PA-025`，把 cache-friendly prompt 边界从 `PA-018` 彻底分离并正式收口。
2. 启动 `PA-024`，把 retrieval observability 的 trace 语义、展示结构与监控面做成正式任务。
3. 在 retrieval boundary 稳定前提下，继续细化 `PA-020 / PA-021` 的 capability 读取边界。

## 关联入口

- 任务板：[01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- 已完成任务卡：[PA-018](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md>)
- 正式验收审计：[PA-018 Acceptance Audit](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md>)
- 文档索引：[docs/INDEX.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/INDEX.md)
- 会话日志目录：[99_LOGS](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/99_LOGS)
