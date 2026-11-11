# Session 73 - PA-030 trace call model observability

## 本次目标
- 将 trace 面板 `call_model` 的 cache hit / TTFT / 输出保真补齐
- 建立任务卡、OpenSpec change、验收标准与审计记录
- 跑通前端相关测试与构建

## 本次完成
- 新建 `PA-030` 任务卡并同步 `00_DASHBOARD.md`、`01_TASK_BOARD.md`
- 新建 OpenSpec change `add-trace-panel-call-model-observability`
- 补齐 `proposal.md`、`design.md`、`tasks.md`、delta spec
- 在 `HomeSidebar` 增加 `call_model -> call_tool` 相邻归因逻辑
- 补充 `HomeSidebar.spec.ts` 与 `runtime-store.spec.ts` 断言
- 跑通 `npm run test:unit` 与 `npm run build`
- 新增 `2026-06-03-pa030-acceptance-audit.md`

## 修改文件
- `src/components/HomeSidebar.vue`
- `tests/HomeSidebar.spec.ts`
- `tests/runtime-store.spec.ts`
- `openspec/changes/add-trace-panel-call-model-observability/`
- `management/task-system/00_DASHBOARD.md`
- `management/task-system/01_TASK_BOARD.md`
- `management/task-system/03_TASKS/PA-030-strengthen-trace-panel-call-model-observability.md`
- `management/task-system/02_REVIEWS/2026-06-03-pa030-acceptance-audit.md`

## 验证结果
- `npm run test:unit`：通过，`11` 个测试文件、`114` 个测试全部通过
- `npm run build`：通过

## 下一步动作
- 回到 `PA-021`，继续推进 skills registry / bridge 主线

## 断点续跑提示
- 若继续 trace 展示深化，优先以 `PA-030` 的 delta spec 和验收审计为边界，不要直接改写 `PA-024/029` 的既有完成态定义
