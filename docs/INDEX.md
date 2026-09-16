# Pony Agent 文档索引

## 1. 总览

- [项目记忆文件](../AGENT.md)
- [重构说明](tauri-rust-refactor.md)
- [设计基础](design-foundations.md)
- [仓库对比索引](repo-comparison-index.md)
- [Spec 运行时历史一致性](spec-runtime-history-consistency.md)
- [上下文紧缩原则](context-compaction-principles.md)
- [Codex / Hermes / Claude Code 对比说明](codex-hermes-claude-code-对比说明.md)

## 2. 架构

- [架构总览](architecture/overview.md)
- [架构术语表](architecture/terminology.md)
- [Context/State 子系统 V1](architecture/context-state-subsystem.md)
- [Rust 运行时设计](architecture/runtime.md)
- [前端工作台架构](architecture/frontend-workbench.md)
- [Thinking 参数适配器](architecture/thinking-param-adapter.md)
- [Turn Lifecycle、Hooks 与 Recovery 架构基线](architecture/turn-lifecycle-hooks-and-recovery.md)
- [Session Control Plane 与 Audit Surface 架构基线](architecture/session-control-plane-and-audit-surface.md)
- [Runtime Ownership Split (PA-065)](architecture/runtime-ownership-split.md)
- [Async Provider IO Migration (PA-066)](architecture/async-provider-io-migration.md)
- [Blocking Helper Unification (PA-067)](architecture/blocking-helper-unification.md)
- [Per-Session Async Turn Task Model (PA-068)](architecture/per-session-async-turn-task-model.md)
- [Tool Descriptor / Registry / Governed Dispatcher 真相源 (PA-076)](architecture/tool-runtime-descriptor-registry.md)
- [锁序规范](concurrency/lock-ordering.md)

## 2.1 Session Control 主线

- [PA-042 canonical spec：history-control audit surface](../openspec/specs/session-control-audit-surface-and-history-evidence-summary/spec.md)
- [PA-043 canonical spec：run-control audit surface](../openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md)
- [PA-042 任务卡](../management/task-system/03_TASKS/PA-042-build-session-control-audit-surface-and-history-evidence-summary.md)
- [PA-043 任务卡](../management/task-system/03_TASKS/PA-043-build-run-control-audit-surface-and-summary-first-explainability.md)
- [PA-042 验收审计](../management/task-system/02_REVIEWS/2026-06-05-pa042-acceptance-audit.md)
- [PA-043 验收审计](../management/task-system/02_REVIEWS/2026-06-05-pa043-acceptance-audit.md)

## 2.5 性能与诊断

- [Turn 完成时 UI 冻结：根因诊断与解决路径](analysis/turn-completion-ui-freeze-diagnosis-2026-06-16.md)
- [Trace Redteam 审计 (2026-06-01)](analysis/trace-redteam-2026-06-01.md)
- [上下文构建与缓存策略 canonical spec](../openspec/specs/context-assembly-and-cache-strategy/spec.md)
- [PA-056 任务卡](../management/task-system/03_TASKS/PA-056-redesign-context-assembly-and-cache-strategy.md)

## 2.6 工具面研究与对比

- [内置工具面三方对比：Pony Agent vs Codex vs Claude Code](analysis/builtin-tool-surface-comparison-2026-06-22.md)

## 3. 决策记录

- [决策记录索引](decisions/README.md)
- [0008 会话数据架构向事件溯源演进（分阶段落地）](decisions/0008-event-sourcing-evolution.md)

## 4. 开发指南

- [前端开发指南](guides/frontend.md)
- [Rust 智能体开发指南](guides/rust-agent.md)
- [前端分栏与可观测性边界](guides/frontend-layout-and-observability-boundary.md)
- [端到端系统测试方案](guides/e2e-system-test-plan.md)

## 5. 学习记录

- [学习记录索引](learning/INDEX.md)

## 6. 规范

- [工程规范与最佳实践](standards/engineering.md)
- [文档维护规范](standards/documentation.md)
- [OpenSpec 规范驱动开发约束](standards/spec-driven-development.md)

## 7. 任务系统

- [任务系统入口](../management/task-system/README.md)
- [项目总控面板](../management/task-system/00_DASHBOARD.md)
- [任务板](../management/task-system/01_TASK_BOARD.md)
- [PA-018 验收审计](../management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md)
- [PA-056 任务卡](../management/task-system/03_TASKS/PA-056-redesign-context-assembly-and-cache-strategy.md)
- [PA-056 Spec 审核](../management/task-system/02_REVIEWS/2026-06-16-pa056-spec-review.md)
- [PA-056 Code 审核](../management/task-system/02_REVIEWS/2026-06-16-pa056-code-review.md)
- [PA-056 Code 审核 Follow-up](../management/task-system/02_REVIEWS/2026-06-17-pa056-code-review-followup.md)
- [PA-056 本轮收口日志](../management/task-system/99_LOGS/2026-06-17-pa056-workspace-mode-closeout.md)
- [PA-076 阶段 1–7 + runtime 切换收口日志](../management/task-system/99_LOGS/2026-08-02-pa076-phases-1-7-and-runtime-switch.md)
- [PA-076 阶段 3 审核](../management/task-system/02_REVIEWS/2026-08-02-pa076-phase3-review.md)

