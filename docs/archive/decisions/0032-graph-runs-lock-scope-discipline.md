# 0032 graph_runs 非重入锁作用域纪律与桌面端会话页卡死修复

Status: implemented

## 背景

桌面端新版本出现「界面能进、一进入会话页面即卡死」的 P0 故障。运行时铁证（gdb 主线程 backtrace）显示：

```
load_session_runtime_view（同步 Tauri 命令，主线程执行）
  → resolve_graph_run_submission_plan
  → Option::or_else → resolve_graph_run_for_retrieval
  → load_active_graph_run_for_session
  → std::sync::Mutex::lock → lock_contended（futex_wait_queue，永久阻塞）
```

`HostControlPlane::resolve_graph_run_submission_plan`（`history_commands.rs`）在持有 `self.graph_runs`
（`std::sync::Mutex`，**非重入**）时经 `.or_else` 调用 `resolve_graph_run_for_retrieval`，后者在
`graph_projection.rs` 再次获取同一把锁 → 同线程自死锁。该锁作用域由提交 `24c064c4` 引入
（为新增 ask-wait 自愈判定而扩大了 guard 范围）。

触发条件是前端进入会话页的默认入参：`runId: null` + 非空 `sessionId`（`src/stores/runtime.ts`）。
由于 Tauri v2 中非 `async` 命令在主线程执行，主线程一旦死锁，窗口不再重绘，表现为永久卡死。

## 候选方案

### 方案 A：改用可重入互斥锁（`parking_lot::ReentrantMutex` 等）
- **做法**：把 `graph_runs` 换成可重入锁，允许同线程二次加锁。
- **落选原因**：
  1. 用锁类型掩盖调用图缺陷：真正的问题是「guard 生命周期跨越了会再次加锁的调用」，可重入锁会让同类缺陷长期隐形；
  2. 引入新依赖与新锁语义（跨 `await`、poison 策略、`Send`/`Sync` 约束），爆炸半径远超本缺陷；
  3. 可重入锁不解决跨线程 ABBA 与长持锁，只解决同线程重入这一个子集。

### 方案 B：把同步 Tauri 命令改为 `async`（移出主线程）
- **做法**：给 `load_session_runtime_view` 等命令加 `async`，使其在异步运行时执行。
- **落选原因**：
  1. 不解决根本缺陷：自死锁依旧存在，只是把「窗口冻结」变成「该命令永久 pending（界面停在加载态）」；
  2. 仓库有 85 个同步命令，逐个改造属跨切面重构，违反最小闭包；
  3. 主线程阻塞只是该缺陷的表现面之一，控制面内部其它调用点（如 `load_session_snapshot`）同样会踩中。

### 方案 C：删除 fallback（无 `run_id` 时按 session 找活跃 run）
- **做法**：去掉 `resolve_graph_run_for_retrieval` 的 fallback 分支。
- **落选原因**：fallback 是**正确且必要**的语义（前端默认不传 `runId`，需要按 session 归位活跃 run）；删除会改变 C1 契约（有活跃非终态 run 时应返回 `continue_graph_run_stream`）。

### 方案 D：锁作用域收敛 + 命中路径单 guard 原子读 + 机械守卫（选中）
- **做法**：
  1. `run_id = Some(id)` 且 `load_run` 命中时，run 快照与 `has_unresolved_ask_waits` 在**同一 guard 内**完成（保持修复前原子读语义，避免撕裂读）；
  2. 仅「无 `run_id` / 未命中」的 fallback 分支才在**锁外**调用 `resolve_graph_run_for_retrieval`，随后用**另一次独立短锁**判定 ask waits（消除重入；该路径的原子语义在 `24c064c` 之前本就不存在）；
  3. 新增机械守卫 `scripts/check-graph-run-lock-scope.mjs`：括号级扫描 `self.graph_runs.lock()` 作用域内的二次加锁调用，禁用集合 = 基线 4 名 ∪ 自动发现全部 locker，并带「发现数退化即 Exit 2」防线与 `--self-test` 正负样例。
- **选中原因**：单文件最小闭包、不引入依赖、保留全部既有语义（含 `24c064c` 的 ask-wait 自愈），并把「同类缺陷不得再犯」固化为可机械执行的门禁。

## 决定

1. **锁作用域纪律**：任何 `self.graph_runs.lock()` 作用域内，禁止调用会再次加锁的方法
   （`resolve_graph_run_for_retrieval` / `load_graph_run` / `load_execution_checkpoint` /
   `load_active_graph_run_for_session`，以及自动发现出的全部 locker）。
2. **命中路径原子读**：`resolve_graph_run_submission_plan` 的 `run_id` 命中路径必须单 guard 完成
   「run 快照 + ask-wait 判定」；fallback 路径允许「锁外解析 + 独立短锁」。
3. **门禁固化**：`scripts/check-graph-run-lock-scope.mjs` 纳入常规验证；`.dev-team/check-lock-discipline.py`
   作为独立实现的交叉校验。

## 后果

- 桌面端会话页卡死消除：同口径实机 E2E 由 `wchan=futex_wait_queue / VERDICT=FROZEN_FUTEX`
  变为 `wchan=do_poll / VERDICT=RUNNING_OK`，真实会话页正常渲染（证据见
  `docs/release/2026-10-14-pa118-deadlock-fix/`）。
- 新增 `graph_run_submission_plan_deadlock`（13 例）与 `graph_run_lock_concurrency_red`（2 例，8 线程确定性并发）
  两组验收测试，死锁路径用例自带 5s 看门狗，超时即失败而非挂死测试进程。
- 已知非阻塞登记项：`mod.rs:2059`（`stop_graph_run` 持锁调用 `load_execution_checkpoint`，实参
  `session_id=None` 使其在加锁前提前返回，**当前非死锁**，但模式脆弱）；fallback 路径 TOCTOU
  （原子语义在 `24c064c` 之前不可达，非本次回归）。二者记录于
  `.dev-team/contract-matrix-deadlock.md` 的 P1 / P5。
