# PA-030 补强 trace 面板的 call model 可观测性

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- [add-trace-panel-call-model-observability](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-trace-panel-call-model-observability)

## Delta Spec
- [trace-panel-call-model-observability/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-trace-panel-call-model-observability/specs/trace-panel-call-model-observability/spec.md)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `completed`

## 目标
让前端 trace 面板中的 `call_model` 从“整轮摘要入口”升级为“单次模型调用的真实观测面”，补齐缓存命中、TTFT 和输出保真展示，使其能作为运行时问题排查与行为审计的一线证据。

## 输出
- `call_model` 的 cache hit / TTFT / 输出 token / 耗时展示收口
- `call_model` 的真实输出展示，覆盖工具调用输出与消息输出
- 多 hop 归因规则固化，避免前一个 hop 误吞后一个 hop 的回答
- 前端高层联动测试与回归验证闭环
- 任务系统、spec、实现、日志同步更新

## 验收标准
- 每个 `call_model` 节点优先展示该 hop 自己的指标；只有最后一个 hop 才允许回退整轮聚合值
- `provider_stream` 才展示 TTFT；buffered response 不得伪造首 token 延时
- 当某个 `call_model` 产出的是工具调用时，该节点明细必须展示工具调用输出，而不是只留给 `call_tool` 节点
- 当某个 `call_model` 产出的是消息时，该节点必须展示该 hop 自己的消息文本，不得回填别的 hop 的最终回答
- `call_tool` 节点仍保留独立执行明细；`call_model` 中的工具输出展示只做归因补完，不替代 `call_tool`
- 相关前端测试全部通过，且测试用例覆盖 cache hit、TTFT、tool-followup、多 hop 输出归因与 persistence/hydration 保真

## 当前进展
- 已建立 OpenSpec change：`add-trace-panel-call-model-observability`
- 已补 proposal / design / delta spec / tasks，并完成任务勾选
- `HomeSidebar` 已新增 `call_model -> call_tool` 相邻归因逻辑，可在 `call_model` 明细下保真展示工具调用输出
- 已补 `HomeSidebar.spec.ts` 与 `runtime-store.spec.ts` 的高层联动断言，并跑通完整前端测试集与生产构建

## 下一步动作
- 回到 `PA-021`，继续推进 skills registry / bridge 主线

## 当前卡点
- 无

## 断点续跑提示
继续前先看：
- `src/components/HomeSidebar.vue`
- `src/stores/runtime.ts`
- `tests/HomeSidebar.spec.ts`
- `tests/runtime-store.spec.ts`

## 完成情况
- `src/components/HomeSidebar.vue`
  已新增 `call_model` 工具输出归因与预览逻辑，支持在单次模型调用明细中展示工具调用输出与消息输出。
- `tests/HomeSidebar.spec.ts`
  已覆盖多 hop 时“前一个 `call_model` 只展示本 hop 的工具输出、不回填后续最终回答”的行为。
- `tests/runtime-store.spec.ts`
  已覆盖后端提供 trace timeline 时的 tool-followup 顺序、cache hit 与最终 hop 指标保真。

## 验证
- `npm run test:unit`
- `npm run build`

## 验收审计
- [2026-06-03-pa030-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-03-pa030-acceptance-audit.md)
