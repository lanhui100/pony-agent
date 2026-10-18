# Wave-36 红相冻结清单（Test Agent / task-2）

- 冻结时间：2026-10-18T13:52+08:00
- 基线 commit：`ddbc2fdaaf45447d36310603b57d1984191e1dfd`（`git rev-parse HEAD`）
- 契约：`.dev-team/contracts/wave-36-pa138-145.md`（Protocol Agent，冻结版，含 §0 supersede / §14 裁决）
- 环境：Linux x86_64，cargo test（debug），Rust workspace `pony-agent-core`
- 分级：L2（组件/契约）+ 1 个 L1 契约一致性用例 + 2 个验收仪器脚本
- **冻结声明：自本文件落盘起，以下测试文件只读，Executor 不得改判；后续绿相由 executor 改实现，测试保持原样。**
  唯一例外：C-141-* 的新测试文件 `tests/wave36_plan_execution_approval.rs` 于 PA-141 骨架落地后由 Lead 通知补写（见 §5）。

## 1. 测试文件清单与冻结哈希（sha256）

| 文件 | 改动类型 | tests | sha256 |
| :--- | :--- | :--- | :--- |
| `crates/pony-agent-core/tests/wave36_draft_gate_session.rs` | 新增（PA-138 C-138-1/3/4） | 4 | `ee1afef75bc6fe66b0233e6d0a507f86d170bb4fbd0a28f5ffb9023bebf2efca` |
| `crates/pony-agent-core/tests/wave36_recovery_retry_consistency.rs` | 新增（PA-145 C-145-3） | 1 | `23a838de826e54aaee7c51cb65c327e76ab0d9658997d72935c53460b5d0fab4` |
| `crates/pony-agent-core/src/agent/retry.rs` | 仅 `#[cfg(test)]` 区（≥640）新增 PA-143/144 两测 | +2 | `504b57593eb3472ae25d4aca095fce168ab89d2df4b7850a8bce4eeeb87e5016` |
| `crates/pony-agent-core/src/agent/provider/mod.rs` | 仅 `#[cfg(test)]` 区修订 PA-143 契约测试 | 0 | `32f4a129598f117df6ea61265039d6a0e99b1ab4a505c32655a3c8700a0bc1be` |
| `crates/pony-agent-core/src/agent/path_permission.rs` | 仅 `#[cfg(test)]` 区补 POSIX else 断言（PA-142） | 0 | `c3a9dbec21015da02784761bc5ed02f9550c1b0fdcaf2e595e5beda6bbd0ef03` |
| `tests/acceptance/remediation/wave36_zero_assert_census.sh` | 新增验收仪器（PA-142 C-142-2/3） | — | `571df8eb9d3f1b12da657f8d36c9b1b4df1e92119731a10ed840a62cd488ad76` |
| `tests/acceptance/remediation/wave36_pa139_full_lib_50x.sh` | 新增验收仪器（PA-139 C-139-1） | — | `7b49f98d3e33b833d648ce49e3bcc8a6596d2f5dae71e6935579fcb0cc4a4e40` |

未改动任何 `#[cfg(test)]` 之外的业务逻辑；未 `#[ignore]`；未 `#[allow]`；未删除既有断言。

## 2. 红相/绿相收据（`tests/acceptance/receipts/wave36/`）

红色 = 当前 HEAD 上语言原生 assert/panic 失败（exit 101），非编译失败。

