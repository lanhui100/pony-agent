# Checkpoint Message Controls Spec Review

## 审核对象

- [proposal.md](C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-message-controls-and-bottom-menu/proposal.md)
- [design.md](C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-message-controls-and-bottom-menu/design.md)
- [tasks.md](C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-message-controls-and-bottom-menu/tasks.md)
- [spec.md](C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-message-controls-and-bottom-menu/specs/session-control-surface-and-feedback-loop/spec.md)

## 审核方式

- 使用 `opencode`
- 模型：`anthropic/claude-opus-4-7`
- 方式：只读 spec 审核，不改文件

## 模型结论

- 总体结论：`需修改后通过`

## 高优先级问题

1. 缺少稳定 `turnId <-> nodeId` 映射的显式 spec contract。
2. “清理当前笨重设计”对 sidebar 的迁移范围不够明确。
3. 新增 checkpoint read model 的数据来源边界不够收口，容易让实现阶段误解为需要新 endpoint 或私有拼装。
4. 快捷键方案缺少现有全局快捷键冲突审计与最终固定规则。
5. tasks 缺少 runtime state schema 扩展这个实现前置依赖。

## 采纳与调优

以上 5 条全部采纳，并已完成一轮 spec 调优：

1. 在 spec 中新增 `Stable turn-to-checkpoint mapping` requirement，明确无映射时必须隐藏入口，禁止顺序猜测。
2. 在 design 中补 sidebar 迁移清单，区分保留、弱化和移除项；在 spec 中明确 sidebar 只能作为次级 explainability surface。
3. 在 spec 中新增 `Checkpoint read model SHALL be derived from existing runtime state` requirement，明确允许扩展既有 runtime state schema，但不新增独立 checkpoint command family。
4. 在 tasks 中新增快捷键占用审计与最终组合固定任务，并在 design 中补充快捷键冲突风险说明。
5. 在 tasks 中新增 `Runtime State Schema Extension` 前置章节，要求先补齐 `turnId` 与 rollback capability 相关字段。

## 结果

- 当前版本已从“方向正确但落地边界偏松”收紧为“可以指导实现拆解”的 spec 包
- 后续可直接进入实现或继续做一次更偏工程可测性的二审
