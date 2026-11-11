# 契约矩阵：ADR 0123 补充修正（wave-adr0123-supp）

来源：会话 `pa_01a1cb08-f340-7020-83a2-3a9b2d62df29` 诊断结论（trace 见 session.jsonl seq71/73/83/85）。
基线提交：`27da736d`（ADR 0123 已实施）+ `d5fa1673`。

## 冻结契约（任何实现不得偏离；偏离须先改本文件并由 Lead 冻结）

| ID | 契约 | 真源位置 | 变更类型 |
|---|---|---|---|
| C1 | 发送给 provider 的 native transcript 必须**配对闭合**：任一 `assistant.tool_calls` 之后必须紧跟匹配 `tool_call_id` 的 `tool` 消息；无主 `tool` 消息必须丢弃。该不变量在**截断之后**仍成立。 | `agent/context_window.rs` `truncate_native_messages` | additive（内部不变量） |
| C2 | 截断路径的修复必须复用唯一状态机 `tool_pruner::repair_and_filter_tool_roundtrips`，禁止新建第二套补齐逻辑。 | 同上 | additive |
| A1 | 新增 `pub fn failed_turn_checkpoint_text(prior_assistant_text:&str, tool_activities:&[String], failure_reason:&str) -> Option<String>`：在既有检查点文本基础上追加一行 `失败原因：<reason>`；`reason` 为空则与既有 `interrupted_turn_checkpoint_text` 逐字一致；前缀必须是 `[中断轮次检查点`。 | `agent/session/store.rs` | additive（新 pub API） |
| A2 | 失败轮（`persist_failed_turn_context`）一律使用 A1 产出的文本；**不得**因为存在工具步骤就丢弃 `error`。 | `agent/runtime/turn_persist.rs` | 行为修正 |
| D1 | 新增 `pub fn is_interrupt_placeholder_text(text:&str) -> bool`：识别 `[中断轮次检查点`、`（本轮上游调用中断`、`（本轮由用户终止` 三类中断注记。 | `agent/session/store.rs` | additive（新 pub API） |
| D2 | `has_legacy_reasoning_gap` 的两处豁免判定一律改用 D1，禁止整段 `clear()` 误杀中断轮 transcript。 | `agent/session/store.rs` | 行为修正 |
| B1 | 0 步被取消轮次的用户消息必须可进入折叠历史（或给出无法修复的定位报告 + backlog 待办）。 | `agent/runtime/turn_control.rs` / `turn_stream.rs` | 待定（先定位） |

## 不变量（回归红线）

1. `[中断轮次检查点]` 前缀与「已执行的工具步骤：」段落格式不得改变（TUI 逐字回放，`store.rs:4127`、`bin/tui/errors.rs:244` 依赖）。
2. 既有 `interrupted_turn_checkpoint_text` 签名与行为不得改变。
3. 非 reasoning 会话的 transcript 清洗行为不变（不得新增清空路径）。
4. 截断 token 预算语义不变：`reserved >= budget` 仍返回空 transcript（HEAD 既有）。

## NFR 归属

- 外部 IO / 网络超时：**N/A** — 本次仅内存消息装配与本地会话日志投影，无新增网络/外部调用点。
- 可观测性：沿用 `docs/observability.yaml`（ADR 0123 已登记），不新增对外接口。
- 并发：截断与修复为纯函数，无共享可变状态。

## Write scopes（互斥）

- exec-context: `crates/pony-agent-core/src/agent/context_window.rs`
- exec-turn: `crates/pony-agent-core/src/agent/runtime/turn_persist.rs`、`runtime/turn_control.rs`、`runtime/turn_stream.rs`、`session/store.rs`
- test-agent: `crates/pony-agent-core/tests/adr0123_supplement_test.rs`（新建，只此一个文件）
- protocol-agent: `.agents/notes/implemented/architecture/`、`.dev-team/`
- Lead: `.dev-team/`、git 写操作（唯一写主）