| C-id | 用例 | 命令 | 当前结果 | 收据 |
| :--- | :--- | :--- | :--- | :--- |
| **C-138-1** | `draft_gate_blocks_write_for_runtime_injected_session` | `cargo test -p pony-agent-core --test wave36_draft_gate_session -- draft_gate_blocks_write_for_runtime_injected_session` | **RED exit 101**：写工具返回 `status="ok"`（`left: ok / right: error`）⇒ 门禁回退 `"default"`，注入会话 `sess` 的 Draft 匹配不到 | `T-W36-138-01_draft_gate_session_red.log` |
| **C-138-3** | `draft_gate_does_not_cross_block_other_sessions`（两分支：跨会话不误伤 / 无 Draft 不拦截） | `cargo test ... --test wave36_draft_gate_session -- draft_gate_does_not_cross_block_other_sessions` | GREEN exit 0（1 passed） | `T-W36-138-03_cross_session_control.log` |
| **C-138-4** | `draft_gate_fails_closed_when_context_session_is_missing`（分支1：session=None 且任一 Draft ⇒ Deny；分支2：无 Draft ⇒ Allow） | `cargo test ... --test wave36_draft_gate_session -- draft_gate_fails_closed_when_context_session_is_missing` | **RED exit 101**：分支1 写工具 `ok:true`（应 Deny `plan_draft_gate`） | `T-W36-138-04_fail_closed_red.log` |
| （PA-138 冻结回归护栏）| `draft_gate_keeps_plan_control_exempt`（plan_control merge 不被 `plan_draft_gate` 拒绝） | `cargo test ... --test wave36_draft_gate_session -- draft_gate_keeps_plan_control_exempt` | GREEN exit 0（与 §0 supersede 无冲突；start_execution 审批语义归 C-141） | `T-W36-138-02_plan_control_exempt_guard.log` |
| **C-142-1** | `path_permission::tests::win_prefix_normalization_strips_verbatim`（POSIX else 分支 4 条真断言：zone==WorkspaceRoot、canonical==root.join(raw)、≠ `C:\WS\file`、starts_with(root)） | `cargo test -p pony-agent-core --lib -- agent::path_permission::tests::win_prefix_normalization_strips_verbatim` | GREEN exit 0（1 passed，Linux 上真断言执行） | `T-W36-142-03_path_permission_verbatim_green.log` |
| **C-142-2** | census 基线红：`git show HEAD:.../path_permission.rs` 重放 | `bash tests/acceptance/remediation/wave36_zero_assert_census.sh`（对基线文件） | **RED exit 1**：`VIOLATION [CFG_ONLY_ASSERTIONS] ... line 757, fn win_prefix_normalization_strips_verbatim` | `T-W36-142-01_census_red_baseline.log` |
| **C-142-2** | census 当前树绿 | 同上（默认扫描 `crates/pony-agent-core/src`） | GREEN exit 0：`RESULT: PASS — 0 violation / scanned 110 file(s)` | `T-W36-142-02_census_green_after_fix.log` |
| **C-142-3** | census 仪器自证（V1/V2 正反例 + 断言委托/失败原语判据） | `bash .../wave36_zero_assert_census.sh --self-test` | GREEN exit 0：`SELFTEST RESULT: PASS` | `T-W36-142-01_census_selftest.log` |
| **C-142-5 / C-139-3** | 静态护栏：`#[ignore]`=0、`assert!(true)`=0；`MOCK_SERVER_FINISH_TIMEOUT=5000` 保持 | grep 判据 | `ignore_count=0 / vacuous_assert_count=0 / finish_timeout_hits=1` | `T-W36-142-05_T-W36-139-03_static_guards.log` |
| **C-143-1/2** | `agent::retry::tests::stage2_classify_does_not_match_digits_inside_longer_numbers` | `cargo test -p pony-agent-core --lib -- agent::retry::tests::stage2_classify_does_not_match_digits_inside_longer_numbers` | **RED exit 101**：`elapsed=40832ms -> TransientRetryable`（`408` 命中） | `T-W36-143-01_retry_digit_substring_red.log` |
| **C-143-3** | `agent::provider::tests::is_rate_limit_error_matches_provider_wording`（修订：`tpm` 正例 + bare `429001` 负例） | `cargo test -p pony-agent-core --lib -- agent::provider::tests::is_rate_limit_error_matches_provider_wording` | **RED exit 101**：`!is_rate_limit_error("...code":"429001")` 失败 | `T-W36-143-02_provider_rate_limit_contract_red.log` |
| **C-144-1** | `agent::retry::tests::stage2_classify_ignores_schemeless_host_port_digits`（`:42953` / `:50208` / 精确 `:429` / `:502`） | `cargo test -p pony-agent-core --lib -- agent::retry::tests::stage2_classify_ignores_schemeless_host_port_digits` | **RED exit 101**：`endpoint=127.0.0.1:42953/v1 -> TransientRetryable` | `T-W36-144-01_retry_schemeless_endpoint_red.log` |
| **C-145-3** | `wave36_recovery_retry_consistency::l1_recovery_and_retry_classification_agree_on_same_error_text` | `cargo test -p pony-agent-core --test wave36_recovery_retry_consistency` | **RED exit 101**：`operation timed out` 在 recovery 侧 `Abort` 而 retry 侧 `TransientRetryable`（左 false/右 true）⇒ 平行判据已漂移（recovery 表只有 `timeout` 无 `timed out`） | `T-W36-145-01_recovery_retry_consistency_red.log` |
| **C-145-4** | 既有契约护栏 `tests/recovery_acceptance.rs` | `cargo test -p pony-agent-core --test recovery_acceptance` | GREEN exit 0（6 passed，含 529 fallback / context 溢出） | `T-W36-145-04_recovery_acceptance_guard.log` |
| **C-139-3** | 禁止掩膜回归：wave-35 端口/URL 剥离用例 | `cargo test -p pony-agent-core --lib -- agent::retry::tests::stage2_classify_ignores_endpoint_port_digits` / `-- ..._url_wording` | GREEN exit 0（各 1 passed） | `T-W36-139-03_guard_endpoint_port_strip.log` / `..._url_wording.log` |
| **C-139-1** | 50 轮全量脚本（仅脚本自证，未跑满；红相基线由独立样本 1/8、1/13 证明） | `bash tests/acceptance/remediation/wave36_pa139_full_lib_50x.sh` | 脚本冒烟：失败检测 PASS（`T-W36-139-00_script_fail_detection.log`）、绿径 PASS（`..._selftest_green.log` / `..._selftest_summary.log`）；**50/50 留待 executor-a 修复后执行** | 同上 + `pa139_runs/` |

## 3. 红相失败性质核验（非编译失败、非吞异常）

