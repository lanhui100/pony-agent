# 契约矩阵：进入会话页卡死（graph_runs 非重入锁自死锁）冻结基准

| 项 | 值 |
| --- | --- |
| 契约版本 | `deadlock-v3`（v1 冻结 → v2 task-5 收口 → v3 task-9 复审整改；历史版本保留于变更记录） |
| 冻结时间 | 2026-10-14T05:36Z（v1）/ 06:16Z（v2）/ 06:50Z（v3） |
| **生效仓库修订** | **`ba0948e`**（`fix(core): eliminate graph_runs lock reentrancy self-deadlock in resolve_graph_run_submission_plan`）—— v3 全部收据均在此**干净修订**上重跑 |
| 历史修订 | `506220b`（v1 RED 基准，修复前）→ `533e7b8`（**v2 收据失效**，见下）→ `ba0948e`（v3 生效） |
| v2 收据失效声明 | v2 把 R10–R17 钉在 `533e7b8`，但那些 GREEN 结果实测的是**未提交工作树**；从纯净 `533e7b8` 快照复现得到 RED（`L441 REENTRANT_CALL`、`lock_sites=25`、`blocking=1`、Exit 1）。**v2 的 rev 标注作废**，以 v3 的 `ba0948e` 为准 |
| 唯一编写方 | protocol-agent（L2-P 协议特派员） |
| 写域 | 仅 `.dev-team/`（未触碰 `src/`、`src-tauri/`、`crates/`、`tests/`、`scripts/`；未执行 `cargo build`；未触碰 `target/`、`target-check/`） |
| 关联 NFR | `.dev-team/nfr-baseline.json` / `.dev-team/nfr-baseline.schema.json` |
| 机械门禁 | `.dev-team/nfr-lint.py`（`--self-test` / `--mutation-test`）、`.dev-team/check-lock-discipline.py` |
| 物理收据 | `.dev-team/task9-receipts.log`（**v3 生效收据，rev `ba0948e`**）、`.dev-team/task1-receipts.log`（v1 RED）、`.dev-team/task5-*.log`（v2，rev 标注已作废，数值保留为过程记录） |
| 收口依据 | L3-P 审查 FAIL（`/tmp/l3p-review.json`）→ task-5；L3-P **复审** FAIL（`/tmp/l3p-recheck.json`）→ task-9（收据修订校准 + 自检盲区 + logging 豁免措辞 + E2E 收据修正 + TOCTOU 登记） |
| 工作树 clean 自证 | `git log -1` = `ba0948e`；`git status --porcelain -- crates src src-tauri scripts tests` = **空**；`git diff HEAD --stat -- crates src src-tauri scripts tests` = **空**。全仓仅 `.dev-team/**`（本契约证据工件）与 `management/task-system/01_TASK_BOARD.md`（Lead 看板）为脏，二者均非代码域（见 `task9-receipts.log` R20） |

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
| **ERROR（阻塞）** | 死锁路径文件 `history_commands.rs` | `python3 .dev-team/check-lock-discipline.py` → **Exit 0** | **GREEN**（rev `ba0948e`，R24）：`blocking_violations=0`、`lock_sites=26`、`history_commands.rs` = ok；v1 的 RED 基准（L441 于 L432 起的作用域内调用 `resolve_graph_run_for_retrieval`）已由修复闭环，保留为历史收据 R3/R9 |
| P1（登记，不阻塞） | 其它文件 | 同命令 `WARN(P1)` 输出 | `mod.rs:2059` 于 L2039 起作用域内调用 `load_execution_checkpoint`（`stop_graph_run`，条件性同类隐患） |

**非空证明（防恒真门禁）**：
- 负向对照 `.dev-team/fixtures/lock-discipline/good/` → Exit 0
- 正向对照 `.dev-team/fixtures/lock-discipline/bad/` → Exit 1
- 全模块扫描 10 文件 / 26 处 `self.graph_runs.lock()`（v2），仅报 0 ERROR + 1 P1（无误报）

