# PA-171: TUI 连环错误根因修复 —— goal 续轮失败止损 / provider 限流冷却 / 失败轮归因

## 状态

- **状态**: 进行中（wave-68，W1 契约冻结中；阶段进度以 `.dev-team/state.json` → `waves[wave=68]` 为准）
- **优先级**: P1
- **Complexity**: B（跨 tui / control_plane / goal / provider / retry / turn_flow 多模块 + LLM 调用面）
- **创建日期**: 2026-11-09
- **诊断来源**: TUI 会话 `pa_01a1ba70-a314-7712-adf2-4bf675a96f99`（trace 维度审核 / trace 审核 loop），前段 `pa_01a1bae5-8b23-7dd2-8384-37dd3cdbdf3d`
- **决策依据**: ADR 0103（本卡同提交落盘）、ADR 0092（同类「续轮无让位」教训）

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

## 断点续跑指引（后续会话入口）

1. 读本卡 → 读 `.dev-team/state.json` 的 `waves[wave=68]`（含 `baseline_commit` / `protocol_commit` / `red_commit` / 各分支 `status`）。
2. 红相未锚定：从 `t-red` 继续（Test Agent + `test-expert`）；已锚定：按分支状态续跑对应 Executor。
3. 语义真值源：本卡 §交付契约（C1–C6）与 ADR 0103；契约变更必须走契约演进门禁，不得就地改。
4. 唤起词：**「继续 PA-171」**。
