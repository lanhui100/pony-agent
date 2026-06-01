# PA-027 接入 OpenSpec 并升级复杂任务交付流程

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`
- Started At: `2026-06-01`
- Completed At: `2026-06-01`

## 目标
把 GitHub 开源的 OpenSpec 接入 Pony Agent 当前仓库与任务系统，让复杂开发任务在真正开始开发时按需进入 spec-first 流程，再进入实现与验收。

## 输出
- 仓库内 OpenSpec 目录与基础配置
- Codex 可用的 OpenSpec workflow skills 与 prompts
- 一份面向本仓库的 spec-driven 开发规范
- 任务系统中的复杂任务判定与执行规则
- 一份 canonical spec，约束“复杂任务必须先有 spec”
- 明确“任务系统负责记录，OpenSpec 在开发启动时接入”，而不是给现有任务预挂 change

## OpenSpec Change
- 本次为治理接入与基线初始化，直接建立仓库级 canonical spec 与流程规则，后续复杂任务从 `openspec/changes/` 起步

## Canonical Spec
- [spec-driven-delivery/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/spec-driven-delivery/spec.md)

## Spec 状态
- `OpenSpec initialized`
- `Canonical workflow spec established`

## 验收标准
- 仓库存在 `openspec/`，并已配置 `spec-driven` schema
- Codex 可直接使用 `/opsx:propose`、`/opsx:explore`、`/opsx:apply`、`/opsx:archive`
- 仓库内文档明确复杂开发任务何时必须先走 OpenSpec
- 仓库内文档明确 OpenSpec 是在开始开发时按需接入，而不是在 backlog 阶段全量预建
- 任务系统明确复杂任务需要回填 OpenSpec change、canonical spec 与 spec 状态
- 本仓库已有至少一份 canonical spec 约束复杂任务交付流程

## 当前进展
- 已安装 `@fission-ai/openspec@1.3.1`
- 已执行 `openspec init --tools codex --profile core --force`
- 已生成：
  - `openspec/config.yaml`
  - `.codex/skills/openspec-*`
  - `%USERPROFILE%\\.codex\\prompts\\opsx-*.md`
- 已新增 `docs/standards/spec-driven-development.md`
- 已新增 canonical spec：`openspec/specs/spec-driven-delivery/spec.md`
- 已回写 `AGENT.md`、`docs/INDEX.md`、任务系统入口、任务板与 Dashboard
- 已明确当前不对现有任务批量接 OpenSpec change，后续在复杂任务开工时再启动

## 验收结果

### A. OpenSpec 初始化

状态：`达成`

- `openspec/` 已建立
- 默认 schema 为 `spec-driven`
- Codex 已具备 core profile 的 4 个 workflow skills 与 4 个 prompts

### B. 任务系统集成

状态：`达成`

- 复杂任务判定规则已写入任务系统入口
- 任务卡目录已新增 OpenSpec 关联字段要求
- Dashboard 与 Task Board 已纳入本次治理变更

### C. 仓库规范收口

状态：`达成`

- `AGENT.md` 已明确复杂开发任务在开发启动前建立 OpenSpec 变更
- `docs/standards/spec-driven-development.md` 已定义本仓库的 spec-first 流程
- 已建立 canonical spec 固化该规则

## 下一步动作
- 后续遇到新的复杂开发任务并准备开始开发时，先从 `/opsx:propose "<change>"` 开始
- 视需要把 OpenSpec workflow 扩展到 `custom` profile，补 `verify / sync / continue / new`

## 当前卡点
- 当前仅启用 `core` profile；如后续复杂任务需要更细粒度的 artifact 推进，再升级 workflow profile

## 断点续跑提示
继续前先看：
- `docs/standards/spec-driven-development.md`
- `openspec/config.yaml`
- `openspec/specs/spec-driven-delivery/spec.md`
- `management/task-system/README.md`
