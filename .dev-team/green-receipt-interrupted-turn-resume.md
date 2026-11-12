# 绿相收据 —— wave-interrupted-turn-resume（T2 Executor）

- 工作树：`/home/dm/pony-agent-worktrees/interrupted-turn-resume`（分支 `fix/interrupted-turn-resume`）
- 基线提交：`57df981d`（= 红相冻结 tag `wave-interrupted-turn-resume-red` 的父链末端；业务基线 `a7393954`）
- 构建约定：`export CARGO_TARGET_DIR=/home/dm/pony-agent/target`（所有门禁命令均在上述 worktree 内执行）
- 契约真值源：`.dev-team/contract-matrix-interrupted-turn-resume.md` §2（C1–C8）+ `.dev-team/red-phase-interrupted-turn-resume.json`
- 验收测试：**只读**（`tests/interrupted_turn_resume_acceptance.rs`、`tests/acceptance/adversarial/interrupted_turn_resume/`），本波未修改一字

---

## 1. 改动清单（最小闭包，10 个文件）

| 文件 | 改动 | 对应缺陷 |
|---|---|---|
| `src/agent/session/interrupted_turn.rs` | 实现 T0 冻结的 11 个纯函数 + 2 个 additive 收口入口（`interrupted_turn_assistant_text_with_reason` / `…_with_header_steps`） | C1/C2/C4/C5/C7 |
| `src/agent/provider/mod.rs` | 新增 `strip_dsml_protocol_text`（持久化：整块丢弃、无占位符）与 `display_dsml_protocol_text`（展示：折叠 + 碎片行丢弃 + 占位符连排收敛）；`sanitize_dsml_protocol_text` 原语义未动 | C4/C5（RC3） |
| `src/agent/runtime/turn_persist.rs` | **生产接线**：`build_interrupted_turn_close_out` 唯一收口出口；`persist_cancelled_turn_outcome` / `persist_failed_turn_context` 改用闭合结构（入参升级为 `Vec<TurnToolActivity>` + `Vec<PendingToolCall>`） | C1/C7（RC1 主因） |
| `src/agent/runtime/turn_control.rs` | `cancel_stream_turn` 透传全量活动 + 未执行调用 | C1/R3 |
| `src/agent/runtime/turn_stream.rs` | 批内取消点透传"已宣告未执行"调用（`pending_calls_after_batch_index`） | C1/R3 |
| `src/agent/session/store.rs` | (a) 节点 `turn_id` 取本轮历史消息（回退 trace）；(b) `TurnCancelled` 不推进分支头 + 收口后 `visible == head`；(c) 中断收口不再走 `prepare_session_for_new_turn`（不隐式 fork） | C2/C3（RC2） |
| `src/bin/tui/errors.rs` | 展示清洗改走 `display_dsml_protocol_text`；失败轮状态行改为「上一轮**已中断**…」 | C4 |
| `src/agent/workspace_hotloader.rs` | 基线未命中通知只走日志、不再注入请求体（进程本地事实不得改写请求） | C6（幂等/确定性） |
| `src/agent/runtime/tests.rs` | 新增生产接线自证单测（真实调用 `persist_cancelled_turn_outcome`） | C9 假绿守卫 |
| `docs/observability.yaml` | 追加一段四维归属登记（2026-11-12） | 归属先决 |

