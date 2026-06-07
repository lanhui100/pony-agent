# Pony Agent 文档索引

## 1. 总览

- [项目记忆文件](C:/Users/HUAWEI/Documents/pony-agent/AGENT.md)
- [重构说明](C:/Users/HUAWEI/Documents/pony-agent/docs/tauri-rust-refactor.md)

## 2. 架构

- [架构总览](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/overview.md)
- [架构术语表](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/terminology.md)
- [Context/State 子系统 V1](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/context-state-subsystem.md)
- [Rust 运行时设计](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/runtime.md)
- [前端工作台架构](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/frontend-workbench.md)
- [Turn Lifecycle、Hooks 与 Recovery 架构基线](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/turn-lifecycle-hooks-and-recovery.md)
- [Session Control Plane 与 Audit Surface 架构基线](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md)

## 2.1 Session Control 主线

- [PA-042 canonical spec：history-control audit surface](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/session-control-audit-surface-and-history-evidence-summary/spec.md)
- [PA-043 canonical spec：run-control audit surface](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md)
- [PA-042 任务卡](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-042-build-session-control-audit-surface-and-history-evidence-summary.md)
- [PA-043 任务卡](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-043-build-run-control-audit-surface-and-summary-first-explainability.md)
- [PA-042 验收审计](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa042-acceptance-audit.md)
- [PA-043 验收审计](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa043-acceptance-audit.md)

## 3. 决策记录

- [决策记录索引](C:/Users/HUAWEI/Documents/pony-agent/docs/decisions/README.md)

## 4. 开发指南

- [前端开发指南](C:/Users/HUAWEI/Documents/pony-agent/docs/guides/frontend.md)
- [Rust 智能体开发指南](C:/Users/HUAWEI/Documents/pony-agent/docs/guides/rust-agent.md)

## 5. 学习记录

- [学习记录索引](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/INDEX.md)

## 6. 规范

- [工程规范与最佳实践](C:/Users/HUAWEI/Documents/pony-agent/docs/standards/engineering.md)
- [文档维护规范](C:/Users/HUAWEI/Documents/pony-agent/docs/standards/documentation.md)
- [OpenSpec 规范驱动开发约束](C:/Users/HUAWEI/Documents/pony-agent/docs/standards/spec-driven-development.md)

## 7. 任务系统

- [任务系统入口](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/README.md)
- [项目总控面板](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
- [任务板](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [PA-018 验收审计](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md)

## 8. OpenSpec

- [OpenSpec 根目录](C:/Users/HUAWEI/Documents/pony-agent/openspec)
- [复杂任务交付 canonical spec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/spec-driven-delivery/spec.md)
- [OpenSpec 归档目录](C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive)

## 9. 路线图

- [重构阶段计划](C:/Users/HUAWEI/Documents/pony-agent/docs/roadmap/phases.md)

## 10. 文档使用建议

- 想快速理解项目：先看 `AGENT.md`
- 想理解为什么这样做：看“决策记录”
- 想理解方向和边界：看“架构”与“路线图”
- 想开始写代码：看“开发指南”
- 想沉淀学习和未来写文章素材：看“学习记录”
- 想知道现在做到哪：看“任务系统”
- 想理解 session control 这一轮已经收口了什么：先看 `docs/architecture/session-control-plane-and-audit-surface.md`，再看 `PA-042 / PA-043` canonical specs
- 想保持工程质量：看“规范”
- 想推进复杂开发任务：看“OpenSpec”