---

## 2. 机械判定命令清单（验收门禁，冻结）

> 由 Lead / 独立验收方执行；Executor 不得自行裁决验收结果。
> 表中所有 `cargo test` 命令在实测时均以 `timeout -k 5 1800` 包裹（死锁时硬杀 → Exit 124），表中展示的是被包裹的命令本体。

| ID | 命令（**已执行口径**） | 期望 Exit Code / 选中用例数 |
| --- | --- | --- |
| G1 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test --lib agent::control_plane::tests::submission_plan_prefers_checkpoint_projection_when_available -- --exact --test-threads=1` | 0 **且 `running N tests` 中 N = 1** |
| G2 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test --lib agent::control_plane::tests::lifecycle_boundary_checkpoint_does_not_override_default_submission_plan -- --exact --test-threads=1` | 0 **且 N = 1** |
| G3 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test --test graph_run_submission_plan_deadlock submission_plan_bounded -- --test-threads=1` | 0 **且 N = 5** |
| G4 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test --lib agent::control_plane::tests::submission_plan_starts_fresh_run_when_recovery_contract_requires_replay -- --exact --test-threads=1` | 0 **且 N = 1** |
| G5 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test --lib agent::control_plane::tests::submission_plan_switches_with_session_checkpoint_boundary -- --exact --test-threads=1` | 0 **且 N = 1** |
| G6 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test --lib -- submission_plan_ --test-threads=1` | 0 **且 N = 8**（5 例 `agent::control_plane::tests` + 3 例 `agent::hooks::tests`） |
| G7 | `cargo test --manifest-path crates/pony-agent-core/Cargo.toml --target-dir target-test --test graph_run_submission_plan_deadlock submission_plan_paused -- --test-threads=1` | 0 **且 N = 2** |
| G8 | `python3 .dev-team/check-lock-discipline.py` | 0（`blocking_violations=0`） |
| G9 | `python3 .dev-team/nfr-lint.py` | 0（输出含 `NOTE(logging)` 恒真防护声明） |
| G10 | `python3 .dev-team/nfr-lint.py --self-test` | 0（**21/21** 反例被正确拒绝） |
| G11 | `python3 .dev-team/nfr-lint.py --mutation-test` | 0（钝化 `check_logging_scope` 后 **必须**出现失败；证明 python 独占分支被覆盖） |

**G1/G2/G4 的 RED 语义**：这三条现有用例在修复前会命中死锁路径并**永久挂起**（`timeout` 硬杀 → Exit 124）。因此它们是本缺陷的**回归探针**，无需新增即已覆盖 C1/C2 主路径；修复后实测 `1 passed / 0 failed`。

**⚠ 假绿禁令（L3-P 审查 FAIL 项 + task-5 追加发现，v1→v2 修正）**

libtest 的过滤语义决定了"只看 Exit Code"必然可被假绿击穿。本波实测到**两处同类假绿**：

| 假绿形态 | 实测证据 | 原因 |
| --- | --- | --- |
| v1 G3/G7：`cargo test --lib submission_plan_bounded`（用例实为**集成测试**） | `running 0 tests` / `test result: ok. 0 passed` / **Exit 0** | `--lib` 不覆盖 `tests/*.rs` target，过滤命中 0 例 |
| v1 G1/G2/G4/G5：`--lib <裸函数名> -- --exact` | `running 0 tests` / **Exit 0**（见 R16） | `--exact` 要求**全路径**测试名（`agent::control_plane::tests::<fn>`），裸函数名命中 0 例 |

