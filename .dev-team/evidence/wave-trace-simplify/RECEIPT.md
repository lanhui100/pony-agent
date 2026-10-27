# 物理收据：配置页「全局轨迹上报」精简为单行标签 + 右端开关

日期：2026-10-27 · 裁决：用户（只留「全局轨迹上报 + 右端开关」，无分区标题、无边框/背景色）· ADR：[0050](../../../docs/decisions/0050-trace-reporting-switch-ui-simplification.md)

## 1. 机器门禁（真实 exit code）

| 命令 | 结果 | exit |
| :--- | :--- | :--- |
| `npx vitest run tests/otel-reporting-switch.spec.ts` | `Test Files 1 passed` / `Tests 4 passed (4)` | 0 |
| `npx vitest run`（全量） | `Test Files 107 passed` / `Tests 1056 passed \| 12 skipped` | 0 |
| `npx vue-tsc --noEmit`（含 `noUnusedLocals`） | 无输出（0 diagnostics） | 0 |
| `check-declaration.sh --declaration docs/observability.yaml` | `结论: PASS (exit=0)` | 0 |
| `audit-diff.sh` | `[OK] audit-diff: 无越界实现、收口点完整` | 0 |

> 全量套件有一处跨文件 unhandled `persistHistory` 定时器告警（`tests/runtime-store.spec.ts`），单文件复跑 119 例全绿、exit 0，且与本单改动无因果关系（未触碰 runtime store），判定为既有 flake。

## 2. 真实浏览器（agent-browser，vite dev `127.0.0.1:5199`）

- 路径：左栏「设置」→「配置 → 通用」tab。该项为裸字段行：无「诊断与遥测」标题、无边框、无背景色；页面底部无「当前是浏览器预览模式…」浅色 notice（`config-general-section` 内 `<p>` 节点数 = 0）。
- DOM 取证（`config-diagnostics-section`）：innerText = `全局轨迹上报`（无「诊断与遥测」）；class = `max-w-[680px] space-y-2`（无 `border*` / `bg-*`）；section 内 `<input>` 数 = 0；switch `aria-label=全局轨迹上报`、默认 `aria-checked=false`。
- switch：`role=switch`、`aria-label="全局轨迹上报"`、默认 `aria-checked="false"`；点击 → `true` → 再点 → `false`。
- 10 个旧 `data-testid`（隐私告示 / OTel 四项 / 用量治理四项 / 成本预算）全部不存在。
- Console：`--clear` 后完成开-关交互，`total 0 / errorCount 0`（唯一跨页 warning 是既有 `[pony-agent][perf] long-task`）。
- 截图：`config-general-final.png`（三次裁决后：裸字段行 + 无底部浅色 notice）、`config-general-after-v2.png`（二次裁决）、`config-general-after.png`（首轮）、`config-diagnostics-row.png`、verifier 侧 `verifier-config-general-switch-{on,off}.png`。

## 3. 独立验收

verifier（fresh context，只读）：6/6 PASS；2 NOTE = 验收用例按 ADR 0050 契约变更（断言强度净增）、
`AppSettings` 契约字段有意保留（去 UI 不去契约）。

## 4. 改动清单

- `src/components/config/ConfigGeneralSection.vue`：删除 5 个输入处理函数 + 隐私告示/OTel/用量治理/成本预算四块；标签改名「全局轨迹上报」；二次裁决再去掉「诊断与遥测」分区标题与卡片 `border`/`bg-white`（裸字段行）；三次裁决再删底部浅色 notice 行（`settingsStore.notice`）。
- `tests/otel-reporting-switch.spec.ts`：UI-1 首条改判为「单行标签 + 无注释无子项 + 无分区标题/边框/背景色」反向断言。
- `src/components/telemetry/TelemetryPage.vue`、`src/types/settings.ts`、`docs/architecture/agent-trace-pipeline.md`、ADR 0047：措辞事实同步。
- 新增 `docs/decisions/0050-trace-reporting-switch-ui-simplification.md` + 两处索引。
