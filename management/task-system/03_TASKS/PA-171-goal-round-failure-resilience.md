# PA-171: TUI 连环错误根因修复 —— goal 续轮失败止损 / provider 限流冷却 / 失败轮归因

## 状态

- **状态**: **已实施（wave-68 / a1 已合入主干）**，收尾项见文末「遗留与后续」
- **优先级**: P1
- **Complexity**: B（跨 tui / control_plane / goal / provider / retry / turn_flow 多模块 + LLM 调用面）
- **创建日期**: 2026-11-09
- **诊断来源**: TUI 会话 `pa_01a1ba70-a314-7712-adf2-4bf675a96f99`（trace 维度审核 / trace 审核 loop），前段 `pa_01a1bae5-8b23-7dd2-8384-37dd3cdbdf3d`
- **决策依据**: ADR 0103（本卡同提交落盘）、ADR 0092（同类「续轮无让位」教训）
- **落地提交**: 红相 `e8c460ab` · 契约 `e1227763` · t-w2 `d04d543c` · 文档/ADR `59ec6797`+`1f5610d6` · t-w1+t-w3 `12a155b5` · a1 对抗红相 `414c276e` · a1 修复 `d8f08e8d`
- **账本与收据**: `.dev-team/state-wave68.json`（决策 D68-1..D68-11）、`contract-matrix-pa171.md`（含 amendment a1）、`nfr-baseline-wave68.json`、`pa171-red-receipt.json`、`regression-baseline-wave68.json`、`review-*-wave68-v*.json`、`evidence-wave68/gates.md`

## 需求背景（诊断结论，已取证）

用户报告：TUI 里那条"按 trace 做维度审核与优化"的会话后期出现连续错误。取 `~/.local/share/PonyAgent/sessions.db`
（`session_turn_traces` / `turn_events`）后得到完整时间线（2026-11-09）：

| 时间 | 现象 | 证据 |
| :-- | :-- | :-- |
| 10:13–10:26 | 5 轮均在跑到 `CALL MODEL #52–#92` 时被 **45s 总超时**杀死（sync POST 路径） | `elapsed=45002ms`，`provider/mod.rs:2521` |
| 10:28:51 | 429 `exceeded_current_quota_error`：Kimi 5 小时限额，06:01 UTC 恢复 | 响应正文 |
| 10:29:08–10:30:22 | `goal-8..goal-20` **16 轮连败**，每轮首个模型调用即 429，`turn/end reason=error`、`duration≈0` | turn_events 连续 16 条 |
| 10:30:22 | `roundsStarted=20/20` 预算耗尽 → 静默停；goal 仍 `active/Armed`，横幅只显示"进行中 [20/20]" | `goal_store.json`、`tui.rs:8146` |
| 10:43 | 才恢复（换模型后 `providerMode=live`） | traceTimeline |

**三层根因**

1. **触发（外部）**：单 provider（magpie→kimi/k3.1-pro）先 45s 超时、后配额 429；sync 客户端 `.timeout(45s)` 为整请求总时限（`provider/mod.rs:2521`），首跳失败只对同一 provider 重试 ≤5 次（`provider/mod.rs:4401`），**无跨 provider 备用链**（全仓 fallback 仅 `provider_followup_local_fallback`，只兜工具 followup），无冷却/熔断，`Retry-After` 为死码（`retry.rs:270-293` 零生产调用点）。
2. **放大（主因，harness 缺陷）**：`run_goal_rounds`（`bin/tui.rs:8127-8178`）只判 Esc / 8 轮 / 待作答请求，**不看上一轮成败**；`control_plane/mod.rs:2145` 把 `Result` 丢弃（`start_turn_stream` 返回 `()`）；`consecutive` 是线程局部计数而 `rounds_started` 是跨线程全局计数（`goal.rs:380`，失败轮照样 +1）⇒ 429 被"重放"，且多轮次线程各持一份循环 ⇒ **16 = 2×8**、轮号乱序（8→5）、20 轮预算 74 秒烧光。
3. **观测（放大可见性）**：失败轮 `provider_source/mode/fallback_reason` **硬编码 None**（`turn_flow.rs:581-583`）；TUI 把 429 折叠成"上游服务暂时不可用，重试已用尽"（`bin/tui/errors.rs:76/147`）⇒ 用户看不到"配额耗尽"，只剩满屏红字。

**噪声（非根因，不在本卡范围）**：`invalid_path` 14 次（模型猜回执文件名）、`non_zero_exit` 12、`fs_not_observed` 4。