**冻结规则（后续所有门禁命令必须满足）**：
1. 任何测试名过滤门禁命令，**必须校验 `running N tests` 的 N ≥ 契约声明值**（本契约：G1/G2/G4/G5=1、G3=5、G6=8、G7=2）；仅 Exit Code 0 不足以放行。
2. 使用 `--exact` 时必须给**全路径**测试名；否则去掉 `--exact` 并按子串过滤 + 校验选中数。
3. 集成测试（`tests/*.rs`）必须用 `--test <target>` 口径，禁用 `--lib`。
4. 门禁实现方（含 AI）不得以"命令 Exit 0"替代"用例确实被选中且通过"的证明。

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
| `logging`（原字段保留） | `{format:"json", trace_id_required:true}` | `additionalProperties:false`，不得删除；`trace_id_required` 仅 `type:boolean`（→ python 独占强制分支） |
| `logging_scope` | `"new_structured_logging_surface_only"` | `enum` 单值（适用范围收窄声明） |
| `logging_legacy_eprintln_exempt` | `true` | `const: true` |
| `logging_debt_ref` | `"PA-119"` | `const: "PA-119"`（**由 schema const 钉死**） |
| `logging_eprintln_exempt_scope` | `"existing_style_non_json_lock_poison_recovery_diagnostics"` | `enum` 单值（豁免覆盖范围显式化） |
| `logging_eprintln_exempt_covers_new_lines` | `1` | `integer, minimum: 1`（禁止"声明豁免却覆盖 0 处新增行"） |
| `logging_compliance_claimed` | `false` | `const: false`（**恒真防护**：禁止主张 logging NFR 已满足） |

附加语义约束（由 `nfr-lint.py` 强制）：`lock_acquisition_timeout_ms < deadlock_watchdog_timeout_ms`；`logging_eprintln_exempt_covers_new_lines ≥ 1`；`logging_compliance_claimed` 必须为 `false`。
未知字段拒绝：schema 顶层与 `retry` / `logging` 均 `additionalProperties: false`；`nfr-lint.py` 另做递归 unknown-key 检查（R2/R11 反例证明有效）。
**分支归属（不得含糊）**：`logging_debt_ref` / `logging_eprintln_exempt_scope` / `logging_eprintln_exempt_covers_new_lines` / `logging_compliance_claimed` 均由 **schema 的 `const`/`enum`/`minimum`** 强制，python 侧同名判断属纵深防御；**唯一 python 独占强制分支是 `logging.trace_id_required != true`**（schema 无 const），由 G11 `--mutation-test` 证明其被 self-test 覆盖。

### 3.1 logging 适用范围收窄与豁免（Lead 裁定：task-5 立约 / task-9 整改）

L3-P 审查指出：本次修复新增的 `eprintln!` 诊断输出不满足 `logging{format:json, trace_id_required:true}`。
**Lead 裁决**：本次**不引入结构化日志框架**（属跨切面重构，违反最小闭包原则）。契约层处置如下：

1. **适用范围显式收窄**：`logging{format,trace_id_required}` 的约束范围收窄为 **`new_structured_logging_surface_only`**（仅新增结构化日志面），原字段与其必填/拒绝语义**完整保留**，`additionalProperties:false` 不变。
2. **豁免显式化，范围如实声明（task-9 整改）**：豁免 = `logging_legacy_eprintln_exempt = true` + `logging_debt_ref = "PA-119"` + **`logging_eprintln_exempt_scope = "existing_style_non_json_lock_poison_recovery_diagnostics"`** + **`logging_eprintln_exempt_covers_new_lines = 1`**。
   - 覆盖范围如实为：**既有风格（非 JSON）的锁中毒恢复诊断输出，含本波新增的 1 处**（`history_commands.rs:456`，修复提交 `ba0948e` 使该文件 `eprintln!` 由 11 处增至 12 处）。v2 曾声称"仅覆盖 legacy eprintln"，与本波事实不符，**该表述作废**。
   - 字段名 `logging_legacy_eprintln_exempt` 沿用 task-5 冻结名（保持向后兼容），**其语义以 `logging_eprintln_exempt_scope` 为准**，不再声称"仅覆盖 legacy"。
   - 关于"正则强制债务单"的表述**已纠正**：`logging_debt_ref` 由 schema `const:"PA-119"` 钉死；`nfr-lint.py` 中的 `^PA-[0-9]+$` 正则为**纵深防御，不构成可独立触发的强制分支**，不得声称其能独立拒绝。
