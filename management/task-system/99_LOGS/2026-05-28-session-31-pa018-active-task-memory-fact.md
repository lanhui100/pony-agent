# 2026-05-28 Session 31 PA-018 Active Task Memory Fact

## 本轮目标

- 继续推进 `PA-018`
- 给 `LongTermMemory` 再补一条保守、可审计、稳定的项目事实来源
- 补齐对应 Rust 验证与任务系统回写

## 本轮改动

- 更新：
  - `src-tauri/src/agent/session.rs`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 为 `LongTermMemory` 新增显式项目焦点事实：
  - `project_focus.active_task`
- 当前只在用户消息明确表达：
  - “现在开始 PA-018”
  - “当前优先推进 PA-018”
  - `focus on PA-018`
  - `start with PA-018`
  这类语义时写入
- 新规则只会在同时识别到任务样式标识时落地，并且同类记录按 `kind` 覆盖更新，只保留当前激活任务
- 这让 `LongTermMemory` 不再只有“用户偏好”和“显式 note”，开始覆盖更贴近项目推进状态的稳定事实

## 验证

已通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_
cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
npm run verify
git diff --check -- src-tauri/src/agent/session.rs
```

结果：

- `append_turn_` 相关 `8` 条 Rust 定向测试全部通过，其中包含：
  - 提取显式当前任务焦点
  - 任务焦点切换时覆盖更新
  - 普通任务提及不误写入
- `context::tests` 的 `6` 条 retrieval contract 测试全部通过
- `cargo check` 通过
- `npm run verify` 重新通过，当前结果为前端 `58` 个测试、`vite build` 与 Rust `cargo check` 全部通过
- `git diff --check` 无空白错误，只有仓库现有 LF/CRLF 提示

## 当前结果

- `PA-018` 在 `A. 结构边界` 上更进一步，因为 `LongTermMemory` 已从“偏好 + note”扩展到一类显式项目事实
- 这条事实对后续 planner / capability / 任务系统协同都更有价值，也更接近真正的 retrieval boundary 目标
- 但仍不能关单，因为更广范围的 capability 默认消费与更多稳定事实来源还没补齐

## 下一步动作

1. 继续扩展 `LongTermMemory` 的其他保守稳定事实来源
2. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口
3. 在接近收口前，按 `PA-018` 验收标准逐项补齐最终证据

## 当前卡点

- 稳定项目事实来源虽然增加了，但整体覆盖仍不够高
- capability / bridge 层对 retrieval boundary 的系统迁移仍未完成
- 现有验证已较强，但还不足以直接支撑最终关单
