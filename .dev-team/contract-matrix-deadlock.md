# 契约矩阵：进入会话页卡死（graph_runs 非重入锁自死锁）冻结基准

| 项 | 值 |
| --- | --- |
| 契约版本 | `deadlock-v1`（冻结） |
| 冻结时间 | 2026-10-14T05:36Z |
| 仓库修订 | `506220b` (main) |
| 唯一编写方 | protocol-agent（L2-P 协议特派员） |
| 写域 | 仅 `.dev-team/`（未触碰 `src/`、`src-tauri/`、`crates/`、`tests/`、`scripts/`） |
| 关联 NFR | `.dev-team/nfr-baseline.json` / `.dev-team/nfr-baseline.schema.json` |
| 机械门禁 | `.dev-team/nfr-lint.py`、`.dev-team/check-lock-discipline.py` |
| 物理收据 | `.dev-team/task1-receipts.log` |

---

## 0. 根因证据链（已由运行时主线程 backtrace 铁证确认）

```
[前端] src/stores/runtime.ts:1053-1057  payload.runId = null（硬编码）
   │        （loadSessionRuntimeViewState 进入会话页时必发）
   ▼
[Tauri] src-tauri/src/lib.rs:768  fn load_session_runtime_view(...)  ← 同步命令，主线程执行
   ▼
[crates] history_commands.rs:428  resolve_graph_run_submission_plan()
   │        L432  let (run, has_unresolved_ask_waits) = {   ← 锁作用域起始
   │        L433  let store = self.graph_runs.lock()        ← 持有 std::sync::Mutex（非重入）
   │        L441  .or_else(|| self.resolve_graph_run_for_retrieval(None, query.session_id))
   ▼
[crates] graph_projection.rs:396  resolve_graph_run_for_retrieval()
   │        run_id == None → 短路 and_then → 落到 .or_else 分支
   ▼
[crates] graph_projection.rs:376  load_active_graph_run_for_session()
   │        L383  self.graph_runs.lock()                    ← 同线程二次加锁
   ▼
        std::sync::Mutex 非重入 → 同线程自死锁（无 panic、无超时、永久挂起）
```

**触发条件（充分条件，必现）**：`run_id = None` **或** `run_id = 不存在的 run`，且 `session_id.trim()` 非空。
与 checkpoint 是否存在无关（加锁块位于函数首部、先于 checkpoint 查询）；与 session 是否存在活跃 run 无关（`load_active_graph_run_for_session` 在加锁**之前**只对 session_id 做 trim/空判定，随后无条件 `lock()`）。

**语义意图 vs 缺陷**：L441 的 fallback（无 run_id 时按 session 找活跃 run）是**正确且必须保留**的语义；缺陷仅在于该调用发生在锁作用域**内**。修复方向 = 锁作用域收敛（先取快照、释放锁、再做 fallback 与 ask-wait 判定），**不得删除 fallback 语义**。

---

## 1. 契约矩阵（黑盒）

判定术语：
- **有界返回**：单次调用 wall-clock ≤ `deadlock_watchdog_timeout_ms` = **5000 ms**（见 `nfr-baseline.json`）。
- **硬边界**：外部 `timeout -k 5 300` 包裹测试进程；死锁时进程被强杀，退出码 124/137 ≠ 0 → 判 FAIL。
- **测试内断言**：新增有界性用例必须自带 `Instant::now()` 断言 `elapsed < 5000ms`。

### C1 — `run_id=None` 且 `session_id` 非空必须有界返回（禁止自死锁）

| 输入 | 期望输出 | 判定方式（可机械执行） |
| --- | --- | --- |
| `resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery{ run_id: None, session_id: Some("<非空>"), node_id: None })`；session 无活跃 run、无 checkpoint | 有界返回；`command="start_graph_run_stream"`、`run_id=None`、`source="default"` | G1 / G2 / G3（见 §2）；RED 态：进程挂起 → Exit 124 |
| 同上，但 session 存在活跃非终态 run（Ready/WaitingUser/Running）、无 checkpoint | 有界返回；`command="continue_graph_run_stream"`、`run_id=Some(该活跃 run.id)`、`source="graph_run_reconciled"` | G3（新增用例覆盖） |
| 同上，但存在该 session 的 checkpoint | 有界返回；保持既有 checkpoint 协调语义（`source ∈ {"checkpoint","graph_run_reconciled"}`），**行为与修复前一致** | G1（现有用例 `submission_plan_prefers_checkpoint_projection_when_available`） |

