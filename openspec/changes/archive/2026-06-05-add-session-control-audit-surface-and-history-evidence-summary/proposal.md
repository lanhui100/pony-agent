## Why

`PA-037` 已把 session control 的显式操作入口和第一轮反馈面做出来，`PA-041` 也已经把 history-state evidence 落入了 persisted/read-plane 基线。但当前系统仍然缺一层更稳定的 `Session Control Plane audit surface v1`，导致：

- 前端需要继续组合多个局部字段，才能解释一次 control action 到底发生了什么
- 用户可以点击控制动作，但很难稳定验证“命中了哪条 boundary、是否 degraded、工作区是否真的恢复”
- reload 后虽然底层 evidence 已存在，但前端缺少更直接的 summary read model，解释体验仍然脆弱

本次 change 的关键约束是：

- `PA-042 v1` 只覆盖 `history checkout / restore / fork / switch` 的 summary contract
- summary 必须来自后端真相源，而不是前端私有拼装
- summary 只能是审计读面，不得反向成为 `restore / cursor / branch / rollback` 的仲裁输入
- hooks 的可拓展性继续建立在 canonical boundary 上，而不是让 UI 和 runtime 各自猜测状态

## What Changes

- 定义 `Session Control Plane` 的正式术语与架构基线
- 为 history-control 新增统一的 audit summary contract
- 基于既有 persisted evidence 收口 history-state summary 的最小投影
- 在 `session snapshot / runtime view / history-control response` 暴露同口径 summary
- 把前端 history explainability 展示从“兼容字段链 + 私有解释”切到“summary 优先消费”
- 补 summary 的 file-backed reload、control-plane projection 与 UI guard 验收

## Impact

- 用户能更清楚地理解一次控制动作的结果
- 开发者能更稳定地调试 reload、degraded restore 和 hook guard 相关问题
- 后续若继续扩展 control-plane hooks、审批型 guardrail 或企业化审计，不需要重新发明新的前端解释链
