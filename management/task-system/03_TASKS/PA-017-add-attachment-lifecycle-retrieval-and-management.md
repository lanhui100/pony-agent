# PA-017 补齐附件生命周期、检索与管理面

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## 目标
基于 `PA-016` 的附件资产目录，补齐附件中心的生命周期策略、跨会话检索与最小管理面，让附件不再只是“能被最近一轮找回”，而是成为可审计、可清理、可查询的产品级资产。

## 输出
- 附件 TTL / 清理策略
- 最小附件检索与筛选入口
- 手动管理动作草案（查看 / 清理 / 失效 / 导出元数据）
- 附件生命周期事件与审计字段
- 附件中心管理面边界说明

## 验收标准
- 能按附件名、会话、时间或 MIME 类型做最小检索
- 能区分“仍被引用”“已过期”“可清理”“缺失 payload”等资产状态
- 系统能执行最小清理策略，而不破坏仍被引用的消息历史
- 宿主或前端可以读取附件资产列表与状态，而不直接扫描磁盘目录
- 文档明确本卡不强行引入复杂语义搜索或向量索引

## 当前进展
- `PA-011` 已完成最小多模态 session memory
- `PA-016` 预计先把附件资产层与跨会话索引建立起来
- 当前仍没有 TTL、批量清理、手动管理台与跨会话检索
- 当前 recent image recall 逻辑仍偏向“最近一次有附件的用户消息”

## 本轮验收
- 已落地 `AttachmentLifecycleStatus / AttachmentAssetQuery / AttachmentCleanupRequest / AttachmentCleanupResult`
- 已补齐 `active / missing_payload / reclaimable / expired` 资产状态、最小检索与显式 cleanup
- 已完成测试隔离修复，避免共享附件根目录污染 `session_regression`
- 已通过：
  - `cargo test --manifest-path src-tauri/Cargo.toml --test session_regression`
  - `npx vitest run tests/runtime-store.spec.ts`

## 下一步动作
- 在资产目录稳定后补状态字段与生命周期策略
- 把最小检索和管理动作放到统一附件中心入口
- 视需要再决定是否为图片之外的附件补摘要、标签或更强搜索索引

## 当前卡点
- 如果在没有资产目录的前提下直接做 TTL 和清理，很容易误删仍被引用的附件，或把历史消息与物理文件状态搞乱

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-016-build-attachment-center-index-and-catalog.md`
- `management/task-system/03_TASKS/PA-011-expand-multimodal-session-memory.md`
- `docs/architecture/overview.md`
- `src-tauri/src/agent/session.rs`
