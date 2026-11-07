# 契约矩阵：同签名重复工具调用闸（集合 → 计数）

来源：`.agents/notes/proposed/bug-fix/2026-11-07-duplicate-tool-call-gate-retry-false-positive.md`
基线 commit：`ae9312f5`（`cargo test -p pony-agent-core --lib` = 1336 passed，收据 `.dev-team/regression-baseline-wave0-duplicate-gate.json`）
模块前缀：`crates/pony-agent-core/src/agent/runtime/`

## 本轮范围（Lead 裁决）

- **采纳**：note Proposal 第 1 项（计数闸）+ 第 2 项（文案事实化）+ 第 3 项**前半**（写类成功 ⇒ 清空其它签名计数）+ 验收第 7 条（note 迁移 & ADR 0060 链入）。
- **写域标记真值源（复核结论，纠正 note 的假设）**：注册表已有分类 —— `crates/pony-agent-core/src/agent/tools.rs:7944 pub fn tool_kind_for_name(name: &str) -> ToolKind`，内部经 `canonical_tool_name` 归一别名；`ToolKind::{Read,Search,Write,Execute,BatchExecute,Interactive,Composite,External}`（`tools.rs:245`）。**禁止**在闸内另建字符串白名单。
- **延后（登记后续条）**：第 3 项**后半**「只读轮询类给更高阈值（4）/不参与计数」——注册表无「轮询」分类标记（`team_task_list`=Read、`list_agents`=Interactive、`JobOutput`=Execute，无 poll 语义），按 note 自身纪律「先补标记再改闸」，需先扩 `ToolKind`/execution_policy，属独立变更。
- **延后**：第 4 项（软拦截，新增 error code + telemetry 面）。

## C1 limits.rs 限额三件套 + 解析 + 测试覆写

```
DEFAULT_MAX_SAME_SIGNATURE_OCCURRENCES_PER_TURN: usize = 2   // 允许执行次数（第 N+1 次拦）
MAX_ALLOWED_SAME_SIGNATURE_OCCURRENCES_PER_TURN: usize = 8
MAX_SAME_SIGNATURE_OCCURRENCES_ENV = "PONY_AGENT_MAX_SAME_TOOL_SIGNATURE_OCCURRENCES_PER_TURN"
parse_max_same_signature_occurrences_per_turn(raw: Option<&str>) -> usize   // 照抄 limits.rs:160-164 风格
max_same_signature_occurrences_per_turn() -> usize                          // 照抄 limits.rs:141-158 风格（含 #[cfg(test)] 覆写槽）
#[cfg(test)] set_same_signature_occurrences_override(value: usize) -> usize // 照抄 set_same_tool_failure_override
```
非法/越界（0、>8、非数字、空串）一律回退 `DEFAULT`，**禁止取 0**。

## C2 SameSignatureTracker（stream_support.rs，紧邻 ConsecutiveFailureTracker:645）

```
pub(super) struct SameSignatureTracker { counts: BTreeMap<String, usize> }
new() -> Self
occurrence_count(&self, signature: &str) -> usize       // 只读
is_exhausted(&self, signature: &str, limit: usize) -> bool   // count >= limit；只读、不记账
record_executed(&mut self, signature: String, changed_world: bool)  // 唯一记账点
clear(&mut self)                                        // 归零
pub(super) fn tool_execution_changed_world(tool_call: &ToolCall, tool_result: &ToolResult) -> bool
// = tool_result.status != "error" && tool_kind_for_name(&tool_call.name) ∈ {Write, Execute}
```
`record_executed(sig, true)` 语义：**先清空除 `sig` 外的所有签名计数，再把 `sig` 自身 +1**；`record_executed(sig, false)` 只 +1。
**偏离 note 的裁决（Lead）**：**不实现 `from_batch` 预置计数**。note 的 `from_batch` 让首批调用先记 1 次、执行点再记 1 次（双重计数）⇒ 默认 limit=2 时**第 2 次**即熔断，直接违反验收第 3 条（前 2 次执行、第 3 次触发）。故：turn 起始用 `new()`（空计数），计数**只在执行点**发生。

## C3 闸点/记账点（6 处，两路径对称）

