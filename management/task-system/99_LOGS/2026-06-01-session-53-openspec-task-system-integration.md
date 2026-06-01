# 2026-06-01 Session 53 OpenSpec Task System Integration

## 本次目标

- 研究 GitHub 开源 OpenSpec 的当前工作流
- 把 OpenSpec 接入 Pony Agent 当前仓库
- 把“复杂开发任务在开始开发时按需接 OpenSpec”正式写入任务系统

## 本次完成

- 确认 OpenSpec 当前主线是仓库内 `openspec/` + `changes/` + `specs/` 的 artifact-guided workflow
- 确认当前官方建议入口为 `/opsx:propose`
- 安装 `@fission-ai/openspec@1.3.1`
- 执行 `openspec init --tools codex --profile core --force`
- 生成 `openspec/config.yaml`、`.codex/skills/openspec-*` 与 Codex 全局 `opsx-*` prompts
- 新增 `docs/standards/spec-driven-development.md`
- 新增 canonical spec：`openspec/specs/spec-driven-delivery/spec.md`
- 更新 `AGENT.md`、`docs/INDEX.md`、任务系统入口、任务卡目录要求、Dashboard、Task Board
- 新增 `PA-027` 任务卡闭环记录本次治理接入

## 变更文件

- [AGENT.md](/C:/Users/HUAWEI/Documents/pony-agent/AGENT.md)
- [package.json](/C:/Users/HUAWEI/Documents/pony-agent/package.json)
- [package-lock.json](/C:/Users/HUAWEI/Documents/pony-agent/package-lock.json)
- [docs/INDEX.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/INDEX.md)
- [docs/standards/spec-driven-development.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/standards/spec-driven-development.md)
- [management/task-system/README.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/README.md)
- [management/task-system/03_TASKS/README.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/README.md)
- [management/task-system/00_DASHBOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
- [management/task-system/01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [management/task-system/03_TASKS/PA-027-integrate-openspec-into-task-system.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-027-integrate-openspec-into-task-system.md)
- [openspec/config.yaml](/C:/Users/HUAWEI/Documents/pony-agent/openspec/config.yaml)
- [openspec/specs/spec-driven-delivery/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/spec-driven-delivery/spec.md)

## 当前结果

- Pony Agent 现在已经具备正式 OpenSpec 基础设施
- 复杂开发任务在启动实现后不再只依赖聊天历史与任务卡，而是要求先有 proposal/spec/design/tasks
- 任务系统仍负责记录、排期、推进；OpenSpec 不会预先挂到所有 backlog/ready 任务上
- 当前 profile 为 `core`，足以支撑第一阶段 spec-first 工作流

## 下一步动作

1. 下一个复杂任务启动时，直接使用 `/opsx:propose "<change>"`
2. 在真实复杂任务中验证这套流程是否需要升级到 `custom` profile
3. 如果 `core` profile 不够，再补 `verify / sync / continue / new` 等 workflow

## 断点续跑提示

如果后续要继续完善这套集成，先检查：

- OpenSpec 的实际使用摩擦是否主要来自 profile 不够
- 任务卡字段是否还需要增加 `archive status` 或 `change owner`
- 是否要为关键产品域预建更多 `openspec/specs/<capability>/spec.md`
