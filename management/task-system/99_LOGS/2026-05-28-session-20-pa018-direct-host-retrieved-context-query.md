# 2026-05-28 Session 20 PA-018 Direct Host Retrieved Context Query

## 本轮目标

- 继续推进 `PA-018`
- 给宿主层补一个 retrieval 原生查询入口
- 补验证并同步任务文档

## 本轮改动

- 更新：
  - `src-tauri/src/agent/control_plane.rs`
  - `src-tauri/src/agent/runtime.rs`
  - `src-tauri/src/lib.rs`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 宿主层新增 retrieval 原生查询面：
  - `load_retrieved_context`
- control plane 新增：
  - `RetrievedContextQuery`
  - `load_retrieved_context(...)`
- Tauri 命令层也新增：
  - `load_retrieved_context`
- 这意味着上层不必一定先走 `inspect_host` 复合响应，再从中拆出 retrieval 结果
- 现在可以直接按：
  - `turn_id`
  - `session_id`
  - `run_id`
  查询统一的 `RetrievedContextState`

## 验证

已通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
```

结果：

- Rust `lib` 全量 `103` 个测试全部通过
- 新增的 direct retrieved context 查询测试通过
- `cargo check` 通过

## 当前结果

- `PA-018` 现在不只是“宿主 inspection 可以附带 retrieval”
- 宿主层已经拥有正式的 retrieval 原生查询面
- 这让后续前端、宿主能力入口或上层 bridge 更容易直接基于统一 retrieval contract 工作

## 下一步动作

1. 继续盘点还有哪些上层入口仍然默认暴露原始 session artifacts，而不是 retrieval 视图
2. 继续扩展 `LongTermMemory` 的稳定事实来源，但保持可审计和保守写入
3. 在更接近收口前，做一次针对当前验收标准的逐条完成度审计

## 当前卡点

- 宿主层已有 retrieval 原生查询面，但前端与更多上层能力还没有系统性迁移到这一入口
- `LongTermMemory` 仍主要覆盖显式用户表达，还没有覆盖更完整的稳定事实来源
