# Tasks: Add Trace Panel Call Model Observability

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-030` 任务卡并同步 dashboard / task board 状态
- [x] 1.2 完成 `add-trace-panel-call-model-observability` 的 proposal / design / spec 文档

## 2. Trace Panel Implementation

- [x] 2.1 收口 `call_model` 节点的 cache hit、TTFT、耗时与输出 token 展示逻辑
- [x] 2.2 为 `call_model` 明细补齐“工具调用输出”和“消息输出”的保真展示
- [x] 2.3 固化多 hop 归因规则，禁止前一个 `call_model` 错误吞并后续 hop 输出

## 3. Verification

- [x] 3.1 补充 `HomeSidebar` / `runtime-store` 的高层联动测试覆盖上述行为
- [x] 3.2 运行前端相关测试集并修复回归，直到全部通过
- [x] 3.3 回写任务卡、会话日志与验收证据
