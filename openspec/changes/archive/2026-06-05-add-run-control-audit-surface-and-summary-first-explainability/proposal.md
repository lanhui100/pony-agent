## Why

`PA-038` 已把 `submission_plan / wait_user / stop_requested / run_resume` 的 boundary evidence 和最小前端消费做出来，`PA-037` 也已经把 session control 的显式入口和基础反馈面做出来。但当前系统仍然缺一层更稳定的 `Run Control audit surface v1`，导致：

- 前端仍需要组合 `submission plan / execution checkpoint / control boundary evidence / local fallback` 多份局部字段，才能解释一次 `stop / continue / resume / replay` 到底发生了什么
- 用户可以点击运行控制动作，但很难稳定验证“命中了哪条 boundary、为什么是 resume 不是 continue、为什么必须 replay”
- reload 后虽然底层 evidence 已存在，但前端缺少更直接的 run-control summary read model，解释体验仍然脆弱

本次 change 的关键约束是：

- `PA-043 v1` 只覆盖 `stop / continue / resume / replay(start)` 的 summary contract
- summary 必须来自后端真相源，而不是前端私有拼装
- summary 只能是审计读面，不得反向成为 `submission plan / checkpoint / graph phase` 的仲裁输入
- 前端范围只做 `summary-first consumption + explainability`，不重做 `PA-037` 已成立的交互信息架构

## What Changes

- 为 run-control 新增统一的 audit summary contract
- 基于既有 persisted evidence / submission plan / checkpoint 真相源收口 run-control summary 的最小投影
- 在 `session snapshot / runtime view / run-control response` 暴露同口径 summary
- 为 `start_graph_run_stream` 在 run-control 语义下补充可判定的 `start_reason` / replay-start 区分，明确排除普通首轮启动
- 把前端 run-control explainability 展示从“兼容字段链 + 私有解释”切到“summary 优先消费”
- 补 summary 的 file-backed reload、control-plane projection 与 UI guard 验收

## Impact

- 用户能更清楚地理解一次 `stop / continue / resume / replay` 的结果
- 开发者能更稳定地调试 replay_required、paused run、checkpoint arbitration 和 blocked run-control 相关问题
- replay/restart 与普通首轮启动的语义边界将更可验证，避免 `start_graph_run_stream` 被双重解释
- 后续如果继续扩展 `Session Control Plane` 的 monitor/drilldown 或企业化审计，不需要重新发明新的前端解释链
