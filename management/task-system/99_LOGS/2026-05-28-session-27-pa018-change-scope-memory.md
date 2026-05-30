# 2026-05-28 Session 27 PA-018 Change Scope Memory

## 本轮目标

- 继续推进 `PA-018`
- 再补一条保守、显式、可审计的 `LongTermMemory` 稳定事实来源
- 让 memory 边界继续贴近真实协作约束

## 本轮改动

- 更新：
  - `src-tauri/src/agent/session.rs`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `LongTermMemory` 新增第五条保守偏好写入来源：
  - `user_preference.change_scope`
- 当用户明确表达：
  - “不要修改无关文件”
  - “不要回滚无关改动”
  - `don't modify unrelated files`
  - `do not touch unrelated changes`
  之类的显式要求时，会写入长期偏好记录
- 新记录内容为：
  - `Avoid modifying unrelated existing changes.`
- 该记录仍保持：
  - `source = explicit_user_message`
  - `updated_at_ms`
- 该偏好沿用 `user_preference.*` 的单槽 identity 规则，不会因为重复表达而无限追加

## 验证

已通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_change_scope_preference_into_long_term_memory
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_task_system_sync_preference_into_long_term_memory
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_explicit_file_reference_preference_into_long_term_memory
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
```

结果：

- 新增的变更范围偏好写入测试通过
- 上轮新增的任务系统同步偏好与文件引用偏好测试继续通过
- Rust `cargo check` 通过

## 当前结果

- `LongTermMemory` 现在已覆盖：
  - `user_preference.response_language`
  - `user_preference.response_style`
  - `user_preference.file_reference_style`
  - `user_preference.task_system_sync`
  - `user_preference.change_scope`
  - `user_memory.explicit_note`
- 这让当前 memory 边界不仅能记住回答风格，还能记住文件引用、任务系统同步与变更范围这类真实执行约束
- 但当前仍主要是显式偏好与显式 note，尚未形成更完整的稳定项目事实层

## 下一步动作

1. 继续补一条新的、同样保守且显式的稳定事实来源，优先考虑明确项目约束或工作约束表达
2. 或继续把 `HomeWorkspace` 更深层的 graph/checkpoint/继续恢复提示切到 retrieval facts
3. 接近收口前，按 `PA-018` 验收标准逐条审计当前完成度

## 当前卡点

- `LongTermMemory` 的事实来源虽然更完整了，但仍偏向显式偏好，项目级稳定事实仍然偏少
- 前端已有多条 retrieval UI 消费链路，但 capability / bridge 层仍未系统迁移到 retrieval boundary
- 仍不能把 `PA-018` 标记完成，因为验收标准里的“更多上层默认拿 retrieval 或稳定衍生 contract”还未达到足够覆盖