**复现查询**（只读，供后续会话复核）：

```bash
python3 - <<'PY'
import sqlite3,json
c=sqlite3.connect('file:/home/dm/.local/share/PonyAgent/sessions.db?mode=ro',uri=True)
sid='pa_01a1ba70-a314-7712-adf2-4bf675a96f99'
for tid,td in c.execute("select turn_id,trace_data from session_turn_traces where session_id=? order by updated_at_ms",(sid,)):
    d=json.loads(td); errs=[t['error'] for t in (d.get('traceTimeline') or []) if t.get('error')]
    print(tid, d['phase'], len(errs))
PY
```

## 目标

把"一次上游限流"收敛为**一次可见失败**：续轮在连续失败后停止并说明原因，失败轮不消耗目标预算，限流期间快速失败并冷却，
失败在 trace 与界面都可归因。

## 交付契约

- **C1 续轮失败止损**：同一续轮循环内连续 2 轮以 `failed` 终态结束即停止新增 goal 轮，并把停止原因写入 TUI 可见状态行（不是静默 break）。
- **C2 失败不烧预算**：只有**成功发起且未以失败终态结束**的轮次才计入 `rounds_started`；预算耗尽时界面必须显示"已停止（原因）"而非"进行中"。
- **C3 限流快速失败 + 冷却**：`429/rate_limit/quota` 分类后不再走 500ms→8s 短退避打 5 次；命中后 provider 进入冷却，冷却期内直接返回带 `Retry-After`/恢复时间的可读失败，不发起 HTTP。
- **C4 失败轮归因真值**：失败轮 trace 的 `provider_source` / `provider_mode` / `fallback_reason` 落真实值（不再是硬编码 None），并新增 `error_class`（timeout / rate_limited / quota / auth / network / other）供审计机判。
- **C5 错误文案如实**：quota/限流失败在 TUI 显示真实原因（含恢复时间），禁止折叠为"重试已用尽"。
- **C6 观测归属**：`docs/observability.yaml` 更新失败语义归属与收口点；`audit-diff.sh` / `check-declaration.sh` Exit 0。

## 非目标

- **跨 provider 自动故障转移**（需 `providers.json` 增候选链 + 新 ADR，本卡只登记为 PA-172 候选）。
- 重构 provider 层重试框架 / 45s 硬编码改配置化（登记 PA-172 候选）。
- 审计脚本回执命名与 `invalid_path` 噪声治理（独立事项）。

## 波次与写域（wave-68，DAG）

| 任务 | 内容 | write_scopes |
| :-- | :-- | :-- |
| `t-red` | 红相验收测试（Test Agent，`test-expert` 双模态） | `crates/pony-agent-core/tests/` |
| `t-w1` | C1+C2：续轮失败止损、失败不烧预算 | `crates/pony-agent-core/src/bin/tui.rs`、`.../agent/control_plane/mod.rs`、`.../agent/goal.rs` |
| `t-w2` | C3+C5：限流分类/冷却、错误文案 | `.../agent/provider/mod.rs`、`.../agent/retry.rs`、`.../bin/tui/errors.rs` |
| `t-w3` | C4：失败轮 trace 归因 | `.../agent/turn_flow.rs`、`.../agent/runtime/turn_stream.rs`、`.../agent/runtime/turn_persist.rs` |

`t-w1/w2/w3` 写域两两不相交，可在 `t-red` 锚定后并行；`t-red` 串行前置（`blocked_by`）。

## 验收（物理收据）

1. `cargo check -p pony-agent-core --all-targets` Exit 0。
2. `cargo test -p pony-agent-core --test tui_batch2_regression --test tui_session_resilience_acceptance` Exit 0（红相 → 绿相）。
3. 新增断言：429 连败 ≥2 轮后续轮停止（用例以假 provider 注入 429，不依赖网络）。
4. `ratchet`/`goal`：失败轮后 `rounds_started` 不增；预算耗尽状态行含原因。
5. 失败轮 trace 三字段非 null 且 `error_class` 与错误文本一致。
6. `bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base <基线>` 与 `check-declaration.sh <repo>` Exit 0。

## 落地结果（物理收据，2026-11-09）

