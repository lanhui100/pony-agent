# 证据包 session-ad203415-47ce-458e-afd3-2c0ed4ca2ce7（pack_hash=f1b7072ee43e723b）

- 任务: wave-64 停止即时生效 + 终态由最后一条状态消息唯一承载 / 分级 B / 源码 L=908 文件 12
- 区间: 06472465..043c26e1

## 提交序列
- af599ef497c6 test(wave-64): freeze red-phase acceptance tests (F1 stopped/error status, F2 no extra message, R1/R2 stop semantics)  12 files changed, 2009 insertions(+), 23 deletions(-)
  files: .dev-team/contracts/wave-64-stop-terminal-status.md, .dev-team/l2at-done-wave64-tests.json, .dev-team/regression-baseline-wave64.json, .dev-team/state.json, .dev-team/wave64-red-receipts.md, crates/pony-agent-core/tests/stop_graph_run_wave64.rs, tests/HomeWorkspace.spec.ts, tests/acceptance/wave64/f1-f2-workspace-terminal-status.spec.ts
- 778717ecfda2 feat(ui): 工具退出码分类解析与中文标签（P1-2 数据层）+ 清理假 token 字面量  6 files changed, 280 insertions(+), 25 deletions(-)
  files: crates/pony-agent-core/src/agent/otel/mod.rs, crates/pony-agent-core/src/agent/ponysentry/sanitizer.rs, crates/pony-agent-core/tests/otel_tool_payload_test.rs, crates/pony-agent-core/tests/otel_turn_failure_reason_test.rs, src/lib/runtime/tool-result-meta.ts, tests/tool-exit-code-meta.spec.ts
- 801cafe05e1d feat(wave-64): 停止即时生效 + 终态由最后一条状态消息唯一承载  23 files changed, 1719 insertions(+), 162 deletions(-)
  files: .dev-team/contracts/wave-64-stop-terminal-status.md, .dev-team/l2at-done-wave64-tests.json, .dev-team/review-wave64-a.json, .dev-team/review-wave64-b.json, .dev-team/review-wave64-l3p.json, .dev-team/wave64-browser-gate.md, .dev-team/wave64-green-fe.md, .dev-team/wave64-green-rs.md
- 5f2950605630 fix(wave-64): prepare_turn 失败派发补 terminal-once 闸门 + 交付归档与可观测性归属登记  6 files changed, 211 insertions(+)
  files: .dev-team/review-wave64-recheck.json, .dev-team/state.json, .dev-team/wave64-delivery.md, .dev-team/wave64-green-rs.md, crates/pony-agent-core/src/agent/runtime/turn_stream.rs, docs/observability.yaml
- 043c26e1f167 chore(release): wave-64 元复盘与收口（审查收据/浏览器证据/元复盘）  1 file changed, 27 insertions(+), 120 deletions(-)
  files: .dev-team/meta-retrospective.md

