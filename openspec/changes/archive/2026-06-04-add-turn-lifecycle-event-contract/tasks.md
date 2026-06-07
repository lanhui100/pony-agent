# Tasks: Add Turn Lifecycle Event Contract

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-031` 任务卡并同步 dashboard / task board 状态
- [x] 1.2 完成 `add-turn-lifecycle-event-contract` 的 proposal / design / spec 文档
- [x] 1.3 产出 turn lifecycle 架构母文档并回填 docs 索引

## 2. Contract Definition

- [x] 2.1 定义 canonical turn phase 状态机
- [x] 2.2 定义 canonical 一级 turn events 与最小 payload 字段
- [x] 2.3 明确 model hop / tool hop / terminal state 在 lifecycle 中的表达方式

## 3. Implementation and Verification

- [x] 3.1 在 Rust 与 TS 共享 contract 层落第一版类型/枚举收口
- [x] 3.2 为 runtime/stream/control-plane 增补定向测试
- [x] 3.3 回写任务卡、日志与验收证据