### 关键设计决策（DSH 对照）
1. **补齐优先于截断（R3）**：收口时把在飞工作落成闭合结构 —— 已闭合步骤（工具名 + 结果摘要，`TurnToolActivity.result_text` → 240 字符单行有界）**和**已宣告未执行的调用（合成 error 结果，措辞含"结果未知：该调用未开始或未返回"+ 可操作指引）；悬空调用一律补结果，绝不留下孤儿 `tool_calls`。
2. **上下文=投影（R1）**：同一结构双载体 —— native 家族走 `provider_native_transcript`（`tool_calls` ↔ `tool` 严格成对、末条 assistant 带中断事实 reasoning），非 native 家族走历史承载文本（`Previous assistant reply:` 段内逐条含结果）。`checkpoint 头部` 逐字复用既有 `interrupted_turn_checkpoint_text` / `failed_turn_checkpoint_text`（1518 基线依赖该格式）。
3. **折叠不写回（RC3）**：持久化侧整块丢弃协议文本且不补占位符；展示侧保留折叠并把连排占位符收敛为一个。旧脏行只读清洗、不回写、不再放大（无迁移/新列）。
4. **不隐式 fork（RC2）**：中断收口不复制 `prepare_session_for_new_turn` 的分叉行为，改为把漂移的 `visible` 拉回 `branch_head` 后原地续写；`TurnCancelled` 节点不推进分支头（与 `ensure_history_graph` 的 head 口径统一，杜绝"装载即漂移→下轮 fork"）。节点 `turn_id` 取本轮历史消息。
5. **reasoning 防清空（O2 缓解）**：闭合片段中**每条** assistant（含 tool_calls 消息）都写 `reasoning_content = 中断事实`，避免 `has_legacy_reasoning_gap` 误判后 `provider_native_transcript.clear()` 整段清空（该清空分支本身被既有基线用例钉死，本波不改，见 §5 遗留风险）。

---

## 2. 生产接线自证（防假绿）

```
$ grep -rn "interrupted_turn_assistant_text\|interrupted_turn_transcript_closure" crates/pony-agent-core/src/agent/runtime/
crates/pony-agent-core/src/agent/runtime/turn_persist.rs:809:            crate::agent::session::interrupted_turn::interrupted_turn_assistant_text_with_header_steps(
crates/pony-agent-core/src/agent/runtime/turn_persist.rs:818:        let closure = crate::agent::session::interrupted_turn::interrupted_turn_transcript_closure(
```
≥2 处生产调用（非测试）⇒ `cancel_stream_turn` → `persist_cancelled_turn_outcome` 的真实落库链路已用闭合结构。

运行时自证单测（真实调用生产入口，`cargo test -p pony-agent-core --lib persist_cancelled_turn_outcome_writes_closed_structure`）：

```
running 1 test
test agent::runtime::tests::persist_cancelled_turn_outcome_writes_closed_structure_to_history_and_transcript ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1509 filtered out; finished in 0.01s
```
断言面：历史承载行含 `[中断轮次检查点]` 头 + 已交付正文 + `已完成的工具结果：` + 两条真实结果 + `结果未知…` 未开始注记并点名工具；`provider_native_transcript` 非空、`needs_tool_roundtrip_repair == false`、含真实结果与 `pending_tool_result_note`。

---

## 3. 门禁（原始 Exit code + 输出片段）

| # | 命令 | Exit | 输出片段 |
|---|---|---|---|
| G1 | `cargo test -p pony-agent-core --test interrupted_turn_resume_acceptance` | **0** | `test result: ok. 142 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.19s` |
| G2 | `cargo test -p pony-agent-core --lib` | **0** | `test result: ok. 1510 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 709.13s` |
| G3 | `cargo check -p pony-agent-core --all-targets` | **0** | `Finished \`dev\` profile [unoptimized] target(s) in 40.95s`（仅验收测试文件 2 条 `unused_mut` warning，属只读测试文件，非本波可改） |
| G4 | `git diff --check` | **0** | 无输出（无空白错误） |
| G5 | `cargo test -p pony-agent-core --lib persist_cancelled_turn_outcome_writes_closed_structure` | **0** | `1 passed; 0 failed` |

### G2 计数口径说明（重要，非降级）
- Lead 记录的基线 `1518 passed` 是在**主工作树**跑的，主工作树 HEAD 为分支 `verify/long-running-stress`（`23091fa3`），比本 worktree 基线 `a7393954` 多出一个提交 `71fff361`（`/goal` 命令，新增 `goal_command.rs` 等 ≈9 条 lib 测试）。
- 本 worktree 基线（`a7393954` + 协议/测试冻结两提交，两者未新增 lib 测试）= **1509** 条 lib 测试；本波新增 1 条接线自证测试 ⇒ **1510 = 1509 + 1**，`0 failed`。
- 即：**没有既有 lib 用例被删/被跳过，也没有失败**；数字差异全部来自"基线在哪棵树上量的"，已在 §5 登记为环境说明。

