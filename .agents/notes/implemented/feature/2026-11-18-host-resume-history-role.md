# Agent Note: 宿主续轮历史行改用 host 角色 + 组合正文派生识别

Status: implemented

工单：B235 / B238。影响文件：`agent/session/host_resume.rs`（新）、`agent/session/store.rs`、
`agent/session/types.rs`、`agent/context.rs`、`agent/context_window.rs`、`bin/tui.rs`、
`session/host_resume_tests.rs` / `agent/context/host_history_tests.rs`（新）。

## Problem

- B235：Ask/审批/goal 续轮的宿主文案作为 `TurnInput.message`（`display_message=None`）经 runtime
  落库为 **user** 行；它不是用户发言，却污染轮次语义（后续轮次看到伪造用户发言、标题/摘要失真）。
- B238：B232 把「【挂起前用户指令（宿主补录）】+ 宿主文案」作为续轮 user 行落库；当会话首轮即
  挂起时，该行成为首个非续轮 user 行 ⇒ 标题派生取到「【挂起前用户指令（宿主补录）】」、
  `is_host_continuation_prompt` 不命中组合正文 ⇒ needs_ai_title / continuation_only / turn_count
  全部被污染。

## Decision

- **schema 选「host 角色」而非新增 `HistoryMessageKind` 变体**：kind 只服务 assistant 合成记录，
  `append_turn` 用 `kind.is_some()` 决定 TurnCancelled 节点语义/是否 prepare_session_for_new_turn，
  复用会改坏轮次收口。`TurnHistoryMessage.role` 本就是自由字符串、SQLite role 列无 CHECK 约束，
  故 additive 新增白名单值 `host`（`session/host_resume.rs::HOST_HISTORY_ROLE`）。
- 写路径 `store.rs` 两处 append（正常/失败轮）统一走 `user_history_role(content)`：宿主续轮
  （含 B232 组合正文）⇒ `host`，真实用户 ⇒ `user`。
- 模型面：`context.rs::to_provider_history_message` 把 `host` 投影为 `ProviderRole::Developer`
  （`Host continuation context: …`），续轮指令对模型仍可见，且不伪造用户发言；native transcript
  回放（`context_window.rs`）把 `host` 落回 `user`，绝不把未知角色发给上游（避免 400）。
- 派生侧单一判据：`is_host_continuation_prompt` 增加「段标包围块」识别（`host_resume.rs::
  is_composed_host_continuation`），旧日志的 user 组合行同样被标题/turn_count/needs_ai_title/
  continuation_only 跳过；`session_is_continuation_fragment` 把 host 行计为续轮。
- TUI 组合正文改调 `host_resume::compose_host_continuation`（输出逐字不变，B232 冻结断言不受影响）。

## Alternatives considered

- **A. 新增 `HistoryMessageKind::HostResume` 并复用 kind 通道。** 落选：`kind.is_some()` 是
  TurnCancelled/新轮预处理的既有开关，新增 kind 会改变轮次收口语义。
- **B. 只改派生侧、不改落库角色。** 落选：B235 的核心是「历史行不得冒充用户发言」；
  派生修正只解决 B238 的显示，不能消除污染源。
- **C. native transcript 也保留 `host` 角色。** 落选：上游协议不认未知角色，回放会 400；
  只对 provider 历史投影做 developer 映射，native 回放落回 user。

## Verification

- `--lib host_resume` 4 passed、`--lib host_history` 3 passed（host 落库/投影可见/旧日志真实用户
  不回归/旧 user 组合行派生正确）；
- `--lib session` 293 passed、`--lib title` 15 passed、`--test approval_resume_e2e` 6 passed；
- `check --all-targets` exit 0；`npm run rust:lint` PASS；`size:check` 0 new（store.rs 净零增长）。
- 已知边界：native-tool-flow 会话的 `provider_native_transcript` 内该轮仍是 user 消息（模型面
  不变）；本卡只改历史行语义。
