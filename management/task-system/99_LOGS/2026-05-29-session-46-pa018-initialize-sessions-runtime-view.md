# 2026-05-29 Session 46 PA-018 Initialize Sessions Runtime View

## 本轮目标

- 继续推进 `PA-018`
- 把 `initializeSessions()` 里的恢复链进一步收拢到统一的 runtime-view 聚合入口
- 减少 `runtime store` 默认路径对原始 `load_execution_checkpoint` 读面的依赖

## 本轮改动

- 更新：
  - `src/stores/runtime.ts`
  - `tests/runtime-store.spec.ts`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`

## 本轮完成

- 新增 `loadSessionRuntimeViewState()` store helper，统一封装：
  - tauri 下的 `load_session_runtime_view`
  - 浏览器 fallback 下的本地派生 runtime view
- `loadSessionState()` 改为消费统一 helper，而不是自己区分 tauri / browser 两套 runtime-view 组装逻辑
- `initializeSessions()` 在空 session / checkpoint 恢复场景下也改走统一 helper
- `runtime-store` 测试已同步改为覆盖：
  - 空 session 初始化保持 idle
  - initialization 恢复 running checkpoint 时由 runtime view 直接带回 checkpoint

## 当前结果

- `runtime store` 在默认会话恢复路径上进一步减少了对原始宿主读面的手工拼装
- `load_execution_checkpoint` 不再是 store 主链默认恢复入口，而是退回到更底层、非主路径的宿主能力
- 这让 host / adapter 邻接层的 retrieval-first 聚合入口更一致，但仍不足以宣布 `PA-018` 完成

## 验证

- `npm exec vitest run tests/runtime-store.spec.ts`
- `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts`
- `npm exec vue-tsc -- --noEmit`

## 下一步动作

1. 继续审 capability / bridge / 其他 UI 面是否还有默认原始读面残留
2. 继续补强 `PA-018` 的 closeout 证据包，而不是过早宣告完成
