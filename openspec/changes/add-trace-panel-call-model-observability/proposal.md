## Why

当前前端 trace 面板已经能展示 `call_model / call_tool / build_context` 的基本步骤，但 `call_model` 仍缺少两类关键观测信号：

- 单次模型调用级别的缓存命中与 TTFT（首 token 延时）没有稳定显示
- 单次模型调用的真实输出没有完整展开，尤其缺少“该次调用究竟产出了工具调用还是消息输出”的保真视图

这会让 trace 面板在最需要排查 hop 级行为时退化成“整轮摘要”，无法支撑运行时可观测性与故障复盘。

## What Changes

- 为前端 trace 面板新增 `call_model` 级别的可观测性合同，明确单次模型调用必须展示缓存命中、TTFT、耗时与输出
- 将 `call_model` 的输出视为“本次模型调用实际发出的内容”，包括工具调用输出与消息输出，而不是只回填最终 assistant 文本
- 明确多 hop 链路中的归因规则：前一个 `call_model` 不得错误吞并后续 hop 的输出，工具调用必须能追溯到触发它的模型调用
- 补齐前端高层联动测试与回归验证，把上述行为纳入正式验收
- 将本次工作挂到 `PA-030` 任务卡与任务系统，确保 spec、实现、测试、日志同步推进

## Capabilities

### New Capabilities
- `trace-panel-call-model-observability`: 规定 trace 面板中单次 `call_model` 的指标展示、输出保真与多 hop 归因行为

### Modified Capabilities
- 无

## Impact

- 前端 trace 展示：`src/components/HomeSidebar.vue`
- 前端运行时拼装与保真：`src/stores/runtime.ts`
- 共享类型：`src/types/runtime.ts`
- 前端高层联动测试：`tests/HomeSidebar.spec.ts`、`tests/runtime-store.spec.ts`
- 任务系统与审计：`management/task-system/`