### C2 — `run_id=不存在` 且 `session_id` 非空必须有界返回

| 输入 | 期望输出 | 判定方式 |
| --- | --- | --- |
| `run_id: Some("<store 中不存在>")`、`session_id: Some("<非空>")` | 有界返回；退化语义与 C1 完全一致（`store.load_run` 返回 None → fallback 到 session 活跃 run；仍无则 default） | G4（现有用例 `submission_plan_starts_fresh_run_when_recovery_contract_requires_replay`，其 `run-stale` 从未写入 graph_runs store → 命中 C2 路径） |
| `run_id: Some("<不存在>")`、`session_id: None` | 有界返回；不触发 fallback（`load_active_graph_run_for_session` 对 None 直接返回 None），`source="default"` | G4b（新增用例，可选） |

### C3 — 自愈语义保留：存在 unresolved ask waits ⇒ `start_graph_run_stream` + `run_id=None`

| 输入 | 期望输出 | 判定方式 |
| --- | --- | --- |
| 命中 run（无论经 run_id、fallback 还是 checkpoint），该 run 在 `GraphRunStore.ask_waits` 中有非空等待项 | `command="start_graph_run_stream"`、`run_id=None`；`source ∈ {"graph_run_reconciled","checkpoint"}` | G5（现有用例 `submission_plan_switches_with_session_checkpoint_boundary`，含 `graph_bind_ask_wait` 与断言文案 "cannot accept a turn … must self-heal"） |

**语义铁律**：修复不得改变 ask-wait 自愈优先级 —— ask waits 判定必须与锁获取解耦（在同一快照内判定，或释放锁后重取），且 `has_unresolved_ask_waits` 的输入 store 必须与 run 来源同源。

### C4 — 命令与 run 相位映射

| run.phase | 期望 `command` | 期望 `run_id` | 判定方式 |
| --- | --- | --- | --- |
| `Completed` / `Failed` / `Cancelled`（终态） | `start_graph_run_stream` | `None` | G6（现有用例 `lifecycle_boundary_checkpoint_does_not_override_default_submission_plan`、`submission_plan_does_not_resume_when_run_is_not_resumable`） |
| `Ready` | `continue_graph_run_stream` | `Some(run.id)` | G6（现有用例 `submission_plan_falls_back_to_graph_run_when_checkpoint_is_absent`） |
| `WaitingUser` | `continue_graph_run_stream` | `Some(run.id)` | G5（`waiting_plan` 断言） |
| `Running` | `continue_graph_run_stream` | `Some(run.id)` | G5（`running_plan` 断言） |
| `Paused` 且 `active_turn_id == None`（可 resume） | `resume_graph_run_stream` | `Some(run.id)`、`source="graph_run"` | **G7（契约要求新增用例，前缀 `submission_plan_paused`）** |
| `Paused` 且 `active_turn_id != None`（不可 resume、不可 continue） | `start_graph_run_stream` | `None` | G7（同上，新增用例覆盖降级分支） |

> 说明：`resolve_submission_command_against_run`（graph_projection.rs:283）是既有的命令-相位协调器；C4 期望值以**协调后**结果为准（例如 `resume` 在不可 resume 时降级 `continue` 或 `start`）。修复不得改动该协调器的判定表。

### C5 — 锁纪律不变量（作用域收敛）

**不变量**：任何 `self.graph_runs.lock()` 的作用域内，不得出现以下任一调用（它们会再次 `self.graph_runs.lock()`）：

1. `resolve_graph_run_for_retrieval`（graph_projection.rs:396）
2. `load_graph_run`（query_commands.rs:365）
3. `load_execution_checkpoint`（query_commands.rs:6）
4. `load_active_graph_run_for_session`（graph_projection.rs:376，加锁点 L383）

并且同一作用域内不得再次出现 `self.graph_runs.lock()`。

| 分级 | 范围 | 判定 | 当前状态 |
| --- | --- | --- | --- |
| **ERROR（阻塞）** | 死锁路径文件 `history_commands.rs` | `python3 .dev-team/check-lock-discipline.py` → **Exit 0** | **RED**：L441 于 L432 起的作用域内调用 `resolve_graph_run_for_retrieval`（blocking_violations=1） |
| P1（登记，不阻塞） | 其它文件 | 同命令 `WARN(P1)` 输出 | `mod.rs:2059` 于 L2039 起作用域内调用 `load_execution_checkpoint`（`stop_graph_run`，条件性同类隐患） |

