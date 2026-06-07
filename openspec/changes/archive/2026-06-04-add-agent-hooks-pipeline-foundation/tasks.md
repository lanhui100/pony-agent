# Tasks: Add Agent Hooks Pipeline Foundation

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-033` 任务卡并同步 dashboard / task board 状态
- [x] 1.2 完成 `add-agent-hooks-pipeline-foundation` 的 proposal / design / spec 文档

## 2. Contract Definition

- [x] 2.1 定义 hook 分类、权限边界与允许挂接的 lifecycle boundary
- [x] 2.2 定义 hook 执行顺序、超时、失败语义与恢复语义
- [x] 2.3 定义 hook traceability / audit / persistence 原则

## 3. Implementation and Verification

- [x] 3.1 落第一版 hook registry / executor / result contract 骨架
- [x] 3.2 增补定向合同测试与 runtime 验证
- [x] 3.3 补 hooks structured result normalization（deny / patch / side-effect-request 最小合同）
- [x] 3.4 回写任务卡、日志与验收证据
- [x] 3.5 打通 hook trace records 的 persisted trace roundtrip 通道（Rust + TS 类型/消费层）
- [x] 3.6 定义 TurnHookPoint -> canonical lifecycle/event binding，并补防漂移测试
