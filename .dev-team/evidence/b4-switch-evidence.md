# task-5（B4）物理收据：全局 Agent Trace 上报开关 + 隐私告知

日期：2026-10-24 · Executor B（executor-desktop）

## 1. 机器门禁（真实 exit code）

| 命令 | 结果 | exit code |
| :--- | :--- | :--- |
| `npx vitest run tests/otel-reporting-switch.spec.ts` | `Test Files 1 passed (1)` / `Tests 4 passed (4)` | **0** |
| `npx vitest run` | `Test Files 93 passed (93)` / `Tests 985 passed \| 10 skipped (995)` | **0** |
| `npx vue-tsc --noEmit` | 无输出（0 diagnostics） | **0** |

基线对照：改动前 `npx vitest run` 仅 `tests/otel-reporting-switch.spec.ts` 4 条红（其余 92 文件绿）。

## 2. 真实浏览器 E2E（agent-browser，vite dev `127.0.0.1:5199`）

- 打开应用 → 左栏「设置」→ 配置页「通用」tab。
- 观测点 1：`[data-testid="config-agent-trace-reporting-switch"]` 存在，`role="switch"`、`aria-checked="false"`（**默认关闭**）→ 截图 `b4-switch-1-default-off.png`
- 观测点 2：`[data-testid="config-agent-trace-privacy-notice"]` 存在，实测文本：
  > 默认关闭。开启后，本机每个 Agent 回合的运行链路会被收集并上传至 Langfuse 诊断平台，内容包括模型调用与工具调用的名称、耗时与 token 用量；密钥、鉴权头等敏感字段在本地脱敏后再上报，不会以明文离开本机。仅用于定位模型/工具链路问题。可随时关闭，关闭后不再产生任何上报。
- 观测点 3：点击开关 → `aria-checked="true"`，保存通道被触发（浏览器预览态回显「当前是浏览器预览模式，设置仅保留在当前页面会话中。」，非 Tauri 环境不落盘，属既有语义）；`config-agent-trace-save-error` 未出现 → 截图 `b4-switch-2-enabled.png`
- 观测点 4：观测页 `[data-testid="telemetry-upload-independent-note"]` 存在，文本含「全局开关」；上传按钮 `aria-label="上传本会话 Trace 至 Langfuse"`；`telemetry-page` 子树 `innerHTML` 不含 `PonySentry` → 截图 `b4-switch-3-telemetry-note.png`
- Console：`agent-browser console --clear` 后完成全部交互，`console --json` 统计 `total: 6 errors: 0`。

> 说明：浏览器预览态（`isTauriAvailable() === false`）**不落盘**，故「开启后持久化」的物理凭证由 `tests/otel-reporting-switch.spec.ts` UI-1 第二条断言给出（真实 `save_app_settings` invoke 入参 `agentTraceReporting: true`），E2E 只证明真实 DOM 可见性与零 Console Error。

## 3. 改动清单

- `src/types/settings.ts`：`AppSettings.agentTraceReporting: boolean`
- `src/stores/settings.ts`：`DEFAULT_SETTINGS.agentTraceReporting = false`；`normalizeAppSettings` 缺失回落 `false`；新增 `setAgentTraceReporting`（失败回滚末态）
- `src/components/config/ConfigGeneralSection.vue`：「诊断与遥测」卡片 + 开关 + 隐私告示 + 保存失败 error 反馈
- `src/components/telemetry/TelemetryPage.vue`：上传 tooltip/aria-label/toast 品牌改 Langfuse + 「与全局开关互不影响」提示条

未改动：`tests/**`、Rust 代码、`src/lib/telemetry.ts`。