**非空证明（防恒真门禁）**：
- 负向对照 `.dev-team/fixtures/lock-discipline/good/` → Exit 0
- 正向对照 `.dev-team/fixtures/lock-discipline/bad/` → Exit 1
- 全模块扫描 10 文件 / 25 处 `self.graph_runs.lock()`，仅报 1 ERROR + 1 P1（无误报）

---

## 2. 机械判定命令清单（验收门禁，冻结）

> 由 Lead / 独立验收方执行；Executor 不得自行裁决验收结果。

| ID | 命令 | 期望 Exit Code |
| --- | --- | --- |
| G1 | `timeout -k 5 300 cargo test -p pony-agent-core --lib submission_plan_prefers_checkpoint_projection_when_available -- --exact` | 0（RED 态：124） |
| G2 | `timeout -k 5 300 cargo test -p pony-agent-core --lib lifecycle_boundary_checkpoint_does_not_override_default_submission_plan -- --exact` | 0（RED 态：124） |
| G3 | `timeout -k 5 300 cargo test -p pony-agent-core --lib submission_plan_bounded`（前缀冻结；须选中 ≥1 用例且含 `elapsed < 5000ms` 断言） | 0 |
| G4 | `timeout -k 5 300 cargo test -p pony-agent-core --lib submission_plan_starts_fresh_run_when_recovery_contract_requires_replay -- --exact` | 0（RED 态：124） |
| G5 | `timeout -k 5 300 cargo test -p pony-agent-core --lib submission_plan_switches_with_session_checkpoint_boundary -- --exact` | 0 |
| G6 | `timeout -k 5 300 cargo test -p pony-agent-core --lib submission_plan_`（既有 C4 相位映射用例集合全绿） | 0 |
| G7 | `timeout -k 5 300 cargo test -p pony-agent-core --lib submission_plan_paused`（前缀冻结；须选中 ≥1 用例） | 0 |
| G8 | `python3 .dev-team/check-lock-discipline.py` | 0 |
| G9 | `python3 .dev-team/nfr-lint.py` | 0 |
| G10 | `python3 .dev-team/nfr-lint.py --self-test` | 0（6/6 反例被正确拒绝） |

**G1/G2/G4 的 RED 语义**：这三条现有用例在修复前会命中死锁路径并**永久挂起**，`timeout` 于 300s 硬杀 → Exit 124。因此它们是本缺陷的**回归探针**，无需新增即已覆盖 C1/C2 主路径。

---

## 3. NFR 基准承诺（`.dev-team/nfr-baseline.json`）

本次修复相关的「并发锁 / 有界等待」承诺（全部为 schema 显式声明字段，非注释、非代码内嵌）：

| 字段 | 值 | 约束（schema） |
| --- | --- | --- |
| `concurrency_lock` | `"mutex"` | `type: string, minLength: 1` |
| `lock_scope_bounded` | `true` | `const: true` |
| `lock_reentrancy_forbidden` | `true` | `const: true` |
| `lock_acquisition_timeout_ms` | `2000` | `1 ≤ x ≤ 5000` |
| `deadlock_watchdog_timeout_ms` | `5000` | `1 ≤ x ≤ 5000`（≤ 5000 硬上限） |
| `lock_forbidden_calls` | C5 四个方法名 | `minItems: 4`、`uniqueItems: true`、`items.enum` 白名单 |

附加语义约束（由 `nfr-lint.py` 强制）：`lock_acquisition_timeout_ms < deadlock_watchdog_timeout_ms`。
未知字段拒绝：schema 顶层与 `retry` / `logging` 均 `additionalProperties: false`；`nfr-lint.py` 另做递归 unknown-key 检查（R2 反例证明有效）。

---

## 4. 非目标 / 遗留登记（严禁夹带重构）

| 编号 | 内容 | 定性 | 处置 |
| --- | --- | --- | --- |
| P1 | `mod.rs:2059` `stop_graph_run` 于 `self.graph_runs.lock()` 作用域内调用 `load_execution_checkpoint`（仅当 runtime checkpoint 缺失时才真正二次加锁） | 同类条件性隐患 | **登记，不阻塞本次放行**；`--strict` 模式可纳入后续清理门禁 |
| P2 | `src/stores/runtime.ts:1056` 硬编码 `runId: null` | 触发条件（放大暴露面），非缺陷本体 | 登记观察项；本次契约不要求修改前端 |
| P3 | `src-tauri/src/lib.rs:768` 同步命令在主线程执行重查询 | 架构级风险（放大了锁竞争/卡死的用户可见性） | 登记观察项；超出本次最小闭包 |

