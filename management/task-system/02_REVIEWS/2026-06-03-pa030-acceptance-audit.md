# PA-030 验收审计

## 任务
- `PA-030` 补强 trace 面板的 call model 可观测性

## 审计范围
- `src/components/HomeSidebar.vue`
- `tests/HomeSidebar.spec.ts`
- `tests/runtime-store.spec.ts`
- `openspec/changes/add-trace-panel-call-model-observability/`

## 验收结论
- 结论：`Pass`

## 验收标准核对

1. `call_model` 指标展示是否补齐 cache hit / TTFT / 耗时 / 输出 token
   - 结果：通过
   - 证据：`HomeSidebar` 仍以单 hop metrics 优先，测试覆盖 `cacheHitInputTokens`、`firstTokenLatencyMs`、buffered latency 不伪造 TTFT。

2. `call_model` 是否能如实展示工具调用输出与消息输出
   - 结果：通过
   - 证据：`HomeSidebar` 已从相邻 `call_tool` 节点派生工具调用输出区块；`HomeSidebar.spec.ts` 已覆盖工具调用输出与最终消息输出分离展示。

3. 多 hop 时是否阻止前一个 `call_model` 错误吞并后一个 hop 的最终回答
   - 结果：通过
   - 证据：`多 hop 时前一个 CALL MODEL 只展示该 hop 自身输出，不回退最终 assistant 回答` 用例通过。

4. store / timeline 保真是否未被这次改动破坏
   - 结果：通过
   - 证据：`runtime-store.spec.ts` 已验证后端提供 trace timeline 时，`call_model -> call_tool -> call_model` 顺序和最后 hop 的 cache hit/耗时指标仍被正确保留。

5. 前端端到端代理验收是否通过
   - 结果：通过
   - 证据：
     - `npm run test:unit`
     - `npm run build`

## 备注
- 当前仓库没有独立 Playwright/Cypress 级浏览器 E2E 基建，因此本次以前端高层组件/store 联动测试作为正式端到端代理验收。