3. **恒真防护（task-9 整改，禁止恒真放行）**：`logging_scope="new_structured_logging_surface_only"` 意味着**当前全仓库无结构化日志面**，`logging{format=json,trace_id_required}` 因此**无代码级检查点、零强制力**。契约明确声明：
   - 本波**不主张** `logging{format=json,trace_id_required}` 这一 NFR「已满足」；
   - 本波只主张「**已显式登记债务（PA-119）+ 有到期条款 + 有恒真防护**」；
   - 机器可校验的防护 = `logging_compliance_claimed` 必须为 `false`（`const:false`），任何人把它改为 `true` 都会被 schema + self-test 拒绝；
   - `nfr-lint.py` 每次 PASS 输出都打印 `NOTE(logging)`，明文声明"无代码级检查点、不主张已满足、禁止读作恒真放行"，与本节措辞一致。
4. **到期条款（强制）**：**PA-119（结构化日志迁移）完成时必须重跑本波 L3-P 五问并关闭本豁免**；关闭动作 = 将 `logging_legacy_eprintln_exempt` 置 `false`、`logging_compliance_claimed` 置 `true`（届时须有真实代码级检查点）、同步 schema/self-test 并重新通过 `nfr-lint.py --self-test` + `--mutation-test`。在 PA-119 关闭前，本豁免**不得**被援引为新增结构化日志面免于 `logging` 约束的依据。

---

## 4. 非目标 / 遗留登记（严禁夹带重构）

| 编号 | 内容 | 定性 | 处置 |
| --- | --- | --- | --- |
| P1 | `mod.rs:2059` `stop_graph_run` 于 `self.graph_runs.lock()` 作用域内调用 `load_execution_checkpoint`（仅当 runtime checkpoint 缺失时才真正二次加锁） | 同类条件性隐患 | **登记，不阻塞本次放行**；`--strict` 模式可纳入后续清理门禁 |
| P2 | `src/stores/runtime.ts:1056` 硬编码 `runId: null` | 触发条件（放大暴露面），非缺陷本体 | 登记观察项；本次契约不要求修改前端 |
| P3 | `src-tauri/src/lib.rs:768` 同步命令在主线程执行重查询 | 架构级风险（放大了锁竞争/卡死的用户可见性） | 登记观察项；超出本次最小闭包 |
| P4 | 既有风格（非 JSON）的锁中毒恢复诊断输出不满足 `logging{format:json,trace_id_required}`（含本波新增 1 处 `history_commands.rs:456`） | 跨切面日志重构（违反最小闭包，Lead 裁定本次不做） | **有条件豁免**：`logging_scope` 收窄为 `new_structured_logging_surface_only` + `logging_legacy_eprintln_exempt=true` + `logging_eprintln_exempt_scope` 显式覆盖范围 + `logging_eprintln_exempt_covers_new_lines=1` + `logging_compliance_claimed=false`（恒真防护）绑定 `logging_debt_ref="PA-119"`；**到期条款见 §3.1 第 4 条** |
| P5 | **fallback 路径 TOCTOU**：T7 只对 `run_id` 命中路径做了原子化（同一锁作用域内取 run 与 ask-wait 判定）；**生产路径** `src/stores/runtime.ts:1056` 恒发 `runId=null` → 恒走 fallback，取 run 与 ask-wait 判定之间仍存在「释放锁 → 重取锁」窗口 | 已知项（**非阻塞**）；依据 L3 复核：`24c064c` 之前该路径**无 ask-wait 判定**、`24c064c..HEAD` 该路径**必自死锁** ⇒ **原子语义从未可达**，故属**既有设计空洞而非本波回归** | 登记为 P 级已知项，本波不修（修它需引入快照/世代号，超出最小闭包）；若后续要求原子语义，须以「单次持锁完成 run 选择 + ask-wait 判定」为验收判据 |

