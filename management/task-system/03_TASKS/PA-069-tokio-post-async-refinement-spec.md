# PA-069 Tokio 异步后优化治理 — Spec

## 基本信息
- 编号: PA-069
- 名称: Tokio 异步后优化治理
- 状态: Spec Review
- 优先级: P1-P2
- 创建日期: 2026-06-25
- 更新日期: 2026-06-25（采纳 3 路审核意见）

## 背景
PA-065~068 已完成 Tokio 异步重构四卡。在审核验收过程中发现仍有 5 个 P1-P2 级别的优化点未被覆盖。3 路并行智能体审核已完成，以下为采纳的汇总意见。

## 采纳的审核意见

### 来自审核员1（异步架构）
- [x] PA-069-A: `spawn_blocking` 不能解决锁内问题 — 锁仍然被持有，需重构 turn 循环在执行工具前放弃锁
- [x] PA-069-A: 拆分为两阶段 — A-1 重构锁范围释放锁 + A-2 真正的异步文件 IO
- [x] PA-069-D: 补充 WAL checkpoint 管理，防止 WAL 无限增长
- [x] PA-069-C: 65+ 调用点，需分类处理，runtime 锁不应容错

### 来自审核员2（并发安全）
- [x] PA-069-B 新增: `apply_skill_source_snapshot` 存在 TOCTOU 竞争 — 两次获取 `capability_registry` 锁之间快照可能已变
- [x] PA-069-C: 不要盲目 grep 替换所有 expect — 分类：runtime 不可容错，其他可容错
- [x] 新增问题: `load_session_snapshot_at` 总是 `write()` 而非 `read()` — 这是真正的瓶颈
- [x] 新增问题: `save_to_backend` 的 9 个调用点需逐个分析是否可降级为增量写

### 来自审核员3（可操作性）
- [x] PA-069-A 估算不切实际: 2h→4-6h
- [x] PA-069-C 范围被低估: 65→120-130 个调用点
- [x] 执行顺序: B→C→D→A→E (B 和 D 可并行)
- [x] `load_session_runtime_view` 违反锁序 sessions→runtime，需要修复

## 任务拆分（更新版）

### PA-069-B: capability_registry Mutex → RwLock (P2) [实际 1h]
- **文件**: `control_plane.rs`
- **问题**: `capability_registry: Mutex<CapabilityRegistry>` 读多写少
- **目标**: 改为 `RwLock`，读互斥消除
- **额外**: 修复 `apply_skill_source_snapshot` 的 TOCTOU 竞争 — 重入后重新验证

### PA-069-C: Mutex 中毒容错修复 (P2) [实际 3-4h]
- **范围**: ~120 个 `.expect("poisoned")` 调用点（control_plane.rs + runtime/mod.rs + execution_control.rs + turn_flow.rs）
- **分类策略**:
  | 锁 | 策略 | 原因 |
  |---|---|---|
  | `runtime` | 保留 `.expect()` | 复杂状态机，中毒后继续更危险 |
  | `capability_registry` | 容错 | 可下次快照时重新加载 |
  | `graph_runs` | 容错 + log | 可重建运行缓存 |
  | `execution_control` | 容错 | 简单状态，恢复安全 |
  | `terminal` | 容错 | 遥测非关键路径 |
  | `sessions_rwlock` | 容错 | 数据可从 SQLite 恢复 |

### PA-069-D: SQLite 写路径优化 (P2) [实际 2h]
- **文件**: `sqlite_session.rs`, `session.rs`
- **问题**: `save_to_backend` → `write_full_store` 全量序列化
- **目标**: 确保热路径优先增量写（`upsert_session`），全量写仅在删除/结构变更时触发
- **补充**: WAL checkpoint 管理 — N 次增量写后触发 `wal_checkpoint(PASSIVE)`
- **注意**: 需验证 `save_to_backend` 的 9 个调用点，确保热路径不走全量写

### PA-069-A: 工具文件 IO 从 runtime 锁内移出 (P1) [实际 4-6h]
- **文件**: `runtime/mod.rs`, `control_plane.rs`, `tools.rs`
- **问题**: `execute_registered_tool_call` 在 `runtime.lock()` 持有期间执行同步文件 IO
- **核心矛盾**: `spawn_blocking` 和 `tokio::fs` 在持有 `MutexGuard` 时无效
- **方案**: 重构 turn 循环，在工具执行前放弃 `runtime.lock()`，执行后重新获取
- **阶段一**: 将 `tool_executor.execute()` 移出锁范围（预解析需要的状态后 drop 锁）
- **阶段二**: 工具执行使用 `spawn_blocking` 释放线程资源

### PA-069-E: 锁序规范文档化 (P2) [实际 1h]
- **新增**: `docs/concurrency/lock-ordering.md`
- **内容**: 
  ```
  锁序: runtime → capability_registry → graph_runs → sessions_rwlock
  ```
- **修复**: `load_session_runtime_view` 的锁序违反 sessions→runtime
- **注意**: 若 PA-069-A 将 runtime 改为 `tokio::sync::Mutex`，锁序语义变化需同步更新文档

## 执行顺序
```
Phase 1 (可并行、低风险):
  PA-069-B  capability_registry RwLock      [1h]

Phase 2 (依赖 B):
  PA-069-C  Mutex 中毒容错 (关键锁)       [3-4h]

Phase 3 (独立、中风险):
  PA-069-D  SQLite 写路径优化              [2h]

Phase 4 (高风险、需准备):
  PA-069-A  工具文件 IO 移出锁范围         [4-6h]

Phase 5 (依赖全部):
  PA-069-E  锁序文档化                     [1h]
```

## 验收标准
1. PA-069-B: RwLock 替换 + TOCTOU 修复，编译通过，测试通过
2. PA-069-C: 分类容错，runtime 锁保留 expect，其他容错 + log
3. PA-069-D: 热路径增量写优先，WAL checkpoint 管理，不降级崩溃恢复安全性
4. PA-069-A: 工具执行期间 runtime 锁已释放，大文件搜索不阻塞其他操作
5. PA-069-E: 锁序文档化，load_session_runtime_view 锁序修复

## 验证
```bash
npm run cargo:check:shared
npm run cargo:test:shared
npm run cargo:test:regression
```
