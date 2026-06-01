# Pony Agent 任务系统

## 目的

本目录是 Pony Agent 的正式任务管理系统。

这里负责：

- 计划
- 进度
- 续作
- 评审
- 归档

这里不负责：

- 项目长期事实
- 架构原则
- 技术栈说明

这些内容属于 `AGENT.md` 和 `docs/`。

## 目录结构

- `00_DASHBOARD.md`：项目总控面板
- `01_TASK_BOARD.md`：任务状态流
- `02_REVIEWS/`：审核记录
- `03_TASKS/`：任务卡
- `99_LOGS/`：会话日志和断点续跑记录

## 使用规则

1. 长期计划与推进只在这里维护
2. 每个持续任务都要落到任务卡
3. 每次会话结束都要补会话日志
4. 审核意见写入独立审核文件
5. 文档或代码发生关键变化时，要同步更新相关任务卡

## OpenSpec 集成

- OpenSpec 根目录固定为 `openspec/`
- 任务系统负责记录和推进；OpenSpec 在复杂开发任务开始实现时按需启动
- 复杂任务完成时，要同步维护：
  - `openspec/changes/<name>/`
  - `openspec/specs/`
  - `management/task-system/03_TASKS/*.md`
  - `management/task-system/99_LOGS/*.md`
  - 需要时补 `management/task-system/02_REVIEWS/*.md`

### 复杂任务判定

满足以下任一条件，默认按 spec 开发：

1. 新能力、新页面、新工作流
2. 跨 `runtime / graph / planner / memory / host / frontend` 边界
3. 涉及协议、数据结构、迁移、回滚、兼容性
4. 需要跨多个 session 推进
5. 明显需要多步骤拆解和依赖顺序

轻量 bugfix、文案微调、单文件小修可以不走完整 OpenSpec 流程。
仅处于任务收集、排期、Backlog/Ready 管理阶段的任务，也不要提前创建 OpenSpec change。

### 复杂任务执行顺序

1. 先用任务系统记录任务、状态和优先级
2. 当复杂任务真正开始开发时，建立 OpenSpec change
3. 产出 `proposal / specs / design / tasks`
4. 创建或更新任务卡，并回填 OpenSpec 路径
5. 按 spec 和任务卡实施
6. 完成后同步验收证据、canonical specs 与归档状态

## 状态流

- `Backlog`
- `Ready`
- `In Progress`
- `Review`
- `Blocked`
- `Done`
- `Dropped`

## 续作要求

每个进行中的任务都必须记录：

1. 当前做到哪
2. 下一步最小动作
3. 当前卡点

## 归档规则

- `Done` 任务保留在任务板与任务卡中
- 审核完成后可在后续按阶段归档
- 归档时必须保留完成结论和关联文档/代码路径