---

## 5. 物理收据

原始日志：
- `.dev-team/task9-receipts.log`（**v3 生效收据：R20–R27，rev `ba0948e`，代码域 clean**，2026-10-14T06:49Z）
- `.dev-team/task1-receipts.log`（v1 RED 基准，rev `506220b`，2026-10-14T05:36:59Z）
- `.dev-team/task5-receipts.log` / `task5-cargo-*.log`（v2 过程收据；**rev 标注已作废**，数值保留，见 §5.3）

### 5.1 v1 RED 基准（历史，rev `506220b`）

| 收据 | 命令 | 关键 stdout | Exit Code |
| --- | --- | --- | --- |
| R1 | `python3 .dev-team/nfr-lint.py` | `ok: schema 校验通过；未知字段 0 个；lock_scope_bounded=True … deadlock_watchdog_timeout_ms=5000` / `RESULT: PASS` | **0** |
| R2 | `python3 .dev-team/nfr-lint.py --self-test` | `self-test cases=6 failures=0`（未知顶层/嵌套字段、缺 watchdog、watchdog>5000、forbidden 列表不全 均被拒） | **0** |
| R3 | `python3 .dev-team/check-lock-discipline.py` | `history_commands.rs L441 [REENTRANT_CALL]` + `mod.rs L2059 WARN(P1)`；`blocking_violations=1 registered_p1=1`；`RESULT: RED` | **1**（RED 基准） |
| R4 | `python3 .dev-team/check-lock-discipline.py --root .dev-team/fixtures/lock-discipline/good` | `RESULT: GREEN (C5 satisfied)` | **0** |
| R5 | `python3 .dev-team/check-lock-discipline.py --root .dev-team/fixtures/lock-discipline/bad` | `L13 [REENTRANT_CALL]`；`RESULT: RED` | **1** |
| R6 | `timeout -k 2 5 /tmp/deadlock_repro` | 无 stdout（进程未返回） | **124** |
| R6' | `timeout -k 2 5 /tmp/deadlock_fixed` | `returned=None elapsed=121ns` | **0** |
| R7 | `ls -l scripts/dev-nfr-lint.py` | `No such file or directory` | **2** |
| R8b | `jq -e '… (.deadlock_watchdog_timeout_ms<=5000) … ((keys\|length)==10)' .dev-team/nfr-baseline.json` | `true` | **0** |
| R9 | `python3 .dev-team/check-lock-discipline.py --quiet-ok` | `blocking_violations=1 registered_p1=1` / `RESULT: RED` | **1** |

> R8 首跑因编写方把顶层键数写成 9（实际 10）而 Exit 1，已在 `task1-receipts.log` 中**原样保留**并由 R8b 修正通过，不隐藏失败轨迹。

### 5.2 v3 生效收据（**rev `ba0948e`，干净工作树**，2026-10-14T06:49Z）

原始日志：`.dev-team/task9-receipts.log`。**本节为唯一生效收据**。

