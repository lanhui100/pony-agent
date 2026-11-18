# Agent Note: 审批续轮改为 run-bound 续跑

Status: implemented

工单：B233。影响文件：`agent/control_plane/ask_resume_commands.rs`、`agent/control_plane/mod.rs`、
`agent/graph.rs`、`bin/tui.rs`、`tests/approval_resume_e2e.rs`、`tests/wave65_approval_unit_tests.rs`、
`.dev-team/contract-matrix-approval-resume.md`。

## Problem

审批挂起（路径授权类）批准/拒绝后，TUI 投递 `UiEvent::ApprovalResumeTurn` 走
`start_ask_aware_turn_stream` **新起一轮**：挂起 run 的轮内状态（已执行工具步骤、provider 消息流）
全部丢弃，只能靠已提交历史+宿主文案重建。Ask 续轮已走 `continue_ask_resume_turn_stream`（钉死
run + 一次性注入），审批续轮不同构。

根因还有一处：审批挂起**不释放 `active_turn_id`**（run 停在 Running），同 run 在运行期不可续。

## Decision

- `ask_resume_commands.rs:205` 新增 `continue_approval_resume_turn_stream(sink, run_id, command)`：
  与 Ask 续轮共用 `continue_bound_graph_run_stream`（prepare+step 唯一实现，ask 版改为委托，
  **不复制**第二套图执行）；run 不可续 ⇒ 结构化 error 日志 + 中文 `Err`，**绝不**回落新轮。
- `graph.rs:1037` 新增 `GraphRunner::suspend_for_host_decision`：仅当「无 Ask wait 且
  active turn == 本轮」时把 run 释放到 WaitingUser（幂等保守）；`control_plane/mod.rs` helper
  `release_approval_suspension_turn` 在挂起分支调用。Ask 路径行为不变。
- TUI：`UiEvent::ApprovalResumeTurn { prompt, run_id }` 携带 C3 投影的 `runId`；
  `dispatch_approval_resume`（按键分支唯一出口）在 run_id 缺失时中文提示 + 结构化日志且
  **不投递事件**（fail-closed，不静默新起轮）；消费分支改 `continue_approval_resume_turn_stream`
  （失败按 StreamFailed 兜底）。
- 批准顺序不变：先 `authorize_path(path,"read")` 后 `approve_control_request`；拒绝仍
  `cancel_approval_request`。INV-1/CAS/nonce 未放宽。
- **契约迁移（Lead 批准）**：C4 注明「同 run/turn，不再新起 graph run；run 内仍按 turn 发
  `turn:started`」；C5 T2 改锚 run-bound（同 run_id、run 数不变、最后完成 turn 对齐），T1/T3/T4
  不动；新增 T5（拒绝后同一 run 续跑、越界工具结果绝不回灌）与 T6（run 不可续 fail-closed）；
  变更流水登记。

## Alternatives considered

- **A. 继续用新轮，仅把历史+文案补全。** 落选：挂起 run 的工具步骤与 provider 流仍丢弃，
  且与 Ask 续轮语义分叉。
- **B. 复制一份 approval 版图执行逻辑。** 落选：第二套 prepare/step 必然漂移；共用
  `continue_bound_graph_run_stream` 才能保证单一真源。
- **C. run 不可续时回落新轮。** 落选：会掩盖 run 状态机错误（B233 现场就是 active turn 未释放）；
  fail-closed 才能暴露问题。

## Verification

- `cargo-lane test --test approval_resume_e2e` 6 passed（T1/T3/T4 原样；T2 改锚 run-bound；
  T5/T6 新增）；`--test ask_resume_e2e` 8 passed（1 ignored 既有）、
  `--test permission_fail_closed_contract` 3 passed；`--lib turn_control` 9 passed；
  `--bin pony dispatch_approval_resume` 1 passed。
- `check -p pony-agent-core --all-targets` exit 0；`npm run rust:lint` PASS。
- 已知限制（如实登记）：拒绝路径没有 Ask 式 cancelled 结果注入通道，T5 证明的是同一 run 可续跑、
  模型按宿主文案换路、越界工具结果不回灌；「模型面收到 cancelled 控制结果」无机器可判证据，
  属 F-1（dispatcher 级豁免/注入）范畴。
- B235/B238 结论：不因此改变——续轮仍以组合正文 + `display_message=None` 落 user 行。
