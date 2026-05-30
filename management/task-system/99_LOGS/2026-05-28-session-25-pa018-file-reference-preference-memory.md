# 2026-05-28 Session 25 PA-018 File Reference Preference Memory

## 本轮目标

- 继续推进 `PA-018`
- 给 `LongTermMemory` 再补一条保守、显式、可审计的稳定事实来源
- 补验证并同步任务文档

## 本轮改动

- 更新：
  - `src-tauri/src/agent/session.rs`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `LongTermMemory` 新增第三条保守偏好写入来源：
  - `user_preference.file_reference_style`
- 当用户明确表达：
  - “请使用绝对路径”
  - “不要使用相对路径”
  - `use absolute paths`
  - `reference files with absolute paths`
  之类的显式要求时，会写入长期偏好记录
- 新记录内容为：
  - `Use absolute paths when referencing workspace files.`
- 该记录仍保持：
  - `source = explicit_user_message`
  - `updated_at_ms`
- 该偏好沿用 `user_preference.*` 的单槽 identity 规则，不会因为重复表达而无限追加

## 验证

已通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_explicit_file_reference_preference_into_long_term_memory
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_explicit_memory_note_without_overwriting_preferences
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
```

结果：

- 新增的文件引用偏好写入测试通过
- 既有显式 memory note 测试继续通过
- Rust `cargo check` 通过

## 当前结果

- `LongTermMemory` 现在已覆盖：
  - `user_preference.response_language`
  - `user_preference.response_style`
  - `user_preference.file_reference_style`
  - `user_memory.explicit_note`
- 这让 `PA-018` 的 memory 边界不再只停留在“语言 + 简洁 + note”两类最小偏好
- 但当前仍未形成更完整的稳定项目事实来源，因此还不能视为收口完成

## 下一步动作

1. 继续补一条新的、同样保守且显式的稳定事实来源，优先考虑明确项目约束或工作约束表达
2. 或继续把更深层的 `HomeWorkspace` graph/checkpoint/继续恢复提示切到 retrieval facts
3. 接近收口前，按 `PA-018` 验收标准逐条审计当前完成度

## 当前卡点

- `LongTermMemory` 的事实来源仍主要是偏好和显式 note，项目级稳定事实仍然偏少
- 前端已有多条 retrieval UI 消费链路，但 capability / bridge 层仍未系统迁移到 retrieval boundary
- 仍不能把 `PA-018` 标记完成，因为验收标准里的“更多上层默认拿 retrieval 或稳定衍生 contract”还未达到足够覆盖
