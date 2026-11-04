# wave-64 真实浏览器门禁收据（Tier-1，Chromium + vite preview）

契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §5。

## 执行命令与机器收据

```
$ npm run build
EXIT=0

$ npx playwright test tests/e2e/wave64-terminal-status.spec.ts --reporter=list --retries=0
✓ 1 [chromium] › tests/e2e/wave64-terminal-status.spec.ts:31:1 › wave-64 F1/F2 停止后仅由最后一条状态消息承载（无额外中止消息、无绿系配色） (3.9s)
1 passed (5.6s)
EXIT=0
```

覆盖断言（真实 DOM + 真实事件流，非 mock 组件树）：
1. 停止后 `[data-testid="agent-runtime-status-bar"]` 的 `data-mode === "stopped"`，label 文案「已停止」。
2. `[data-testid="agent-runtime-status-icon"][data-mode="stopped"]` 可见（lucide Pause 语义化暂停图标）。
3. 状态条 outerHTML 不含 `emerald|text-green-|bg-green-|lime-|teal-`（绿系配色零出现）。
4. 消息区不含「本轮已停止。」「你已停止本轮 · 发送新消息可继续」「用户终止，发送消息可继续。」（无额外中止消息）。
5. 浏览器控制台 `console.error` 与 `pageerror` 收集数组为空 ⇒ **Console Error = 0**。
6. 证据截图：`docs/release/evidence/wave-64/stopped-terminal-status.png`（1280×720，全页，实测呈「⏸ 已停止」stone 中性色）。

## 备注

- 首轮两次红相均为**测试时序缺陷**（非产品缺陷）：预览流式仅约 440ms，Playwright click 可操作性往返 + 错误的按钮 testid（`workspace-submit-action` → 停止态切换为 `workspace-stop-turn`）导致点击落在流式结束之后/未命中。改为 `waitForFunction` 等待 `workspace-stop-turn` 出现后 DOM click，即稳定通过；未放宽任何产品断言。
- 该 spec 属**证据类**产物（被 §5 门禁条目引用），按 Phase 4 三分法入仓。
