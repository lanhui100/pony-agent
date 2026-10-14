# PA-118 进入会话页卡死（graph_runs 锁重入自死锁）修复 — 交付证据归档

- 日期：2026-10-14
- 基线修订：`506220b` ｜ 红相锚定：`533e7b8` ｜ 修复提交：`ba0948e` ｜ 收口提交：`92e198a`
- 修复文件：`crates/pony-agent-core/src/agent/control_plane/history_commands.rs`（+31/−13，无夹带）
- 契约与基准：`.dev-team/contract-matrix-deadlock.md`（v3）、`.dev-team/nfr-baseline.json`
- 门禁守卫：`scripts/check-graph-run-lock-scope.mjs`（含 `--self-test`）、`.dev-team/check-lock-discipline.py`、`.dev-team/nfr-lint.py`（含 `--self-test` / `--mutation-test`）

## 1. 根因

`resolve_graph_run_submission_plan` 在持有 `self.graph_runs`（`std::sync::Mutex`，非重入）时经 `.or_else` 调用
`resolve_graph_run_for_retrieval` → `graph_projection.rs:396 load_active_graph_run_for_session`
→ `graph_projection.rs:383` 二次加锁 → **同线程自死锁**（由 `24c064c4` 引入）。

触发路径：前端进入会话页硬编码 `runId: null`（`src/stores/runtime.ts:1053-1057`）调用**同步** Tauri 命令
`load_session_runtime_view`（主线程执行）→ 主线程永久阻塞 → 桌面窗口冻结、不再重绘。

运行时铁证（gdb 主线程 backtrace）：

```
load_session_runtime_view → resolve_graph_run_submission_plan → Option::or_else
→ resolve_graph_run_for_retrieval → load_active_graph_run_for_session
→ std::sync::Mutex::lock → lock_contended  (futex_wait_queue)
```

## 2. 同口径实机 E2E（生产口径：`--features tauri/custom-protocol` 内嵌 dist + 隔离 HOME 真实 sessions.db + Xvfb/dbus）

| 项 | BEFORE（修复前 `533e7b8`） | AFTER（修复后 `ba0948e`） |
| --- | --- | --- |
| 驱动日志 | [`before-e2e.log`](before-e2e.log) `main_thread_wchan=futex_wait_queue`、`VERDICT=FROZEN_FUTEX`（t=1s..30s 恒定） | [`after-e2e.log`](after-e2e.log) `main_thread_wchan=do_poll.constprop.0`、`VERDICT=RUNNING_OK` |
| 截图 | [`before-shot-30s.png`](before-shot-30s.png) sha256 `a8455855…0bdbf`（启动屏 + DIAG 心跳停在 t=1s） | [`after-shot-30s.png`](after-shot-30s.png) sha256 `b53052fa…a744`（真实会话页：左会话列表 + 中正文 + 右状态栏 + 输入区） |

## 3. 门禁（修复后，干净修订 `ba0948e`，全部 Exit Code 0）

- 红相转绿：`graph_run_submission_plan_deadlock` 13 passed（红相 10 passed/3 failed 超时）；`graph_run_lock_concurrency_red` 2 passed（红相 0/2）
- `scripts/check-graph-run-lock-scope.mjs` GREEN（0 blocking；禁用集合 = 基线 4 ∪ 自动发现 18）
- `.dev-team/check-lock-discipline.py` GREEN（`lock_sites=26 blocking_violations=0`）
- `.dev-team/nfr-lint.py` PASS；`--self-test` 21/21；`--mutation-test` PASS（钝化 python 闸门后必有独占分支用例翻红）
- `cargo check --manifest-path src-tauri/Cargo.toml`、`cargo test --lib control_plane` 66 passed、`rustfmt --check`
- 前端：`vitest` 74 files / 791 passed；`check-version-sync` PASS

## 4. NFR 达标与债务声明

- `external_call_timeout_ms` / `resource_release` / `concurrency_lock`（含 `lock_scope_bounded`、`lock_reentrancy_forbidden`）：达标并机械可验。
- `logging{format=json,trace_id_required}`：**当前无代码级检查点**（全仓库无结构化日志面），本波**不主张已满足**；以 `logging_compliance_claimed=false` 恒真防护 + `logging_debt_ref="PA-119"` 显式登记债务，到期条款见契约 §3.1。
- 已知非阻塞项：P5 fallback 路径 TOCTOU（原子语义在 `24c064c` 之前不可达，非本波回归）；P1 `mod.rs:2059`（经机械核实 `session_id=None` 提前返回，非死锁）。