---

## 5. 物理收据

原始日志：`.dev-team/task1-receipts.log`（生成于 2026-10-14T05:36:59Z，rev `506220b`）。

| 收据 | 命令 | 关键 stdout | Exit Code |
| --- | --- | --- | --- |
| R1 | `python3 .dev-team/nfr-lint.py` | `ok: schema 校验通过；未知字段 0 个；lock_scope_bounded=True … deadlock_watchdog_timeout_ms=5000` / `RESULT: PASS` | **0** |
| R2 | `python3 .dev-team/nfr-lint.py --self-test` | `self-test cases=6 failures=0`（未知顶层/嵌套字段、缺 watchdog、watchdog>5000、forbidden 列表不全 均被拒） | **0** |
| R3 | `python3 .dev-team/check-lock-discipline.py` | `history_commands.rs L441 [REENTRANT_CALL]` + `mod.rs L2059 WARN(P1)`；`blocking_violations=1 registered_p1=1`；`RESULT: RED` | **1**（RED 基准，修复后须转 0） |
| R4 | `python3 .dev-team/check-lock-discipline.py --root .dev-team/fixtures/lock-discipline/good` | `RESULT: GREEN (C5 satisfied)` | **0** |
| R5 | `python3 .dev-team/check-lock-discipline.py --root .dev-team/fixtures/lock-discipline/bad` | `L13 [REENTRANT_CALL]`；`RESULT: RED` | **1** |
| R6 | `timeout -k 2 5 /tmp/deadlock_repro` | 无 stdout（进程未返回） | **124** |
| R6' | `timeout -k 2 5 /tmp/deadlock_fixed` | `returned=None elapsed=121ns` | **0** |
| R7 | `ls -l scripts/dev-nfr-lint.py` | `No such file or directory` | **2** |
| R8b | `jq -e '… (.deadlock_watchdog_timeout_ms<=5000) … ((keys\|length)==10)' .dev-team/nfr-baseline.json` | `true` | **0** |
| R9 | `python3 .dev-team/check-lock-discipline.py --quiet-ok` | `blocking_violations=1 registered_p1=1` / `RESULT: RED` | **1**（RED 基准复跑；修复后须转 0） |

> R8 首跑因编写方把顶层键数写成 9（实际 10）而 Exit 1，已在 `task1-receipts.log` 中**原样保留**并由 R8b 修正通过，不隐藏失败轨迹。

**R7 说明**：任务书提到的 `dev-nfr-lint.py` 在本仓库**不存在**（`scripts/` 下无此文件）。故按任务书允许的等价路径，自建 `.dev-team/nfr-lint.py`（基于 `jsonschema` 4.10.3 Draft-07 + 递归 unknown-key 检查 + 有界等待硬约束 + 反例自检）作为等价机械校验，收据 R1/R2 即其真实输出。

**R6 说明**：`/tmp/deadlock_repro.rs` 为**机制等价最小复现**（`std::sync::Mutex` + `let ... = { lock(); ….or_else(|| self.fallback()) }` 形态），编译产物置于 `/tmp`，**未触碰仓库 `target/`**；对照组把 fallback 移出锁作用域后 121ns 有界返回。该收据用于证明「同线程二次加锁 = 永久挂起」这一机制，替代无法在本次会话内执行的 `cargo test`（`target/` 由 Lead 独占构建中）。

**未执行项（诚实声明）**：G1–G7 需 `cargo test`（编译写入 `target/`），与 Lead 的桌面端构建互斥，故本次**未执行**；其 RED 基准由 R3（静态门禁精确命中 L441）+ R6（机制复现 124）+ 主线程 backtrace 三方证据锚定。

---

## 6. 变更记录

| 版本 | 时间 | 变更 |
| --- | --- | --- |
| `deadlock-v1` | 2026-10-14T05:36Z | 首版冻结：C1–C5 契约、G1–G10 门禁、NFR 并发锁字段、P1–P3 遗留登记、R1–R7 物理收据 |
