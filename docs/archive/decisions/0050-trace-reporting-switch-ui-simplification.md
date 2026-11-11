# ADR 0050：配置页「全局轨迹上报」选项收敛为单行标签 + 右端开关

Status: implemented

## 背景

ADR 0047（Wave43）在配置页「通用 → 诊断与遥测」落地的卡片包含四层内容：开关行 + 隐私告示两段 + 「OTel 导出网络与认证」四个输入 + 「用量治理策略」四项 + 「会话成本预算」一项，同一卡片内堆叠了两类互不相干的关注点（轨迹上报 vs 成本治理）。用户裁决（2026-10-27）：该选项设计过于复杂，收敛为「全局轨迹上报」+ 右端 switch 即可，去除注释与子项；二次裁决：连「诊断与遥测」分区标题与卡片边框/背景色一并去掉，字段直接落在页面上。

## 决策

1. **UI 形态**：`src/components/config/ConfigGeneralSection.vue` 中该项为页面裸字段行——无「诊断与遥测」分区标题、无卡片边框、无背景色；左侧文案「全局轨迹上报」，最右端 `Switch`（`data-testid` 沿用 `config-agent-trace-reporting-switch`，`aria-label="全局轨迹上报"`）；旧称「全局 Agent Trace 上报」不再出现。
2. **删除注释与子项**（用户裁决）：隐私告示、OTel 端点/服务名/采样率/Token 四个输入、用量治理策略四项、会话成本预算输入全部从 UI 移除（对应 `data-testid` 同步消失）。
3. **仅删 UI，不删契约**：`AppSettings.agentTraceReporting` / `otelEndpoint` / `otelServiceName` / `otelSamplingRatio` / `otelToken` / `usageGovernancePolicy` / `costBudgetUsd` 的落盘键与 Rust 侧读取语义全部保持不变（core 的 OTel 解析与用量治理判定照旧），因此旧配置文件与 CLI/TUI 宿主行为不受影响；保存失败回滚与就近错误反馈（`config-agent-trace-save-error`）保留。
4. **文案一致性**：观测页「手动上传与全局开关互不影响」提示同步改称「全局轨迹上报」，避免指向已不存在的旧名。
5. **底部浅色 notice 行移除**（用户裁决 2026-10-27）：`ConfigGeneralSection.vue` 不再渲染 `settingsStore.notice`，页面底部「当前是浏览器预览模式，设置仅保留在当前页面会话中。」等浅色提示消失；store 侧 `notice` 状态与其既有契约保留（其他宿主/单测不变），保存失败的错误行（`settingsStore.error`）与开关回滚反馈（`config-agent-trace-save-error`）保留。
6. **验收契约变更（需显式记录）**：Wave43 冻结用例 `tests/otel-reporting-switch.spec.ts` 原断言「隐私告示三段语义」与本次裁决冲突，按新裁决改为——单行标签 + 默认关闭 + 开关落盘，且隐私告示/OTel/用量治理/成本预算入口的 `data-testid` 一律不存在（反向断言防回归）。该改动属需求方裁决的契约变更，非实现方自行放宽。

## 候选方案

- **保留 Wave43 四层卡片（仅删隐私告示两段）**：仍保留 OTel/治理/预算子块，用户反馈的「复杂」未被解决。放弃。
- **只删 OTel 子块，保留用量治理/成本预算**：用户明确选择「整卡只留一行轨迹开关」，且这两块与轨迹上报无因果关系。放弃。
- **把用量治理/成本预算迁移到独立配置分区**：需要新信息架构与导航决策，超出本次最小闭包；当前版本先撤离 UI 入口，若后续仍需人工调参再单独立 ADR。放弃。
- **改放 `AppSettings` 落盘键 / 删 Rust 侧读取**：会造成配置结构契约破坏且 core 治理判定失去输入，远超 UI 精简范围。放弃。

## 后果

- 用户可见：配置页「通用」的该项只剩一行裸字段「全局轨迹上报 + 右端 switch」（无分区标题/边框/背景色），复杂度显著下降；代价是 OTel 端点/采样率/Token 与用量治理、成本预算不再有桌面 UI 入口（仍可通过配置文件与 env 生效）。
- 代码：`ConfigGeneralSection.vue` 删除 5 个输入处理函数（OTel 三项、成本预算、用量治理）与对应模板；`src/types/settings.ts`、`src/components/telemetry/TelemetryPage.vue`、`docs/architecture/agent-trace-pipeline.md` 同步更名。
- 测试：`tests/otel-reporting-switch.spec.ts` UI-1 首条改判为「单行标签 + 无注释无子项」；`UI-1` 落盘断言与 `UI-2` 手动上传解耦断言不变。
- 证据：`tests/otel-reporting-switch` 单测 Exit 0、`npm run typecheck` 0 diagnostics、vite dev 真实浏览器截图（配置页「通用」tab）与 Console 0 error。