## 交付后 7 天相关提交
- 16b691293b8e feat(runtime): parallel_tools 接入批执行（开关默认关，P1-1 / B018 的 D4） files: .dev-team/designs/multi-tool-call-execution.md, backlog/backlog.md, crates/pony-agent-core/src/agent/runtime/batch_plan.rs, crates/pony-agent-core/src/agent/runtime/limits.rs, crates/pony-agent-core/src/agent/runtime/mod.rs, crates/pony-agent-core/src/agent/runtime/tests.rs
- efbb9195c609 feat(runtime): turn_stream 批执行多工具调用（P1-1 / B018 的 D3b） files: .dev-team/designs/multi-tool-call-execution.md, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/provider/openai_sse.rs, crates/pony-agent-core/src/agent/runtime/mod.rs, crates/pony-agent-core/src/agent/runtime/tests.rs, crates/pony-agent-core/src/agent/runtime/turn_stream.rs
- 6d743b821272 feat(provider): ProviderDecision/Response 增 tool_calls 复数位（P1-1 / B018 的 D1a） files: crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/planner.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/runtime/planner_patches.rs, crates/pony-agent-core/src/agent/runtime/tests.rs, crates/pony-agent-core/src/agent/runtime/tool_recovery.rs
- 33b416ef2c10 feat(session): turn 级 degraded/tool_failures 落本地记录并在 trace 面板呈现（P1-2 收尾） files: crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/otel/mod.rs, crates/pony-agent-core/src/agent/runtime/turn_persist.rs, crates/pony-agent-core/src/agent/session/tests.rs, crates/pony-agent-core/src/agent/session/types.rs, crates/pony-agent-core/src/agent/sqlite_session.rs
- ed089e793383 fix(runtime): 主失败路径保留已产出正文（B017）+ 修 wave-64 遗留 stop_turn 用例 files: crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/runtime/tests.rs, crates/pony-agent-core/src/agent/runtime/turn_persist.rs, crates/pony-agent-core/src/agent/runtime/turn_stream.rs
- 85252be7305a feat(telemetry): 上传回执结构化 + 部分成功语义（P1-3） files: crates/pony-agent-core/src/agent/control_plane/query_commands.rs, crates/pony-agent-core/src/agent/otel/mod.rs, crates/pony-agent-core/tests/otel_trace_migration_test.rs, crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs, crates/pony-agent-core/tests/upload_trace_receipt_test.rs, src-tauri/src/lib.rs
- 2a2b02ddd56b feat(ui): 工具行显示退出码分类徽标（P1-2 接线） files: src/components/chat/WorkspaceTurnItem.vue

## 机械红旗（判定必须逐条回应）
- frozen_test_drift:crates/pony-agent-core/tests/stop_graph_run_wave64.rs,tests/acceptance/wave64/f1-f2-workspace-terminal-status.spec.ts,tests/acceptance/wave64/f2-no-placeholder-message.spec.ts
- impl_commit_modified_frozen_test:778717ecfda2
- self_signed_review:review-join-a.json
- self_signed_review:review-join-a2.json
- self_signed_review:review-join-b.json
- self_signed_review:review-join-b2.json
- self_signed_review:review-join-pds3.json
- self_signed_review:review-join-wave59-r2.json
- self_signed_review:review-join-wave59.json
- self_signed_review:review-join-wave61-a.json
- self_signed_review:review-join-wave61-b.json
- self_signed_review:review-join.json
- self_signed_review:review-l3-concurrency.json
- self_signed_review:review-l3-security.json
- self_signed_review:review-l3p-wave60.json
- self_signed_review:review-l3p.json
- self_signed_review:review-plan-pds3.json
- self_signed_review:review-plan-wave58.json
- self_signed_review:review-plan-wave59.json
- self_signed_review:review-plan-wave61-a.json
- self_signed_review:review-plan-wave61-b.json
- self_signed_review:review-plan-wave61-team.json
- self_signed_review:review-plan.json
- self_signed_review:review-promote-A.json
- self_signed_review:review-promote-B.json
- self_signed_review:review-tool-timeout.json
- self_signed_review:review-wave60-a.json
- self_signed_review:review-wave60-b.json
- self_signed_review:review-wave60-recheck.json
- self_signed_review:review-wave61-a.json
- self_signed_review:review-wave61-fa.json
- self_signed_review:review-wave61-fp.json
- self_signed_review:review-wave61-fr.json
- self_signed_review:review-wave61-fv.json
- self_signed_review:review-wave61-l3p.json
- self_signed_review:review-wave61-recheck.json
- self_signed_review:review-wave62-concurrency.json
- self_signed_review:review-wave62-contract.json
- self_signed_review:review-wave62-recheck.json
- self_signed_review:review-wave64-a.json
- self_signed_review:review-wave64-b.json
- self_signed_review:review-wave64-l3p.json
- self_signed_review:review-wave64-recheck.json

## 无法从机器判定的未知项
- 每个门禁实际抓到多少缺陷（gate_caught）需从 review JSON/测试日志判定
- 用户是否口头授权了自扩范围（scope）需读会话原文
