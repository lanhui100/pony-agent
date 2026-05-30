# PA-016 建立附件中心索引与资产目录底座

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
在 `PA-011` 的“session 内最小多模态记忆”之上，正式引入附件中心的资产层，把“消息里引用的附件”与“可跨会话管理的附件资产”拆开，为后续检索、生命周期与管理面建立统一索引。

## 输出
- `AttachmentAsset` / `AttachmentReference` 第一版数据结构
- 跨会话附件索引
- session 附件与资产目录的映射规则
- 最小附件查询入口
- 附件中心边界文档

## 验收标准
- 能明确区分“消息附件引用”与“附件资产目录”
- 同一附件资产不会因为多次引用而无边界复制
- 系统可以跨会话列出附件资产，而不必扫描整份 `sessions.json`
- recent image recall 仍可继续工作，但不再独占附件数据模型
- 文档明确本卡不包含 TTL、清理台或完整搜索体验

## 完成情况
- 已新增 `AttachmentAsset` 与 `AttachmentReference`，并保留 `SessionAttachment = AttachmentReference` 兼容现有调用
- `PersistedStore` 已从只存 `sessions` 扩为：
- `sessions`
- `attachment_assets`
- `session_attachment_index`
- `save_input_attachments()` 现已在落盘 `.dataurl` 时同步生成 `asset_id`
- `append_turn()` 后会统一重建附件资产目录与 `session -> assetIds` 索引
- 旧会话文件加载时会自动回填 `asset_id` 并重建 catalog/index
- `SessionSnapshot.attachment_assets` 与前端 `attachmentAssets` 已接通持久化/恢复链路
- recent-image recall 仍保持“只认最近一条带附件的 user turn”约束，没有回退到跨多轮模糊重放

## 验证
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- `cargo test --manifest-path src-tauri/Cargo.toml --test session_regression`
- `npx vitest run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/providers.store.spec.ts`

## 后续衔接
- `PA-017` 在本卡资产目录之上补附件生命周期、检索与最小管理面
- 这张卡刻意不补 TTL、清理策略或完整搜索体验，避免再次把范围拉回 `PA-011`

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-011-expand-multimodal-session-memory.md`
- `docs/architecture/overview.md`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/context.rs`
- `src/stores/runtime.ts`
