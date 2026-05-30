# 2026-05-29 Session 44 PA-018 Frontend Run Panels Drop Inspect Host

## 本轮目标

- 继续推进 `PA-018`
- 处理前端生产代码里最后残留的 `inspect_host` 默认读面
- 让 `HomeSidebar` / `HomeWorkspace` 的 run 面板在 retrieval 不足时保持空视图，而不是回退原始 host inspection
- 同步回写任务文档与验证证据

## 本轮改动

- 更新：
  - `src/components/HomeSidebar.vue`
  - `src/components/HomeWorkspace.vue`
  - `tests/HomeSidebar.spec.ts`
  - `tests/HomeWorkspace.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `HomeSidebar.refreshRuns()` 现在会：
  - 先尝试内存里的 `retrievedContext.runState`
  - 不足时刷新宿主原生 `load_retrieved_context`
  - 如果刷新后的 retrieval 仍没有活跃 run，则保持空 run 视图
- `HomeWorkspace.refreshGraphRuns()` 现在也会走同样模式：
  - 先用 `retrievedContext.runState`
  - 不足时刷新宿主原生 `load_retrieved_context`
  - 如果 retrieval 仍没有活跃 run，则保持空 graph run 视图
- 这意味着：
  - 前端生产代码里的默认 run 读取链已经不再直接调用 `inspect_host`
  - 原始 host inspection 不再是前端 run 面板的兜底事实源

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts
npm exec vue-tsc -- --noEmit
```

结果：

- `HomeSidebar.spec.ts` 共 `9` 条通过
- `HomeWorkspace.spec.ts` 共 `14` 条通过
- `runtime-store.spec.ts` 共 `28` 条通过
- 总计 `51` 条前端定向测试通过
- `vue-tsc` 通过
- `rg -n "inspect_host" src` 无结果，证明前端生产代码里已不再直接调用 `inspect_host`

## 当前结果

- `PA-018` 在 `C. runtime 接入` 上又补了一条更强的收口证据：
  - 默认提交主链已不再依赖 `inspect_host`
  - 默认 run 面板读取链也已不再依赖 `inspect_host`
- 当前仍不能宣布 `PA-018` 完成交付，因为：
  - capability / bridge 层还没有系统性收口到 retrieval 首选读面
  - 更广范围的 Rust / adapter / host 邻接入口还需要继续核对

## 下一步动作

1. 继续找出 capability / bridge / adapter 层里仍默认读取原始 `session/run/checkpoint` 的公共入口
2. 继续扩展 `LongTermMemory` 的其他显式、保守、可审计稳定事实来源
3. 接近收口时，再做一轮正式 closeout audit，判断是否足以把 `PA-018` 从 `In Progress` 切到 `Done`

## 当前卡点

- 前端生产代码已基本清掉 `inspect_host` 默认读面，但 capability / bridge 层的 retrieval-first 证据还不够完整
- 现有验证更强了，但还不足以支撑最终关单