- C-138-1：`assertion left == right failed`（写工具 `status="ok"` vs 期望 `"error"`）。
- C-138-4：`assertion left == right failed: session-less context with any Draft must fail closed`（写工具 `ok:true`）。
- C-143-1/2：`panicked ... 更长数字串内的片段不得伪造可重试分类: ... elapsed=40832ms -> TransientRetryable`。
- C-143-3：`assertion failed: !is_rate_limit_error("...code":"429001")`。
- C-144-1：`panicked ... scheme-less host:port 的端口不得污染分类: endpoint=127.0.0.1:42953/v1 -> TransientRetryable`。
- C-145-3：`assertion left == right failed: recovery L1 分类与契约不一致（平行 contains 判据漂移）`。
- C-142-1：断言在当前实现上**不应红**（消除假绿而非改行为），红相证据由 C-142-2 census 承担；实测 POSIX 断言通过。

## 4. 契约对照与偏差

- C-138-1 用例名与 C-138-1 验证命令逐字一致；C-138-3/4 用例名为本清单冻结名（矩阵 [pending] 由本文件转正）。
- PA-138 §0 supersede：`(b)` 仅锁「plan_control 不被 plan_draft_gate 拒绝」；`start_execution`/`complete_step` 审批语义全部留给 C-141（本文件不改写该用例）。
- C-145-3 语料在矩阵 §8 基础上增补 `operation timed out`、`connection reset by peer`、`响应预览: http://127.0.0.1:42953/v1`，用于同时锁「合法命中保留」与「URL 剥离一致」。
- C-143-3 修订后的 provider 测试：`{"message":"inference tpm exhausted","code":"1001"}` 为 true（措辞命中），`{"message":"invalid request","code":"429001"}` 为 false（数字子串不作契约）。
- C-144-1 精确端口 `:429` / `:502` 已在同一测试内覆盖（纯词边界仍漏 ⇒ 必须 host:port 结构剥离）。

## 5. 待办移交（冻结后不属于本文件可改范围）

1. **C-138-2**（executor-a）：删除 `test-session-draft-gate` / `PONY_SESSION_ID` / `unwrap_or("default")` 会话键回退（grep 三处 → 0）。
2. **C-138-5**（需协调）：`wave30b_policy_rules_and_plan_gate_test` 必须改为注入 context，不再依赖生产硬编码会话名；其具体调用形态取决于 executor-a 落地的 evaluator-context seam（建议 `evaluate_with_context(.., &DispatchContext)` 默认委派 `evaluate`）。在 seam 冻结前本测试保持现状（当前 3 passed）。
3. **C-141-1..9**（test-agent，暂缓）：待 executor-b 落 `PlanExecutionApproval` / `DispatchContext.plan_execution_approval` / `TurnInput` 骨架后，由 Lead 通知补 `crates/pony-agent-core/tests/wave36_plan_execution_approval.rs`（编译红合法，需原始 stderr 收据）。
4. **C-144-2**（test-agent，冻结后执行）：变异自证——executor-b 落地 scheme-less host:port 剥离并绿后，测试侧做「临时退回旧判据 → 重跑 C-144-1 必红 → 字节级复原（sha256 比对）→ 复跑绿」的受控 mutation，收据含 `MUTATION_APPLIED` 与红 log；执行须与 executor-b 串行，避免与其 `retry.rs` 生产区写域并发。
5. **C-139-1/C-139-4**（executor-a）：修复后执行 `export RUSTFLAGS="-D warnings" && bash tests/acceptance/remediation/wave36_pa139_full_lib_50x.sh`，50/50 逐轮 log 落 `receipts/wave36/pa139_runs/`。
6. **C-145-1/C-145-5**（executor-b/lead）：删除前消费者普查留证（实测 0 消费者）与 529/500 行为取舍声明。
7. **C-142-2/3**：脚本归 test-agent 独占（矩阵 §14.3）；executor-a 只调用，不得修改。

## 6. 仪器说明

- `wave36_zero_assert_census.sh`：解析 `#[test]`/`#[tokio::test]` 函数体，判据 V1 `ZERO_ASSERTION_TEST`（无断言原语/断言委托/失败原语）、V2 `CFG_ONLY_ASSERTIONS`（全部断言被无 else 的 `if cfg!(…)` 包裹）。断言委托按 `assert_*/check_*/verify_*/expect_*` 命名识别；`.expect()/.unwrap()` 视为失败原语（仅 WARNING `WEAK_ASSERTION_TEST`，不判违规）；`agent::process::tests::helper_process` 为子进程辅助体显式豁免（理由见脚本 `ALLOWLIST`）。`--self-test` 以「替换→落盘→读盘校验」三重自证。
- `wave36_pa139_full_lib_50x.sh`：逐轮 `cargo test -p pony-agent-core --lib`，逐轮 log `pa139_runs/run_NN.log`，汇总 `T-W36-139-01_full_lib_50x.log`；失败轮打印失败用例名 + connect 证据计数并 exit 1；支持 `W36_PA139_ROUNDS` / `W36_PA139_FILTER`（后两者仅冒烟/诊断，不构成验收）。