| 收据 | 命令 | 关键 stdout | Exit Code |
| --- | --- | --- | --- |
| R20 | `git log -1` / `git status --porcelain -- crates src src-tauri scripts tests` / `git diff HEAD --stat -- crates src src-tauri scripts tests` | `ba0948ea2673863deb6379923fdc0ce80427b8cc fix(core): eliminate graph_runs lock reentrancy self-deadlock…`；后两条**均为空** ⇒ 代码域 clean | **0** |
| R21 | `python3 .dev-team/nfr-lint.py` | `RESULT: PASS` + `NOTE(logging): …无代码级检查点…不主张该 NFR 已满足…禁止读作恒真放行` | **0** |
| R22 | `python3 .dev-team/nfr-lint.py --self-test` | `self-test cases=21 failures=0` | **0** |
| R23 | `python3 .dev-team/nfr-lint.py --mutation-test` | 钝化后 `failures=1 failing_cases=['logging-trace-id-false-rejected']`；`PASS … python 独占分支用例全部被覆盖` | **0** |
| R24 | `python3 .dev-team/check-lock-discipline.py` | `scanned_files=10 lock_sites=26 blocking_violations=0 registered_p1=1` / `RESULT: GREEN` | **0** |
| R25 | `jq -e '((keys\|length)==16) and (.logging_compliance_claimed==false) …' .dev-team/nfr-baseline.json` | `true` | **0** |
| R26 | G1/G2/G4/G5：`cargo test --target-dir target-test --lib <全路径> -- --exact` | 每条 `1 passed; 0 failed` | **0** ×4 |
| R26b | G3：`… --test graph_run_submission_plan_deadlock submission_plan_bounded` | `running 5 tests` / `5 passed; 0 failed; 8 filtered out` | **0** |
| R26b | G7：`… --test graph_run_submission_plan_deadlock submission_plan_paused` | `running 2 tests` / `2 passed; 0 failed; 11 filtered out` | **0** |
| R26b | G6：`… --lib -- submission_plan_` | `running 8 tests` / `8 passed; 0 failed; 1006 filtered out`（5 `control_plane::tests` + 3 `hooks::tests`） | **0** |
| R26c | `… --test graph_run_submission_plan_deadlock`（全量，附加信息） | `running 13 tests` / `13 passed; 0 failed` | **0** |
| R27 | `sha256sum`（E2E 截图，见 §5.4） | BEFORE `a8455855…` / AFTER `b53052fa…` | **0** |

**合计 19 例契约用例全绿**（G1/G2/G4/G5 各 1 + G3 5 + G6 8 + G7 2），另有集成测试全量 13 例佐证。
**判据两段制**：`timeout -k 5 1800` 硬杀负责"不得死锁/有界返回"（挂起 → 124 ≠ 0）；`running N tests` 计数负责"不得空过滤假绿"（N 必须等于契约声明值）。仅凭 Exit Code 0 一律不得放行。
**过程修正留痕**：R26 首跑的 shell 辅助函数参数传递有误，导致 G3/G7 误跑全量 13 例、G6 空过滤为 0 例；已在 R26b 用逐条字面命令修正并保留原输出，不隐藏失败轨迹。

### 5.3 v2 过程收据（**rev 标注作废**，数值仅作过程记录）

| 收据 | 内容 | Exit Code | 状态 |
| --- | --- | --- | --- |
| R10 | `nfr-lint.py` PASS（logging 4 字段版） | 0 | 数值有效，**rev `533e7b8` 标注作废** |
| R11 | `nfr-lint.py --self-test` `cases=14 failures=0` | 0 | 同上；**14 例存在盲区，已被 v3 的 21 例取代** |
| R12 | `check-lock-discipline.py` GREEN（`lock_sites=26 blocking=0`） | 0 | 同上 |
| R13 | `jq … keys==13` | 0 | 已被 R25（keys==16）取代 |
| R14/R15 | G3=5 例 / G7=2 例 | 0 | 已在 R26b 于 `ba0948e` 重跑 |
| R16 | G1/G2/G4/G5 全路径 `--exact` 各 1 例 | 0 | 已在 R26 于 `ba0948e` 重跑 |
| R16' | 假绿复现：裸函数名 + `--exact` → `running 0 tests` / Exit 0 | 0 | **有效**（假绿实证，与 rev 无关） |
| R17 | G6 = 8 例 | 0 | 已在 R26b 于 `ba0948e` 重跑 |

