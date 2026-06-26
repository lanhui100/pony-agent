# PA-073 推进 unify-provider-retry 到 design/spec 阶段

## 基本信息
- 编号: PA-073
- 名称: 推进统一 retry 边界到 design/spec 阶段
- 状态: Done
- 优先级: P2
- 创建日期: 2026-06-27
- 更新日期: 2026-06-27

## 目标
PA-070 在进入实现前，需要把 `unify-provider-retry-and-backoff-boundary` 从 proposal 推进到 design/spec 阶段，并形成可直接指导实现的合同文档。

## 输出
- `openspec/changes/archive/2026-06-27-unify-provider-retry-and-backoff-boundary/design.md`
- `openspec/specs/provider-retry-and-backoff-boundary/spec.md`

## 范围
- 阅读当前 retry 实现代码（`retry.rs`、`provider.rs`、`tools.rs`）
- 阅读 proposal.md 中定义的三层合同（request-level retry、phase-level fallback、turn-level retry）
- 编写 design.md 和 spec.md

## 依赖
- PA-070 的 proposal.md 已就绪
- retry.rs / provider.rs 已有基础实现

## 验收标准
1. design.md 可指导实现者编码
2. spec.md 包含可测试的验收场景
3. PA-070 任务卡更新为实现前可消费状态
4. change 最终已同步到 canonical spec 并归档

## 当前进展
- 已完成 design/spec 编写
- 后续 PA-070 已继续完成实现、审核、canonical spec 同步与 archive 收口

## 断点续跑
- 当前状态: Done
- 下一步: 无；如需继续推进显式 turn-level retry，应以新 change 承接
