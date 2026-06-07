# Tasks: Add Terminal Trace Envelope And Monitor Truth Source

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-036` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-terminal-trace-envelope-and-monitor-truth-source` 的 proposal / design / spec 文档

## 2. Runtime and Persistence Alignment

- [x] 2.1 为 sync terminal persisted trace 补齐 canonical terminal envelope
- [x] 2.2 为 sync failed 与 streamed cancelled persisted trace 补齐 terminal envelope，并验证不会丢失既有 evidence 字段
- [x] 2.3 保持 session roundtrip / reload 对 terminal envelope 的兼容与保真

## 3. Read-Plane Verification

- [x] 3.1 为 monitor / control-plane 补 mixed sync/stream terminal truth-source 测试
- [x] 3.2 为 failed / cancelled session drilldown 补既有 provider/tool/hook evidence 保真回归
- [x] 3.3 为前端 runtime-store / monitor read-plane 补 terminal envelope 消费回归
- [x] 3.4 回写任务卡、review 文档、日志与验收证据
