# OpenSpec 规范驱动开发约束

## 目的

把 OpenSpec 正式接入 Pony Agent 的任务系统，让复杂开发任务先对齐变更意图、需求边界、技术方案与实施步骤，再进入编码。

目标不是把所有小改动都变重，而是把高风险、多阶段、跨边界任务从“聊天驱动”升级为“spec 驱动”。
任务系统仍然是任务记录与推进主线；OpenSpec 是复杂开发任务在启动实现时按需接入的规范层。

## 何时必须使用 OpenSpec

满足以下任一条件，默认必须先创建 OpenSpec change，再进入实现：

- 引入新能力、新页面、新工作流或新的产品行为
- 变更 `runtime / graph / planner / memory / host control plane / frontend observability` 等跨层边界
- 涉及 API、数据结构、持久化、协议、迁移、回滚或兼容性约束
- 预计需要跨多个 session 持续推进
- 需要拆成 5 个以上实现步骤，或明显存在依赖顺序
- 需求本身还不够清晰，需要先通过 spec 固化范围和非目标

注意：这里的“必须”指进入开发实现前，而不是在 Backlog、Ready 或单纯排期阶段就要预先创建 change。

## 何时可以不走完整 OpenSpec

以下轻量任务可以直接进入现有任务卡与实现流程：

- 明确的小 bug 修复
- 文案、注释、样式微调
- 不改变对外行为的局部重构
- 单文件、小范围、可在一个 session 内完成的维护工作

如果一开始判断为轻量任务，但推进中出现跨边界、跨 session 或需求不清的情况，应补建 OpenSpec change。

## 标准流程

1. 先用任务系统记录任务、优先级、状态和断点续跑信息。
2. 当任务进入复杂开发启动阶段时，用 `/opsx:propose "<change>"` 或等价 `openspec` CLI 建立 change。
3. 在 `openspec/changes/<name>/` 下补齐 `proposal.md`、`specs/`、`design.md`、`tasks.md`。
4. 为该 change 创建或更新 `management/task-system/03_TASKS/*.md` 任务卡，并回填 OpenSpec 链接。
5. 实施时按 `tasks.md` 推进，过程中同步更新任务卡当前进展、下一步动作和卡点。
6. 验证完成后，按 `docs/standards/engineering.md` 的 5 个质量门整理完成证据，再写回任务卡、日志和需要的 review 文件。
7. 需要沉淀为长期规范时，同步 `openspec/specs/`。
8. change 完成后执行归档，保持 `openspec/changes/archive/` 与任务系统结论一致。

## 与现有任务系统的映射

- `openspec/changes/<name>/proposal.md`：回答为什么做、范围是什么
- `openspec/changes/<name>/specs/**/spec.md`：回答行为要求和验收场景
- `openspec/changes/<name>/design.md`：回答怎么做、边界怎么切
- `openspec/changes/<name>/tasks.md`：回答实施顺序和最小工作单元
- `management/task-system/03_TASKS/*.md`：回答当前状态、当前证据、下一步动作、卡点
- `management/task-system/02_REVIEWS/*.md`：回答正式审计和验收结论
- `management/task-system/99_LOGS/*.md`：回答本次 session 做了什么，如何断点续跑

## 复杂任务卡新增要求

复杂任务的任务卡除原字段外，还应显式补充：

- `OpenSpec Change`：对应的 `openspec/changes/<name>/`，如果尚未进入开发可先写 `待启动`
- `Canonical Spec`：相关 `openspec/specs/<capability>/spec.md`
- `Spec 状态`：如 `未开始`、`proposal ready`、`design ready`、`tasks in progress`、`archived`

## 复杂任务的验证口径

复杂任务默认不应只用“测试通过”作为完成依据，而应按 5 个质量门给出证据：

1. `Gate 1 / 生成前约束`：spec 是否已经明确目标、边界、非目标和验收标准。
2. `Gate 2 / 静态质量门`：是否完成编译、类型、lint、基础 contract 检查。
3. `Gate 3 / 行为质量门`：是否有对应该变更面的测试或最小主链验证。
4. `Gate 4 / 系统质量门`：是否覆盖兼容性、可观测性、性能、恢复或跨端风险。
5. `Gate 5 / 发布质量门`：是否说明迁移、回滚、监控点和已知风险。

如果某一层当前不适用，任务卡和 review 文档里必须显式写明跳过原因，而不是默认省略。

## 本仓库当前默认实践

- OpenSpec 根目录：`openspec/`
- Codex workflow skills：`.codex/skills/openspec-*`
- Codex slash prompts：`%USERPROFILE%\\.codex\\prompts\\opsx-*.md`
- 当前默认 profile：`core`
- 当前推荐起点：`/opsx:propose "..."`，复杂任务实现前先出 proposal/spec/design/tasks
