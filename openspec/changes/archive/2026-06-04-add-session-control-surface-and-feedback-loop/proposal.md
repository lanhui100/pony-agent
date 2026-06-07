# Proposal: Add Session Control Surface And Feedback Loop

## Why

Pony Agent 当前已经有相当多的 session 控制底座：

- `stop_turn / stop_graph_run`
- `resume_graph_run_stream / continue_graph_run_stream / start_graph_run_stream`
- `checkout_history_node / restore_branch_head / fork_from_history_node / switch_history_branch`
- `submissionPlan / recoveryMode / historyCursorMode`

但这些能力还没有形成对用户清晰可见的交互闭环：

- `checkout / restore / fork / switch` 已有一部分按钮
- `stop / resume / continue / replay` 仍主要停留在 store 和 control-plane 仲裁里
- degrade result、disabled reason、historical_dirty 等状态还缺统一的用户反馈

因此需要单独落一张前端交互实现卡，把“session 可控”从底层能力完成态推进到产品交互完成态。

## What Changes

- 为 stop / resume / continue / replay 建立显式用户入口
- 为 checkout / restore 的 degrade result 建立显式反馈
- 统一历史态、paused、recovery-capable 与 historical_dirty 的前端状态语言
- 补前端验收测试矩阵

## Out Of Scope

- 新的 history graph 或 branch 语义
- 新的 recovery/checkpoint 合同字段
- workspace rollback 真正实现
- hooks / monitor / trace drilldown 扩展

