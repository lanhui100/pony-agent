# PA-064 Reduce Session Switch And Init Host Read

## Status
- In Progress (spec reviewed and tuned)

## Priority
- High

## Owner
- OpenAI Codex

## OpenSpec Change
- `openspec/changes/reduce-session-switch-and-init-host-read/`

## Canonical Spec
- `openspec/changes/reduce-session-switch-and-init-host-read/specs/session-control-surface-and-feedback-loop/spec.md`

## Spec 状态
- spec reviewed and tuned

## Goal
- 降低多会话场景下切换历史对话和启动恢复时的宿主读取压力
- 保证会话切换本地优先、不卡前台交互
- 消除 session sidebar duplicate key / 多项选中问题

## Current Findings
- `switchSession()` 前台切换本身通常在几十毫秒内完成
- 真正的卡顿来源于自动 `load_session_runtime_view` / `list_sessions`
- `initializeSessions()` 仍会在启动与热重载时触发重量级 host read
- `HomeWorkspace` 首帧 transcript / markdown 挂载会放大 long-task

## Completed So Far
- 缓存会话切换不再自动触发 `load_session_runtime_view`
- 新建会话不再同步 `list_sessions`
- `HomeWorkspace` 引入 staged hydration
- `HomeSessionSidebar` 渲染前按 `conversationId` 去重
- `createSession()` 改为 collision-safe session id + sessionList 去重
- Spec 经 3 维度并行审核后调优：新增缓存校验、错误恢复、后台 turn 持久化、超时、失效会话处理、localStorage 降级

## Remaining Work
- `initializeSessions()` cache-first with checkpoint insufficiency predicate
- Cache validation and corrupted-state fallback
- localStorage unavailability graceful degradation
- Host read timeout (15s) with error + retry
- Stale/deleted session reconciliation in sidebar
- Running turn state persistence across restart
- Concurrent rapid-switch test + other edge-case tests
- Full regression suite for runtime store and workspace

## 后续任务映射
- `PA-065` 拆分 runtime ownership 并解除 turn 与读路径互锁
- `PA-066` 异步化 provider IO 与 streaming 边界
- `PA-067` 收口 blocking 工作并建立统一执行 helper
- `PA-068` 接入 per-session async turn task 模型

## Validation
- `npx vue-tsc --noEmit`
- `./node_modules/.bin/vitest.cmd run tests/runtime-store.spec.ts`
- `./node_modules/.bin/vitest.cmd run tests/HomeWorkspace.spec.ts`
- `./node_modules/.bin/vitest.cmd run tests/HomeSessionSidebar.spec.ts`
