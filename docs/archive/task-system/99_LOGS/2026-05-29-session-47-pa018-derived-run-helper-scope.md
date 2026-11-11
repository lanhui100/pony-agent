# 2026-05-29 Session 47 PA-018 Derived Run Helper Scope

## 本轮目标

- 继续推进 `PA-018`
- 继续减少 production 代码里重复的 retrieval refresh / run 派生逻辑
- 在不引入 UI 回归的前提下，验证哪些入口适合继续抽到统一 helper

## 本轮改动

- 更新：
  - `src/stores/runtime.ts`
  - `src/components/HomeWorkspace.vue`
  - `src/components/HomeSidebar.vue`
  - `tests/HomeSidebar.spec.ts`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`

## 本轮完成

- `runtime store` 新增 `resolveDerivedSessionRun()`，统一封装：
  - 优先尝试内存里的 `retrievedContext.runState`
  - 本地事实不足时，刷新 `loadRetrievedContextState()`
  - 刷新后再推导最小 `GraphRun`
- `submitTurn()` 改为复用该 helper，而不是自己拼一套 retrieval refresh / submission 推断逻辑
- `HomeWorkspace` 也改为先走 store helper，再基于最新 retrieval facts 构造当前 run 视图

## 本轮发现

- `HomeSidebar` 尝试切到同一 helper 时，测试暴露了 run-group / standalone trace 分组回归风险
- 该风险不是 retrieval contract 本身失效，而是侧栏 trace 分组和当前 run 视图之间的耦合更细，贸然抽象会破坏已经稳定的 UI 行为

## 当前结果

- `submitTurn()` 和 `HomeWorkspace` 进一步减少了重复的 retrieval refresh / run 派生逻辑
- `HomeSidebar` 暂时保持原有稳定路径，说明 `PA-018` 目前更适合继续做“保守收敛”
- 这仍然是 `PA-018` 的进行中证据，而不是完成标记

## 验证

- `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts`
- `npm exec vue-tsc -- --noEmit`

## 下一步动作

1. 继续审其他 capability / bridge / UI 面的原始读面残留
2. 对侧栏 trace-run 分组逻辑单独建一轮更小的收敛方案，而不是强行共用当前 helper