## 8.1 工具面新增变更

- [fix-provider-registry-fallback-test-baseline OpenSpec 变更](../openspec/changes/fix-provider-registry-fallback-test-baseline/proposal.md)
- [workspace_read_document（anydoc 文档转换）OpenSpec 变更（已归档）](../openspec/changes/archive/2026-08-06-workspace-read-document-anydoc/proposal.md)
- [workspace_read_document canonical spec](../openspec/specs/workspace-read-document/spec.md)
- [workspace_read_document 收口日志](../management/task-system/99_LOGS/2026-08-06-workspace-read-document-anydoc-closeout.md)

## 8. OpenSpec

- [OpenSpec 根目录](../openspec)
- [复杂任务交付 canonical spec](../openspec/specs/spec-driven-delivery/spec.md)
- [事件溯源演进：阶段 1 turn 内事件日志（ADR 0008）](../openspec/changes/turn-event-log/proposal.md)
- [事件溯源演进：阶段 2 投影层（ADR 0008）](../openspec/changes/session-projection-layer/proposal.md)
- [事件溯源演进：阶段 3 checkpoint 引用化（ADR 0008）](../openspec/changes/checkpoint-event-referencing/proposal.md)
- [事件溯源演进：阶段 4 trace 事件化（ADR 0008）](../openspec/changes/trace-event-projection/proposal.md)
- [第一波工具面 canonical spec](../openspec/specs/first-wave-tool-surface/spec.md)
- [第二波工具面 canonical spec](../openspec/specs/second-wave-tool-surface/spec.md)
- [第三波工具面 canonical spec](../openspec/specs/third-wave-default-tool-alignment/spec.md)
- [Tool Runtime Dispatch canonical spec](../openspec/specs/tool-runtime-dispatch/spec.md)
- [Process Tool Lifecycle canonical spec](../openspec/specs/process-tool-lifecycle/spec.md)
- [Web Access Safety canonical spec](../openspec/specs/web-access-safety/spec.md)
- [上下文构建与缓存策略 canonical spec](../openspec/specs/context-assembly-and-cache-strategy/spec.md)
- [工具权限模型 canonical spec](../openspec/specs/tool-permission-contract/spec.md)
- [工具可观测性 canonical spec](../openspec/specs/tool-observability-contract/spec.md)
- [trace 面板折叠与懒挂载 canonical spec](../openspec/specs/trace-panel-collapse/spec.md)
- [trace 面板 turn 级虚拟滚动 canonical spec](../openspec/specs/trace-panel-virtual-scroll/spec.md)
- [trace 渲染快照投影 canonical spec](../openspec/specs/trace-render-snapshot/spec.md)
- [composer 输入优先级隔离 canonical spec](../openspec/specs/composer-input-priority/spec.md)
- [OpenSpec 归档目录](../openspec/changes/archive)

## 9. UI/UX 设计

- [产品 UI 规范](design/product-ui-spec.md)
- [应用图标](design/app-icon.md)

## 10. 路线图

- [重构阶段计划](roadmap/phases.md)

## 11. 分析文档（索引见 §2.5 性能与诊断、§2.6 工具面研究）

## 12. 文档使用建议

- 想快速理解项目：先看 `AGENT.md`
- 想理解为什么这样做：看"决策记录"
- 想理解方向和边界：看"架构"与"路线图"
- 想开始写代码：看"开发指南"
- 想沉淀学习和未来写文章素材：看"学习记录"
- 想知道现在做到哪：看"任务系统"
- 想理解 session control 已经收口了什么：先看 `docs/architecture/session-control-plane-and-audit-surface.md`，再看 `PA-042 / PA-043` canonical specs
- 想保持工程质量：看"规范"
- 想推进复杂开发任务：看"OpenSpec"
- 想看第二波 builtin 工具面收口范围：先看 `openspec/specs/second-wave-tool-surface/spec.md`，再看 `PA-050 ~ PA-054` 与对应审核/收口日志
- 想对比三方工具面差距、决策后续该实现哪些工具：看 `docs/analysis/builtin-tool-surface-comparison-2026-06-22.md`
- 想理解 thinking 参数适配：看 `docs/architecture/thinking-param-adapter.md`
- 想新增或修改工具、理解工具元数据从哪来、工具如何被治理执行：看 `docs/architecture/tool-runtime-descriptor-registry.md`（含 registry 顺序与工具名透传两条不变量、governed dispatcher 八步管线、runtime 默认切换、Ask/Plan/process/sandbox/web/search/phase-7 模块落点、Sandbox 裁决记录与一条剩余 integrator note（真实 SandboxBackend），以及 Windows 上跑 Rust 测试的方式）
