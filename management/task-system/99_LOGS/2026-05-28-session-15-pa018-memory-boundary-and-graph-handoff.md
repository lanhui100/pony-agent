# 2026-05-28 Session 15 PA-018 Memory Boundary And Graph Handoff

## 本轮目标

- 继续推进 `PA-018`
- 给 `LongTermMemory` 补最小真实存储边界
- 让 graph handoff 开始消费 retrieval 结果
- 补测试、验证并同步任务文档

## 本轮改动

- 更新：
  - `src-tauri/src/agent/session.rs`
  - `src-tauri/src/agent/context.rs`
  - `src-tauri/src/agent/runtime.rs`
  - `src-tauri/src/agent/graph.rs`
  - `src-tauri/src/agent/control_plane.rs`
  - `src-tauri/src/agent/planner.rs`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 在 `session.rs` 中新增 `LongTermMemoryRecord`
- 为 `SessionState / SessionSnapshot` 增加 `long_term_memory_entries`
- 新增 `SessionStore.replace_long_term_memory(...)`，让长期记忆拥有最小独立写入接口
- 为 `LongTermMemory` 增加第一条保守写入策略：只从用户消息中提取明确表达的长期偏好
- 为 `LongTermMemory` 记录补充最小审计字段：
  - `source`
  - `updated_at_ms`
- retrieval 现在会把 `long_term_memory_entries` 映射为：
  - `status = empty`
  - `status = available`
- `runtime.build_graph_turn_handoff()` 现在会先构建 `RetrievedContextState`
- `GraphEngine.build_turn_handoff()` 现在消费的是 retrieval 结果，而不是原始 `SessionSnapshot`
- `runtime.persist_turn_outcome()` 现在会重新走 retrieval，再生成 session summary
- handoff 已带出结构化字段：
  - `run_id / run_phase`
  - `last_referenced_file`
  - `recent_attachment_asset_count`
  - `long_term_memory_status / long_term_memory_entry_count`

## 验证

已通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run verify
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
```

结果：

- `context` 相关 `6` 个测试全部通过
- Rust `lib` 全量 `99` 个测试全部通过
- 前端 `51` 个测试、`vite build` 与 Rust `cargo check` 全部通过
- `npm run verify` 中仍有两类非阻塞 warning：
  - `@vueuse/core` 的 Rollup 注释 warning
  - 一次 `incremental compilation session directory ... 拒绝访问 (os error 5)`
- 随后单独执行的 `cargo check --target-dir target-check` 已无新增 Rust 代码 warning

## 当前结果

- `PA-018` 已不再只有 retrieval contract，开始具备最小可落地的 memory storage boundary
- `LongTermMemory` 已不再只是“可存”，而是已经有了保守但真实的写入来源和最小审计信息
- graph handoff 已经开始消费结构化 retrieval 结果，说明 `PA-018` 正在真正成为后续 graph / capability 层的前置底座
- 任务仍处于 `In Progress`，因为 memory 写入策略和更深层消费替换还没有收口

## 下一步动作

1. 继续把 `LongTermMemory` 从“显式用户偏好”扩展到更完整但仍可审计的稳定事实来源
2. 继续把更多 runtime / graph / capability 邻接边界切到 `RetrievedContextState`
3. 继续扩大 handoff 之后的 graph / capability 结构化消费，减少对原始 session artifacts 的直接依赖

## 当前卡点

- `LongTermMemory` 已有最小存储边界和显式偏好写入策略，但还没有覆盖更完整的稳定事实来源
- graph handoff 已经接入 retrieval，但更深层 graph / capability 消费还没有完全替换