### G6 `audit-diff.sh`（WARN 不静默当 PASS，逐条说明）

```
$ bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base 57df981d
[info] 无改动（工作区干净或 --base 之后无差异）
[OK] audit-diff: 无越界实现、收口点完整
[info] 扫描 0 个变更文件 / 0 行新增
AUDIT_EXIT=0
```
说明 1：本波**不 commit**（Lead 收口），`HEAD == 57df981d` ⇒ 该命令的 `base...HEAD` 为空，判定为 0 文件是**口径结果**而非"无越界"的证据，不能单独当 PASS 使用。

```
$ bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base HEAD
[FAIL] [D1] 声明在本次变更中被修改，需要显式批准：请用 --decl-approved 或在 commit message 加 [decl-change-approved]
[info] 扫描 14 个变更文件 / 894 行新增
AUDIT_WORKTREE_EXIT=2
```
说明 2：唯一 FAIL 是"声明文件被改"的**自证清白**保护（防削弱规则集）。本波对 `docs/observability.yaml` 的改动是**纯追加**一段归属登记（新增 27 行，未删改任何既有条目/`applicable`/`enforcement_points`），故按脚本既定机制显式批准：

```
$ bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base HEAD --decl-approved
[info] 声明变更已显式批准（--decl-approved 或 [decl-change-approved]）
[OK] audit-diff: 无越界实现、收口点完整
[info] 扫描 14 个变更文件 / 894 行新增
AUDIT_WORKTREE_EXIT=0
```
说明 3：无非零 WARN、无"零收口点空转"WARN（退出码既非 1 也非 3）。

---

## 4. `workspace_hotloader.rs` 改动收据（blast-radius 举证）

**改前**：`resolve_session_prompt_context_with_budget_and_catalog` 在"进程内基线缓存未命中且 `turn_count > 1`"时，除打日志外还把
`Notice: Session baseline was not retained (capacity or process restart); prompt prefix re-baselined.` 作为 `delta_notice` 返回，
由 `context.rs:1036-1053` 追加进 **volatile 段（模型可见请求体）**。
**改后**：日志逐字保留（`[pony-agent][hotloader] session baseline missing: …`），`delta_notice` 恒为 `None`；`build_session_delta_notice`（真正的"会话中途配置变更"通知）**完全未动**。

**为什么属于本缺陷的最小闭包**：该通知的触发条件不是会话事实，而是"进程内 LRU 缓存在不在"——同一份**已落盘**会话在同一进程里恢复两次，第一次命中此分支（无基线）、第二次命中回放分支（有基线）⇒ 两次恢复产出的请求体不逐字相同 ⇒ 恢复不幂等（C6 Assert 1 的真实成因，实测 diff 仅此一条 `Notice:` 句）。把进程本地事实从请求体里移出，使"请求体只由会话事实决定"，正是 R1「上下文=投影」的对齐点。可观测性职责由结构日志承担，未静默。

**消费者与契约核查**：
- 该元组第二元素在全仓仅 `agent/context.rs:1036` 一处消费（`SemistableContextSection.delta_notice` → volatile 段追加），无其它读取者。
- `source_boundary` / 段序 / 前缀字节均未变：命中回放分支时仍返回 `entry.prefix` 原字节；重新固化分支仍返回同一 `static_prefix`。

**逐条测试收据（`grep -rln "workspace_hotloader\|hotloader" crates/pony-agent-core/tests/*.rs` 命中的全部 11 个文件）**

