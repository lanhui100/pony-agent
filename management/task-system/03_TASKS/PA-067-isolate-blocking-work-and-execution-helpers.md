# PA-067 收口 blocking 工作并建立统一执行 helper

## 状态
- Status: `Ready`
- Priority: `P2`
- Owner: `待定`

## 依赖
- 可并行于：`PA-066`
- 建议在 `PA-068` 前完成

## Canonical Spec
- `openspec/changes/isolate-blocking-work-and-execution-helpers/specs/agent-workspace-contract/spec.md`

## OpenSpec Change
- `openspec/changes/isolate-blocking-work-and-execution-helpers/`

## 背景
即使 provider 走 async，runtime 里仍会有本地文件 IO、CPU 密集逻辑和第三方同步边界。需要在架构上明确哪些工作必须进入 `spawn_blocking`，避免新的 async 任务被同步工作重新拖住。

## 目标
1. 系统性盘点 blocking 工作
2. 为 blocking / CPU 密集边界建立统一 helper
3. 减少临时散落的 `spawn_blocking` / 同步调用

## In Scope
- 文件 IO / SQLite / 序列化 / 重计算盘点
- 统一 blocking helper / wrapper
- 对不能 async 化的边界做显式隔离

## Out of Scope
- provider HTTP async 化本身
- per-session task 模型

## 验收标准
- blocking 工作 SHALL 被显式分类，而不是混在 async task 内直接执行
- 代码库 SHALL 有统一的 blocking helper / execution helper，而不是散落调用
- `tauri_adapter.rs` 中的 `spawn_turn_stream` 与 `spawn_graph_run_stream` SHALL 提供 PA-068 可接管的转换接口
- 所有经过 helper 迁移的 `spawn_blocking` 调用点 SHALL 有对应的测试覆盖

## 下一步动作
1. 盘点所有 blocking 边界
2. 建 helper
3. 替换最热路径上的手工 blocking 调用

## 断点续跑提示
- `src-tauri/src/tauri_adapter.rs`
- `crates/pony-agent-core/src/agent/session.rs`
- `crates/pony-agent-core/src/agent/frontend_diagnostics.rs`

## 当前进展
- 已完成

## 完成摘要
- 创建 `BlockingHelper::spawn` 统一 blocking worker helper
- 6 个前端诊断 Tauri command 从 `tauri::async_runtime::spawn_blocking` 迁移到 `BlockingHelper::spawn`
- `tauri_adapter.rs` 添加 PA-068 transition target 标记
- PA-068 过渡接口已就绪（`TaskCleanupGuard`）
- Tauri app 测试 30 项全部通过
- 3 轮并行智能体审核后调优
- 清理与收口已完成
