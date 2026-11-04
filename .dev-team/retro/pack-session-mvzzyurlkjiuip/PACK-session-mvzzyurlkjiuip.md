# 证据包 session-mvzzyurlkjiuip（pack_hash=02073844119b61bd）

- 任务: wave-61-team: 修复 dev-team 技能无法触发 agent team/子智能体（工具面暴露+宿主接线+团队策略提示+真实驱动） / 分级 B / 源码 L=2672 文件 26
- 区间: 52a7a21..ed6d35e

## 提交序列
- 680499cee148 feat(wave-60): 可中断停止（结构化 stop_reason + 立即应答 + 可中断读通道 + 宽限期收口 + 双击 Esc）  25 files changed, 1639 insertions(+), 162 deletions(-)
  files: .dev-team/contracts/wave-60-interruptible-stop.md, .dev-team/nfr-baseline-wave60.json, .dev-team/review-l3p-wave60.json, .dev-team/review-wave60-a.json, .dev-team/review-wave60-b.json, .dev-team/review-wave60-recheck.json, .dev-team/state.json, .dev-team/wave60-delivery.md
- bcd71376a720 chore(release): wave-60 元复盘与收口（L3-A/B/L3-P + recheck 收据、NFR 基线、交付说明）  1 file changed, 66 insertions(+)
  files: .dev-team/meta-retrospective-wave60.md
- ee336f310cca fix(wave-60): 补提交 src/lib/runtime/stop-sequence.ts（双击 Esc 判定实现）  1 file changed, 78 insertions(+)
  files: src/lib/runtime/stop-sequence.ts
- 60e7cb84a20a feat(wave-61-team): expose team tools, wire host subagent engine, inject team policy prompt  7 files changed, 447 insertions(+), 31 deletions(-)
  files: .dev-team/green-phase-wave61-team.json, crates/pony-agent-core/src/agent/context.rs, crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/runtime/builder.rs, crates/pony-agent-core/src/agent/runtime/trace_timeline.rs, crates/pony-agent-core/src/agent/tools.rs
- de43b392b2c1 chore(protocol): freeze wave-61 contract v1.3 (C6 single-gate, C7 real teammate execution, C8 workspace root, C9 single-eval)  4 files changed, 609 insertions(+), 284 deletions(-)
  files: .dev-team/contracts/wave-61-team-activation.md, .dev-team/nfr-baseline.json, .dev-team/nfr-baseline.schema.json, .dev-team/red-phase-wave61-team.json
- 922e960bb0ac feat(wave-61-team): real teammate execution + workspace-root propagation + honest 38-tool surface  12 files changed, 1286 insertions(+), 345 deletions(-)
  files: .dev-team/green-phase-wave61-team.json, .dev-team/wave61-team-delivery.md, crates/pony-agent-core/src/agent/context.rs, crates/pony-agent-core/src/agent/orchestration.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/subagent_runtime.rs, crates/pony-agent-core/src/agent/tools.rs, crates/pony-agent-core/tests/pa110_teams_acceptance.rs
- 5c0eb4090e18 feat(wave-61-status-panel): 圆环迁入状态面板第一行 + 状态栏间距/对齐 + 空值不渲染 + 有值入场动效  2 files changed, 274 insertions(+), 138 deletions(-)
  files: src/components/HomeStatusPanel.vue, src/components/chat/WorkspaceComposer.vue
- cf783b000628 test(wave-61-status-panel): 用例缺陷重写 D1-D8（夹具/测量口径/缓动口径）  5 files changed, 294 insertions(+), 33 deletions(-)
  files: tests/HomeSidebar.spec.ts, tests/acceptance/wave61/wave61-empty-values.spec.ts, tests/acceptance/wave61/wave61-harness.ts, tests/acceptance/wave61/wave61-layout.spec.ts, tests/e2e/wave61-status-panel.spec.ts
