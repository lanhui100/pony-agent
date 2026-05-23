# Pony Agent Task Board

## Backlog
- 暂无

## Ready
- 暂无

## In Progress
- `PA-004` 定义 provider 与 tool 抽象
- `PA-005` 把 Vue 工作台接入真实 turn 执行链路
- `PA-006` 实现新对话与历史对话管理
- `PA-008` 补强工具层（多工具、并发、权限、错误恢复）
- `PA-009` 完善 provider 能力配置（思考、多模态、上下文与模型能力）

## Review
- 暂无

## Blocked
- `PA-007` 拆分独立接入层（Tauri / HTTP-SSE adapter）
  说明：等待 `provider / tool / session / runtime` 的核心语义进一步稳定后再抽离

## Done
- `PA-000` 建立项目骨架、文档索引、任务系统和 ADR 体系
- `PA-001` 迁移前端到 Vue + Pinia
- `PA-002` 设计运行时调试台结构
- `PA-003` 实现 Rust 单轮 runtime 骨架
- `2026-05-23` 重构收口轮：完成 `provider / session / runtime / tool` 稳定化、18 条前端单测、`npm run verify` 闭环和 `tauri dev --no-watch` 冒烟验证

## Dropped
- 暂无
