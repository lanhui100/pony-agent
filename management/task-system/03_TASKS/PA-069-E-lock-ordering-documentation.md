# PA-069-E 锁序规范文档化

## Basic Info
- ID: PA-069-E
- Status: Done
- Priority: P2
- Owner: @agent
- Created At: 2026-06-25
- Updated At: 2026-08-08（实现验证通过并收口）
- Estimated Effort: 1h

## Goal
文档化项目锁序规范，标注关键锁的获取顺序，防止未来引入 ABBA 死锁。

## Output
- `docs/concurrency/lock-ordering.md`（新增）
- 关键路径注释补充

## Acceptance Criteria
1. 锁序规范文档化，明确 runtime → sessions_rwlock 顺序
2. 关键 `Mutex`/`RwLock` 字段添加锁序注释
3. 所有 reviewer 理解并确认锁序

## Current Progress
- 已完成并收口（2026-08-08）。
- `docs/concurrency/lock-ordering.md` 已落地（73 行）：5 级规范锁序（runtime → capability_registry → graph_runs → frontend_diagnostics → sessions_rwlock）、获取规则、已知异常（`load_session_runtime_view` 的 sessions→runtime 顺序，已注明安全原因）、锁清单与中毒策略。
- 锁序验证：`load_session_runtime_view` 的 guard 在 `runtime.lock()` 前已 drop，无重叠持有，文档已记录为已知异常。

## Next Action
- 无。已完成收口。