| 门禁 | 结果 |
| :-- | :-- |
| `cargo check -p pony-agent-core --all-targets` | Exit 0 |
| `pa171_goal_round_failure_contract`（C1/C2） | 7 passed |
| `pa171_provider_rate_limit_cooldown_contract`（C3） | 5 passed（loopback mock 计数：限流 1 次 HTTP、冷却期 0 次、503 仍重试） |
| `pa171_failed_turn_attribution_contract`（C4） | 3 passed |
| `pa171_error_copy_contract`（C5） | 89 passed |
| `pa171_error_copy_adversarial`（a1 对抗） | 89 passed（红相 101 → 绿相） |
| `tui_batch2_regression` / `tui_session_resilience_acceptance` | 96 / 98 passed（零回归） |
| `cargo test -p pony-agent-core --lib` | 1404 passed |
| `ts_contract_snapshot` | 1 passed |
| Tier-1 冒烟 | `ponycli` build Exit 0，`--help` Exit 0 |
| 观测门禁 | `audit-diff.sh` Exit 0；`check-declaration.sh` PASS |

现场对照：修复后 `run_goal_rounds` 在同一会话内**最多连续 2 轮失败即停并写明原因**；失败轮不推进
`rounds_started`；限流命中后进入冷却并在冷却期**不发 HTTP**；失败轮 trace 的
`providerSource/providerMode/fallbackReason/errorClass` 可机判（`pa_01a1ba70` 的 16 连败形态不再可能）。

## 遗留与后续（本轮 ADVISORY，已登记未实施）

1. **跨 provider 自动故障转移 + 45s/5 次/30s 预算配置化** → 建议新开 PA-172（需 `providers.json` 候选链与新 ADR）。
2. `errorClass` 未进 TS 契约快照门禁（`tests/ts_contract_snapshot.rs` 的 `serdeKeys` 不含该键，前端类型亦未声明）。
3. 冷却注册表 4 处 `into_inner()` 为 **fail-open**（锁中毒后继续放行），需在契约显式声明或改 fail-closed。
4. `clear_cooldown` / `cooldown_remaining` 零生产调用点 ⇒ 冷却复位只能靠进程重启（6h 上界兜底）。
5. 冷却键 `a|b|c|d` 未转义/未归一（`|` 可碰撞、大小写/尾斜杠变体可绕过）。
6. `bin/tui/errors.rs` 的 `now_unix_ms` 为死参（无法据此算剩余窗口）。
7. `GoalRoundLoopLease` 是**进程级单 bool**（非按 session 分区）：多会话并发续轮会互相阻塞。
8. `turn_loop_in_flight` 为单 bool，轮次线程提前 return 时可清掉仍由另一线程持有的在飞语义。
9. 续轮预算推进落盘失败分支复用 `BudgetExhausted` 文案（真实原因是持久化失败）。
10. `TuiState.goal_rounds_stopped` 只写不读、`reset_session_scope` 未清（跨会话残留）。
11. 租约与 `start_turn_stream_checked` 五臂归约缺可执行用例（`--list` 0 命中；L3-A v3 建议下波补锚点）。
12. `GoalRoundLoopLease` 的类型 doc 首句仍写「`Arc<AtomicBool>` 的 compare-exchange」（a2 已改为
    `HashSet<String>` 按会话分区）——注释陈旧，非语义问题，随下波一并校正。
13. `UiEvent::GoalRoundsStopped` 只带 reason/status_line，**不带 session_id**：按会话分区后，
    非当前会话的停轮上报可能落进当前会话消息区（pre-existing 面，本轮因"跨会话互不阻塞"而多一条可达路径）。
14. a2 的计划复审（`review-plan-wave68-v2.json`）由 Lead 定点复核替代（Subagent 未产出）：
    证据 = `tui.rs:8331-8372`（分区租约 + Drop 释放）、`tui.rs:8160-8180`（抢不到即可见上报）；
    **如实登记为 Lead 复核，非独立审查收据**。

## 断点续跑指引（后续会话入口）

1. 读本卡 → 读 **`.dev-team/state-wave68.json`**（本波次账本；共享 `.dev-team/state.json` 由并行会话并发使用，
   故本波次单列，沿用 wave16/17/61 先例），含 `baseline_commit` / 各分支 `status` / `decisions` D68-1..D68-11。
2. 语义真值源：本卡 §交付契约（C1–C6）+ `.dev-team/contract-matrix-pa171.md`（含 amendment wave-68-a1）+ ADR 0103；
   契约变更必须走契约演进门禁，不得就地改。
3. 若要继续收遗留项：从 §遗留与后续 第 1 条起，按 dev-team 新建波次（勿复用 wave-68 账本）。
4. 唤起词：**「继续 PA-171」**。
