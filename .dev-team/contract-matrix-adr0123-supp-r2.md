# 契约矩阵 v2：ADR 0123 补充——审查缺陷修复（wave adr0123-supp-r2）

来源：L3-A（并发/幂等）+ L3-B（契约/格式）对 `a5634316` 的审查判定。
基线：`bd4c8de1`（工作树 clean）。上一轮元复盘：`.dev-team/meta-retrospective-adr0123-supp.md`。

## 冻结契约

| ID | 契约 | 真源 | 变更类型 |
|---|---|---|---|
| B2-1 | `failed_turn_checkpoint_text` 中「失败原因」行必须 **① 置于「已执行的工具步骤：」段之前**；**② 换行/连续空白折叠为单空格**；**③ 截断至 ≤200 字符（字符安全，超限加 `…`）**。reason 空时输出与 `interrupted_turn_checkpoint_text` **逐字一致**。 | `agent/session/store.rs` | 行为修正 |
| B2-2 | 折叠/截断不得改变 `[中断轮次检查点]` 前缀与「已执行的工具步骤：」段格式（TUI 逐字回放依赖）。 | 同上 | 不变量 |
| D1-1 | 两个中断注记常量（上游中断 / 用户终止）迁至 `agent/session/store.rs` 为 `pub const UPSTREAM_INTERRUPTED_NOTE` / `pub const USER_CANCELLED_NOTE`，`agent/runtime/tool_recovery.rs` 改为引用（删除本地副本）。 | `agent/session/store.rs` + `agent/runtime/tool_recovery.rs` | 单一真源 |
| D1-2 | `is_interrupt_placeholder_text` 判定改为：`trim().starts_with("[中断轮次检查点")` **或** 与上述两个常量**整句相等**。禁止裸子串匹配（正文引用不算注记）。 | `agent/session/store.rs` | 行为修正 |
| A1-1 | `turn_outcome_persisted` 改为**单值真源**：历史消息必须携带 `turn_id`（`append` 路径透传），gate 只认「历史腿」。**删除 trace 腿**（watchdog 写 trace 不写历史，trace 腿会让 gate 恒真）。 | `agent/runtime/turn_control.rs` + `agent/session/store.rs` + `agent/runtime/turn_persist.rs` | 行为修正（breaking 内部契约） |
| B1-1 | 事件重投影必须与写路径同源：`agent/projection.rs` 的检查点文本改为经 `failed_turn_checkpoint_text`（携带 reason 与 `name + description` 工具行），保证 `fold_node_views` / 启动重建不丢失败原因。 | `agent/projection.rs` | 行为修正 |

## 不变量（回归红线）

1. 老会话（无 turn_id、无失败原因行）读回行为不变：`is_none` / 逐字委托路径不变。
2. `sanitize_provider_native_transcript` 对真实 reasoning 空洞的清空能力不得被削弱（D1 只收紧判定，不放开清理）。
3. terminal-once 语义不变：`claim_terminal_status` 仍与 `stop_escalation::escalate_now` 抢同一位，且任何路径不得派发第二条终态。
4. 既有测试（`adr0123_supplement_test.rs` 6 例、`multi_turn_tool_context_test.rs` 3 例）必须继续全绿。

## 验收要求（每项必须有断言）

- B2：reason 含 `\n`/HTML 多段 → 折叠为单行、位置在工具段之前、超长截断到 200 且带 `…`。
- D1：正文引用「（本轮由用户终止」→ false；三类注记（检查点前缀 / 两个常量整句）→ true。
- A1：watchdog 场景（trace 有行、history 无消息）⇒ gate 返回 false 并触发回填；in-band 正常取消 ⇒ gate 返回 true 不重复写。
- B1：重投影（`fold_node_views` 或等价入口）产出的检查点文本包含失败原因且工具行带 description，与写路径逐字一致。

## Write scopes（互斥）

- exec-session: `crates/pony-agent-core/src/agent/session/store.rs`、`crates/pony-agent-core/src/agent/runtime/tool_recovery.rs`
- exec-gate: `crates/pony-agent-core/src/agent/runtime/turn_control.rs`、`crates/pony-agent-core/src/agent/runtime/turn_persist.rs`
- exec-proj: `crates/pony-agent-core/src/agent/projection.rs`
- test-agent: `crates/pony-agent-core/tests/adr0123_supplement_r2_test.rs`（新建，唯一）
- Lead: `.dev-team/`、`session/mod.rs` re-export（公共接线）、git 写操作