> **v2 失效根因（L3-P 复审认定）**：v2 的 GREEN 结果实测于**未提交工作树**，而 rev 却标注为 `533e7b8`（该提交**不含**修复）。从纯净 `533e7b8` 快照复现得 RED（`L441 REENTRANT_CALL`、`lock_sites=25`、`blocking=1`、Exit 1）。v3 改为在干净修订 `ba0948e` 上重跑全部收据，并附代码域 clean 自证（R20）。

### 5.4 实机 E2E 证据（**wchan 归属：E2E 驱动日志**，非 diag-probe 文档）

**口径声明（BEFORE/AFTER 同一生产口径）**：`cargo build --manifest-path src-tauri/Cargo.toml --features tauri/custom-protocol`（内嵌 `dist`）；隔离 `HOME`（`/tmp/pony-run-e2e-*/home`）下真实 `sessions.db`；`Xvfb` + `dbus-run-session`；驱动脚本每 1s 采样主线程 `state/wchan/cpu` 并在 t=10/20/30s 截图。

| 维度 | BEFORE（修复前） | AFTER（修复后，**权威收据**） |
| --- | --- | --- |
| **驱动日志** | `/tmp/pony-before2-e2e.log` | `/tmp/pony-final3-e2e.log`（干净修订 `ba0948e` 上构建的二进制） |
| **wchan（归属：驱动日志）** | `main_thread_wchan=futex_wait_queue`（**全程 30s 恒定**） | `main_thread_wchan=do_poll.constprop.0`（t=1s 后进入事件循环等待） |
| **VERDICT（归属：驱动日志）** | `VERDICT=FROZEN_FUTEX` | `VERDICT=RUNNING_OK` |
| 采样连续性 | t=1s…30s 全部 `wchan=futex_wait_queue`，cpu 由 48.5% 单调衰减至 3.0% | t=1s `futex_wait_queue` → t=2s `R` → t=3s 起 `do_poll.constprop.0` 稳定至 t=30s |
| 截图 | `/tmp/pony-run-e2e-before2/shot-30s.png`，`sha256:a8455855d143ec9a4f23f20d2ec79d7d5f5813fabce8455ae2865aea9320bdbf`，91,599 B | `/tmp/pony-run-e2e-final3/shot-30s.png`，`sha256:b53052fa79c475d20f6279837a5ab43d8c5d9cec1b0dfcd0d6ddb32f2f59a744`，167,745 B |
| 界面 | 未渲染出会话页（主线程被同步命令占死） | 渲染出真实会话页：左侧会话列表 + 中间对话正文 + 右侧状态栏 + 底部输入区 |
| 过程收据（保留） | `/tmp/pony-run-e2e/shot-30s.png`（`0bc68ae4…`，10/20/30s 三图字节相同） | `/tmp/pony-final2-e2e.log`（`do_poll`、RUNNING_OK）+ `/tmp/pony-run-e2e-final2/shot-30s.png`（`666b707fe9917f3edbdcb8562e7d98a2aced893ffc5223e33faf8276afb15212`） |

**结论**：同一生产口径下，BEFORE 主线程 30s 全程阻塞于 `futex_wait_queue`（锁等待，对应 C5 自死锁），AFTER 主线程在 t=3s 起稳定于 `do_poll`（事件循环），`VERDICT=RUNNING_OK`，与 R24（C5 GREEN）及 R26b（G3/G7 全绿）结论一致。
**归属修正（task-9）**：v2 曾把 `wchan` 观察挂到 diag-probe 文档，**该归属作废**；`wchan`/`VERDICT` 一律以 **E2E 驱动日志**（BEFORE `pony-before2-e2e.log`、AFTER `pony-final3-e2e.log`）为唯一出处。

