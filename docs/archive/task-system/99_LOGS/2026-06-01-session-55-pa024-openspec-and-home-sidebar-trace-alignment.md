# 2026-06-01 Session 55 - PA-024 OpenSpec 初始化与 HomeSidebar trace 对齐

## 本次完成
- 修复 `HomeSidebar` timeline 语义和测试之间的偏差。
- 新增 `PA-024` 的 OpenSpec 变更文档。
- 同步 `PA-024` 任务卡到 spec-first `Ready` 状态。

## 代码与测试
- `src/components/HomeSidebar.vue`
  - 保持 timeline 结构不回退。
  - `return` step 在 assistant `pending` 时不提前渲染 reasoning / 最终回复。
  - `return` 详情允许从 assistant message 回退取内容，避免要求 timeline entry 重复持有同一份文案。
  - 抑制 `return` step 折叠头里的最终回复预览，避免和详情重复。
- `tests/HomeSidebar.spec.ts`
  - 全部切到 `traceTimeline` 风格的 selector 和 fixture。
  - `assistant pending` 用例改成真正覆盖 timeline 下的保护逻辑。

## OpenSpec
- 新增目录：
  - [add-model-monitor-telemetry-observability](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-model-monitor-telemetry-observability)
- 新增文档：
  - [proposal.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-model-monitor-telemetry-observability/proposal.md)
  - [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-model-monitor-telemetry-observability/design.md)
  - [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-model-monitor-telemetry-observability/tasks.md)
  - [spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-model-monitor-telemetry-observability/specs/model-monitor-telemetry/spec.md)

## 验证
- `cmd /c npm run test:unit -- tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/runtime-store.spec.ts`
- 结果：`61 passed`

## 当前结论
- `PA-024` 现在已经具备正式 OpenSpec 入口，可以按 spec 继续拆执行任务。
- 当前仍缺一次 `npm run build` 的沙箱外验证；之前在沙箱内会被 `vite/esbuild` 读取配置权限限制拦住。