- ed6d35ef8b72 chore(wave-61-team): finalize v1.4 — single-flight driving, honest interrupt semantics, review receipts & meta retrospective  14 files changed, 1472 insertions(+), 284 deletions(-)
  files: .dev-team/contracts/wave-61-team-activation.md, .dev-team/green-phase-wave61-team.json, .dev-team/meta-retrospective-wave61-team.md, .dev-team/nfr-baseline.json, .dev-team/red-phase-wave61-team.json, .dev-team/review-plan-wave61-team.json, .dev-team/review-wave61-a.json, .dev-team/review-wave61-l3p.json

## 交付后 7 天相关提交
- 0b6c5c216dd7 fix(runtime): promote 结果恢复熔断输入 + 禁止重复执行指引（B010） files: backlog/backlog.md, crates/pony-agent-core/src/agent/runtime/stream_support.rs, crates/pony-agent-core/src/agent/tools.rs, crates/pony-agent-core/tests/timeout_signal_promote_test.rs
- 0ebfd9aeeafa feat(jobs): job 层会话归属，跨会话不可读/不可杀（B012） files: backlog/backlog.md, crates/pony-agent-core/src/agent/jobs.rs, crates/pony-agent-core/src/agent/tools.rs, crates/pony-agent-core/tests/job_session_scope_test.rs
- 87b3a1909b83 fix(jobs): promote 后取消生效 + POSIX 进程组收口（B009） files: backlog/backlog.md, crates/pony-agent-core/src/agent/jobs.rs, crates/pony-agent-core/src/agent/process.rs, crates/pony-agent-core/src/agent/tools.rs, crates/pony-agent-core/tests/run_timeout_promote_test.rs
- 11a93ff23aac feat(tools): Run 超时 promote 为后台 job（opt-in 预览，默认关闭） files: .dev-team/contracts/tool-timeout-policy.md, .dev-team/designs/run-timeout-promote.md, .dev-team/review-promote-A.json, .dev-team/review-promote-B.json, backlog/backlog.md, crates/pony-agent-core/src/agent/jobs.rs
- 767501776c19 feat(tools): 超时 hint 指向后台执行（CT-5/CT-5a，契约 v3） files: .dev-team/contracts/tool-timeout-policy.md, backlog/backlog.md, crates/pony-agent-core/src/agent/tools.rs, docs/analysis/trace-analysis-pa-01a1a134.md, src/lib/runtime/browser-preview.ts
- 94f1053910b7 feat(tools): Run 超时策略对齐 DSH（默认 120s / 上限 600s / 证据 timedOut） files: .dev-team/meta-retrospective-tool-timeout-20261104.md, .dev-team/review-tool-timeout.json, backlog/backlog.md, crates/pony-agent-core/src/agent/tools.rs, docs/analysis/trace-analysis-pa-01a1a134.md, src/lib/runtime/browser-preview.ts
- 22241e977009 feat(tui): optimize provider diagnostics, add secondary model menu and ponyfun branding files: .dev-team/contracts/tui-provider-refactor.md, .dev-team/meta-retrospective.md, crates/pony-agent-core/src/bin/tui.rs, crates/pony-agent-core/tests/tui_provider_refactor_test.rs, docs/screenshots/tui_key_error_diagnostic.png, docs/screenshots/tui_provider_menu.png
- b527a05ab374 fix(ci): unblock main — 移除 wave-61 遗留死代码 teammate_status（deny-warnings）+ V10 元测试跨平台 spawn vitest.mjs files: crates/pony-agent-core/src/agent/orchestration.rs, tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts
- a06e254f105f fix(wave-61-team): reactor-agnostic provider futures (C12) + e2e probe evidence files: .dev-team/contracts/wave-61-team-activation.md, .dev-team/green-phase-wave61-team.json, crates/pony-agent-core/examples/w61_e2e_probe.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/provider/model_catalog.rs, crates/pony-agent-core/src/agent/provider/responses_api.rs
- 889fb980cff8 feat(wave-63): TUI stopping 期按键不再静默吞（有可见反馈、不重复派发） files: crates/pony-agent-core/src/bin/tui.rs
- 86042d420fd7 fix(wave-60): 补提交 composer 的 stopping 禁用态（T4 因并发 wave-61 冲突被排除的 3 行） files: src/components/chat/WorkspaceComposer.vue
- e403302083a2 fix(wave-61-team): idle interrupt must not strand next drive (stale marker) + regression test files: .dev-team/green-phase-wave61-team.json, .dev-team/red-phase-wave61-team.json, .dev-team/review-wave61-fr.json, crates/pony-agent-core/src/agent/orchestration.rs, crates/pony-agent-core/tests/wave61_team_activation.rs
- a09c10c71f4e feat(wave-62): 工具级取消（turn 令牌下沉到 dispatcher 与 Run 子进程，清偿 wave-60 A9） files: .dev-team/review-wave62-concurrency.json, .dev-team/review-wave62-contract.json, .dev-team/review-wave62-recheck.json, .dev-team/state.json, .dev-team/wave62-delivery.md, crates/pony-agent-core/src/agent/dispatcher.rs
- 6b9e23a91586 fix(wave-61-team): drive loop drains until inbox empty (C10-A2) + interrupt marker + non-destructive inbox probe files: .dev-team/green-phase-wave61-team.json, .dev-team/red-phase-wave61-team.json, .dev-team/wave61-team-delivery.md, crates/pony-agent-core/src/agent/orchestration.rs, crates/pony-agent-core/tests/wave61_team_activation.rs
- c8997af10a6a feat(wave-61.1): 圆环后内联显示上下文占用百分比 files: .dev-team/contracts/wave-61-status-panel.md, .dev-team/wave61-red-receipts.md, docs/release/evidence/wave-61-status-panel/after-geometry.json, docs/release/evidence/wave-61-status-panel/after-valued-status-panel.png, docs/release/evidence/wave-61-status-panel/wave61-status-panel-fullpage.png, docs/release/evidence/wave-61-status-panel/wave61-status-panel-viewport.png
- 60fdc0d9372c docs(wave-61-status-panel): 修正 prop JSDoc 与 E-5 的文档漂移（行为不变） files: src/components/HomeStatusPanel.vue
- 8f0f4852690e fix(wave-61-status-panel): 位点缺席收列（R9 取代 R8）+ 键盘可达与降级补齐 files: .dev-team/contracts/wave-61-status-panel.md, .dev-team/meta-retrospective-wave61.md, .dev-team/review-join-wave61-b.json, .dev-team/wave61-delivery.md, .dev-team/wave61-receipts.md, docs/release/evidence/wave-61-status-panel/after-geometry.json
- 427ee0b6c633 chore(wave-61-status-panel): 圆环零值一并隐去 + 绿相收据/截图证据/汇聚审查收据 files: .dev-team/contracts/wave-61-status-panel.md, .dev-team/review-join-wave61-a.json, .dev-team/review-join-wave61-b.json, .dev-team/state-wave61.json, .dev-team/wave61-delivery.md, .dev-team/wave61-green-vitest.json
- 4c0fdc48fb5a chore(protocol): freeze wave-62 turn-cancellation field surface files: .dev-team/contracts/wave-62-tool-cancel.md, crates/pony-agent-core/src/agent/budget.rs, crates/pony-agent-core/src/agent/dispatcher.rs, crates/pony-agent-core/src/agent/execution_control.rs, crates/pony-agent-core/src/agent/runtime/mod.rs, crates/pony-agent-core/src/agent/tools.rs

## 机械红旗（判定必须逐条回应）
- frozen_test_drift:crates/pony-agent-core/tests/wave61_team_activation.rs
- impl_commit_modified_frozen_test:922e960bb0ac
- impl_commit_modified_frozen_test:ed6d35ef8b72
- red_commit_after_impl
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

## 无法从机器判定的未知项
- 每个门禁实际抓到多少缺陷（gate_caught）需从 review JSON/测试日志判定
- 用户是否口头授权了自扩范围（scope）需读会话原文