**R7 说明**：任务书提到的 `dev-nfr-lint.py` 在本仓库**不存在**（`scripts/` 下无此文件）。故按任务书允许的等价路径，自建 `.dev-team/nfr-lint.py`（`jsonschema` 4.10.3 Draft-07 + 递归 unknown-key 检查 + 有界等待/日志范围硬约束 + 21 例反例自检 + `--mutation-test` 分支覆盖证明）。

**R6 说明**：`/tmp/deadlock_repro.rs` 为**机制等价最小复现**（`std::sync::Mutex` + `let ... = { lock(); ….or_else(|| self.fallback()) }` 形态），编译产物置于 `/tmp`，**未触碰仓库 `target/`**；对照组把 fallback 移出锁作用域后 121ns 有界返回。该收据证明「同线程二次加锁 = 永久挂起」这一机制。

**执行范围声明（v3）**：G1–G7 全部在**干净修订 `ba0948e`** 上以 `--target-dir target-test` 重跑（独立构建目录，**未触碰 `target/` 与 `target-check/`**，`target-test/` 已在 `.gitignore:41` 中）；未执行 `cargo build`。

---

## 6. 变更记录

| 版本 | 时间 | 变更 |
| --- | --- | --- |
| `deadlock-v1` | 2026-10-14T05:36Z | 首版冻结：C1–C5 契约、G1–G10 门禁、NFR 并发锁字段、P1–P3 遗留登记、R1–R9 物理收据（RED 基准，rev `506220b`） |
| `deadlock-v2` | 2026-10-14T06:20Z | task-5 收口（依据 L3-P 审查 FAIL）：① 修正假绿门禁——G3/G7 改集成测试口径（N=5/2），并**追加发现并修正** G1/G2/G4/G5 的裸函数名 `--exact` 假绿（改全路径 `--exact`，N=1），冻结"必须校验选中用例数"规则；② logging 适用范围收窄为 `new_structured_logging_surface_only` + `logging_legacy_eprintln_exempt`/`logging_debt_ref="PA-119"`（P4），登记到期条款，self-test 扩至 14 例；③ 补录 BEFORE/AFTER 实机 E2E 证据；④ G1–G7 实测 19 例全绿。**注：v2 的 rev 标注（`533e7b8`）经 L3-P 复审认定失效，见 v3** |
| `deadlock-v3` | 2026-10-14T06:52Z | task-9 复审整改（依据 L3-P **复审** FAIL `/tmp/l3p-recheck.json`）：① **收据修订校准**——生效修订改 `ba0948e`，在代码域 clean 工作树（R20 自证）上重跑 G1–G7 + 全部守卫，v2 的 `533e7b8` 标注作废并保留为过程记录；② **自检盲区修复**——self-test 14→**21** 例，新增 `logging.trace_id_required=false` 等 7 例负例，新增 `--mutation-test`（G11）证明钝化 `check_logging_scope` 后 `logging-trace-id-false-rejected` **必由 PASS 翻 FAIL**，纠正"正则可独立强制债务单"的错误表述（债务单实由 schema `const` 钉死）；③ **logging 豁免措辞与恒真问题**——如实声明豁免覆盖"既有风格（非 JSON）锁中毒恢复诊断输出，**含本波新增 1 处**（`history_commands.rs:456`）"，新增 `logging_eprintln_exempt_scope`/`logging_eprintln_exempt_covers_new_lines`/`logging_compliance_claimed=false`（恒真防护），并写明 `logging{format=json,trace_id_required}` **无代码级检查点、属 PA-119 债务、本波不主张已满足**，lint 输出同步 `NOTE(logging)`；④ **E2E 收据修正**——`wchan`/`VERDICT` 归属改挂 E2E 驱动日志（BEFORE `/tmp/pony-before2-e2e.log` + `a8455855…`；AFTER `/tmp/pony-final3-e2e.log` + `b53052fa…`，final2 保留为过程收据）；⑤ 新增 **P5 fallback 路径 TOCTOU** 登记（非阻塞，原子语义从未可达，非回归） |