| 位置 | 改法 |
|---|---|
| `turn_stream.rs:157` | `let mut same_signatures = SameSignatureTracker::new();`（含为何不预置的注释） |
| `turn_stream.rs:750` | `same_signatures.record_executed(tool_call_signature(&current_tool_call), tool_execution_changed_world(&current_tool_call, &tool_result));` |
| `turn_stream.rs:1328` | `let count = same_signatures.occurrence_count(&next_signature); if same_signatures.is_exhausted(&next_signature, limit) { ... build_duplicate_tool_call_error(&next_tool_call, count + 1, limit) ... }` |
| `turn_sync.rs:46` | 同 turn_stream:157 |
| `turn_sync.rs:293` | `same_signatures.record_executed(tool_call_signature(tool_call), tool_execution_changed_world(tool_call, &tool_result));` |
| `turn_sync.rs:551` | 同 turn_stream:1328 |

不变量：档点不再 `insert`（禁止在被拒绝时记账）；`BTreeSet` 与 `BTreeSet` 导入按需清理（不得留未使用导入，编译器 deny 级别以 `cargo check` 为准）。

## C4 文案（limits.rs，签名改为 3 参）

```
pub(super) fn build_duplicate_tool_call_error(
    tool_call: &ToolCall,
    occurrences: usize,   // 本次被拦的是「第几次」（= 已执行次数 + 1）
    limit: usize,         // 上限（允许执行次数）
) -> String
```
产物必须：① 含「完全相同」；② **不含**「近似」；③ 含实际次数与上限；④ 显式给出比较口径（同名 + 同参数，仅忽略 `description` 与 key 顺序）；⑤ 给三条出路（改参数 / 先确认当前状态 / 直接基于已有证据作答）。

## C5 测试契约（tests.rs 归 Test Agent 独占，Executor 只读）

1. 文案单测：`contains("完全相同") && !contains("近似")`，且含次数与上限文本。
2. tracker 单测：第 1、2 次 `is_exhausted(_, 2) == false`；两次 `record_executed` 后 `is_exhausted(_, 2) == true`；`is_exhausted` 连调多次不改变计数；不同签名互不影响；`clear()` 归零。
3. `parse_max_same_signature_occurrences_per_turn` 单测：`None`/`"0"`/`"9"`/`"abc"` ⇒ 2；`"3"` ⇒ 3。
4. 同步整轮用例 `run_turn_stops_followup_after_consecutive_identical_tool_failures`（tests.rs:5512 起，断言 :5579）：改 4 次请求，并改写 :5574-5578 契约注释。
5. 流式用例 `start_turn_stream_streams_duplicate_tool_recovery_answer`（tests.rs:6802 起，断言 :6902）：补第 3 个同参 tool_call 响应，改 4 次请求。
6. **item 3 覆盖（强制）**：tracker 级 —— `record_executed(read_sig, false)` 后 `record_executed(write_sig, true)` ⇒ `occurrence_count(read_sig) == 0` 且 `occurrence_count(write_sig) == 1`；连续 `record_executed(read_sig, false)` 两次 ⇒ `is_exhausted(read_sig, 2) == true`。
7. **item 3 覆盖（强制）**：`tool_execution_changed_world` 单测 —— `workspace_edit_file`+成功 ⇒ true；`workspace_edit_file`+error ⇒ false；`workspace_read_file`+成功 ⇒ false；`workspace_run_command`+成功 ⇒ true。
8. **item 3 端到端（尽力，冲突则回传 Lead 裁决）**：整轮 `Read(A) → Edit(B) → Read(A)` 同参读两次**不得**触发重复闸（请求数 = 4 且无 recovery 收尾）；`Read(A) → Read(A) → Read(A)` 第 3 次仍触发（=验收 3 同构，已有用例覆盖）。

## C6 门禁

- 红相：test-agent 落盘后 `cargo test -p pony-agent-core --lib` 必须**真实失败**（断言失败/panic，非编译错误），由 Lead 锚定提交。
- 绿相：`cargo test -p pony-agent-core --lib` 全绿（≥1336+新增），加 `--test pa109_goal_acceptance`、`pa110_teams_acceptance`、`pa112_todo_acceptance`、`tool_surface_singularity`、`wave61_team_activation`、`harness_optimizations_test`。
- 可观测性：不新增对外接口/网络 IO/后台任务/LLM 调用面（仅改内部止损判定）⇒ `docs/observability.yaml` N-A，验收跑 `audit-diff.sh` 并记录 Exit 码。