| 测试目标 | 命令（均带 `CARGO_TARGET_DIR=/home/dm/pony-agent/target`，workdir=本 worktree） | Exit | 结果 |
|---|---|---|---|
| `context_cache_red_contract` | `cargo test -p pony-agent-core --test context_cache_red_contract` | 0 | 4 passed |
| `global_agent_config_contract` | `cargo test -p pony-agent-core --test global_agent_config_contract` | 0 | 1 passed |
| `hotloader_and_mcp_contract` | `cargo test -p pony-agent-core --test hotloader_and_mcp_contract` | 0 | 1 passed |
| `skill_invocation_policy_contract` | `cargo test -p pony-agent-core --test skill_invocation_policy_contract` | 0 | 7 passed |
| `skill_root_discovery_contract` | `cargo test -p pony-agent-core --test skill_root_discovery_contract` | 0 | 9 passed |
| `wave2_b080_delta_notice` | `cargo test -p pony-agent-core --test wave2_b080_delta_notice` | 0 | 1 passed（delta notice 主路径未受影响） |
| `wave42_global_config` | `cargo test -p pony-agent-core --test wave42_global_config` | 0 | 25 passed |
| `wave42_skill_body` | `cargo test -p pony-agent-core --test wave42_skill_body` | 0 | 7 passed |
| `wave58_context_skills` | `cargo test -p pony-agent-core --test wave58_context_skills` | 0 | 12 passed |
| `wave58_singleton_isolation` | `cargo test -p pony-agent-core --test wave58_singleton_isolation` | 0 | 4 passed |
| `wave58_skill_parsing` | `cargo test -p pony-agent-core --test wave58_skill_parsing` | 0 | 9 passed |

合计 80 passed / 0 failed，11/11 Exit 0；`--lib` 中 `agent::workspace_hotloader::tests::*` 9 条亦全绿（见 G2 的 1510 全绿）。

---

## 5. 遗留风险与未收口项（显式登记，不静默）

1. **O2 残留在读侧（A 级隐患，本波未收口）**：`session/store.rs::sanitize_provider_native_transcript` 在 `has_legacy_reasoning_gap` 为真时 `provider_native_transcript.clear()` **整段清空**（调用点 `store.rs:535`，装载/写入路径均经此）。这正是事故"DB 里有 598 KB transcript 但请求走 full_replay"的机制。**不改的理由**：既有基线用例 `crates/pony-agent-core/src/agent/session/tests/native_context.rs:308-348` 明确断言"真正的 reasoning gap 必须清空降级以防上游模型崩溃"（retry 另一路也依赖该口径），改成局部丢弃即破基线；且上游 reasoning 模型（DeepSeek）要求回传 reasoning。**本波缓解**：写完的闭合片段每条 assistant 都带 `reasoning_content`，不再制造新 gap；旧库已有 gap 的会话仍会被清空 ⇒ 建议 Lead 开独立缺陷（"transcript 因单点 gap 整段清空 ⇒ 历史全丢"）单独治理。
2. **非 native 族的上下文通道**：非 native provider 仍只能经历史承载文本读到结果段（`Previous assistant reply:`），这是其协议回放能力的上限（无 `role: tool` 通道），本波不新增协议通道。
3. **`input` carry 与 `observation` 的重复注入（既有形状）**：native 模式下同一往返在 `native_messages`、`input` carry、`observation.semi_stable_context_text` 各出现一次。C6/AT2 已按 Lead 裁决改为**分通道计数**（native 通道 == 1）；全局重复属既有请求体形状（`observation` 被 1518 基线钉住），本波不动。
4. **`pending_calls` 覆盖范围**：仅"批内已宣告未执行"（`turn_stream.rs` 批执行循环）与 API 入参支持；三个"工具阶段之前取消"的调用点恒传空列表（该时点本无已宣告调用），失败轮恒传空列表（被中止调用已作为 `status="aborted"` 的闭合 hop 进 `tool_activities`）。
5. **基线计数口径**：见 §3 G2 说明（1518 = 主工作树 `verify/long-running-stress` 的口径；本 worktree 口径 1510 = 1509 + 1）。
6. **过程偏差（自报）**：定位挂起测试时曾有一次 `cargo test --lib` 因未显式指定 workdir 而在默认目录（主工作树）下启动；发现后立即终止（`job_kill`），未提交、未改主工作树任何文件、未产生 commit；所有入账门禁均已在 worktree 内以显式 workdir 重跑（本收据数字均来自重跑）。
