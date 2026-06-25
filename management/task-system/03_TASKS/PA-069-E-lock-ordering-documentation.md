# PA-069-E 锁序规范文档化

## Basic Info
- ID: PA-069-E
- Status: Ready
- Priority: P2
- Owner: @agent
- Created At: 2026-06-25
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
- 待开始

## Next Action
- 分析所有锁获取路径，确定规范锁序
