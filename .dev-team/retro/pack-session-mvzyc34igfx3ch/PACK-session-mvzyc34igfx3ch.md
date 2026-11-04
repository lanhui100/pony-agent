# 证据包 session-mvzyc34igfx3ch（pack_hash=2025ed919e50357b）

- 任务: wave-60 可中断停止（结构化 stop_reason + 立即应答 + 可中断读通道 + 宽限期收口 + 双击 Esc） / 分级 B / 源码 L=1585 文件 22
- 区间: 7510042..680499c

## 提交序列
- 934fe130cd92 chore(wave-pds-3): 刷新 V8 帧预算收据并补提交复盘包  5 files changed, 1026 insertions(+), 32 deletions(-)
  files: .dev-team/retro/pack-session-e17642a6-b180-4ab4-abae-ee3260f00c36/PACK-session-e17642a6-b180-4ab4-abae-ee3260f00c36.md, .dev-team/retro/pack-session-e17642a6-b180-4ab4-abae-ee3260f00c36/pack-session-e17642a6-b180-4ab4-abae-ee3260f00c36.json, .dev-team/retro/pack-session-e8b744b1-8532-4d73-88ef-5979bd16d570/PACK-session-e8b744b1-8532-4d73-88ef-5979bd16d570.md, .dev-team/retro/pack-session-e8b744b1-8532-4d73-88ef-5979bd16d570/pack-session-e8b744b1-8532-4d73-88ef-5979bd16d570.json, tests/acceptance/pds-streaming/receipts/e2e/v8-relative-runs.json
- 6930ef1c6437 chore(protocol): freeze wave-60 stop contract & interface stubs  18 files changed, 470 insertions(+), 1 deletion(-)
  files: .dev-team/contracts/wave-60-interruptible-stop.md, .dev-team/regression-baseline-wave60.json, .dev-team/state.json, crates/pony-agent-core/src/agent/context.rs, crates/pony-agent-core/src/agent/control_plane/graph_projection.rs, crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/execution_control.rs, crates/pony-agent-core/src/agent/mod.rs
- f8769fecca01 test(wave-60): freeze red-phase acceptance tests (C1-C9, 40 cases)  9 files changed, 1921 insertions(+)
  files: crates/pony-agent-core/tests/stop_contract_wave60.rs, crates/pony-agent-core/tests/stop_escalation_wave60.rs, crates/pony-agent-core/tests/stop_interruptible_read_wave60.rs, crates/pony-agent-core/tests/stop_sequence_wave60.rs, crates/pony-agent-core/tests/tui_stop_static_wave60.rs, crates/pony-agent-core/tests/tui_stop_wave60.rs, src-tauri/tests/stop_registry_wave60.rs, tests/stop-sequence.spec.ts
- c575243983ae test(wave-60): migrate request_stop call shape to frozen 2-arg signature  1 file changed, 10 insertions(+), 10 deletions(-)
  files: crates/pony-agent-core/tests/stop_contract_wave60.rs
- 069587a75959 chore(protocol): freeze wave-61 team activation contract & NFR baseline (C1-C5)  4 files changed, 567 insertions(+), 1 deletion(-)
  files: .dev-team/contracts/wave-61-team-activation.md, .dev-team/nfr-baseline.json, .dev-team/nfr-baseline.schema.json, .dev-team/regression-baseline-wave61-team.json
- 1a16a51c38ff chore(protocol): freeze wave-61 contract v1.1 (C1.6 product names, C4 five/six migration surfaces)  2 files changed, 73 insertions(+), 14 deletions(-)
  files: .dev-team/contracts/wave-61-team-activation.md, .dev-team/nfr-baseline.schema.json
- b1d62f352302 test(wave-61-status-panel): freeze red-phase acceptance tests & authorized anchor migrations  33 files changed, 4119 insertions(+), 38 deletions(-)
  files: .dev-team/contracts/wave-61-status-panel.md, .dev-team/regression-baseline-wave61.json, .dev-team/review-plan-wave61-a.json, .dev-team/review-plan-wave61-b.json, .dev-team/state-wave61.json, .dev-team/tools/wave61-capture.mjs, .dev-team/wave61-migrated-vitest.json, .dev-team/wave61-red-purity-migrated.json
- 52a7a2182ca7 test(wave-61-team): freeze red-phase acceptance tests (C1-C5) + migrate 3 frozen contracts  5 files changed, 1390 insertions(+), 57 deletions(-)
  files: .dev-team/red-phase-wave61-team.json, crates/pony-agent-core/tests/tool_surface_wave20_contract.rs, crates/pony-agent-core/tests/wave25_capability_honesty_test.rs, crates/pony-agent-core/tests/wave37_tool_surface.rs, crates/pony-agent-core/tests/wave61_team_activation.rs
- 680499cee148 feat(wave-60): 可中断停止（结构化 stop_reason + 立即应答 + 可中断读通道 + 宽限期收口 + 双击 Esc）  25 files changed, 1639 insertions(+), 162 deletions(-)
  files: .dev-team/contracts/wave-60-interruptible-stop.md, .dev-team/nfr-baseline-wave60.json, .dev-team/review-l3p-wave60.json, .dev-team/review-wave60-a.json, .dev-team/review-wave60-b.json, .dev-team/review-wave60-recheck.json, .dev-team/state.json, .dev-team/wave60-delivery.md

## 交付后 7 天相关提交
- 22241e977009 feat(tui): optimize provider diagnostics, add secondary model menu and ponyfun branding files: .dev-team/contracts/tui-provider-refactor.md, .dev-team/meta-retrospective.md, crates/pony-agent-core/src/bin/tui.rs, crates/pony-agent-core/tests/tui_provider_refactor_test.rs, docs/screenshots/tui_key_error_diagnostic.png, docs/screenshots/tui_provider_menu.png
- 49522cc6c33e fix(ci): 抑制 wave-60 遗留死代码 names() 的 -D warnings 编译失败（Windows 测试步骤） files: crates/pony-agent-core/tests/stop_contract_wave60.rs
- b527a05ab374 fix(ci): unblock main — 移除 wave-61 遗留死代码 teammate_status（deny-warnings）+ V10 元测试跨平台 spawn vitest.mjs files: crates/pony-agent-core/src/agent/orchestration.rs, tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts
- a06e254f105f fix(wave-61-team): reactor-agnostic provider futures (C12) + e2e probe evidence files: .dev-team/contracts/wave-61-team-activation.md, .dev-team/green-phase-wave61-team.json, crates/pony-agent-core/examples/w61_e2e_probe.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/provider/model_catalog.rs, crates/pony-agent-core/src/agent/provider/responses_api.rs
- 889fb980cff8 feat(wave-63): TUI stopping 期按键不再静默吞（有可见反馈、不重复派发） files: crates/pony-agent-core/src/bin/tui.rs
- 93bdca3b18d6 test(wave-63): guard V10 meta-test recursion (nested env marker + self-proof) files: tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts
- e403302083a2 fix(wave-61-team): idle interrupt must not strand next drive (stale marker) + regression test files: .dev-team/green-phase-wave61-team.json, .dev-team/red-phase-wave61-team.json, .dev-team/review-wave61-fr.json, crates/pony-agent-core/src/agent/orchestration.rs, crates/pony-agent-core/tests/wave61_team_activation.rs
- a09c10c71f4e feat(wave-62): 工具级取消（turn 令牌下沉到 dispatcher 与 Run 子进程，清偿 wave-60 A9） files: .dev-team/review-wave62-concurrency.json, .dev-team/review-wave62-contract.json, .dev-team/review-wave62-recheck.json, .dev-team/state.json, .dev-team/wave62-delivery.md, crates/pony-agent-core/src/agent/dispatcher.rs
- 6b9e23a91586 fix(wave-61-team): drive loop drains until inbox empty (C10-A2) + interrupt marker + non-destructive inbox probe files: .dev-team/green-phase-wave61-team.json, .dev-team/red-phase-wave61-team.json, .dev-team/wave61-team-delivery.md, crates/pony-agent-core/src/agent/orchestration.rs, crates/pony-agent-core/tests/wave61_team_activation.rs
- c8997af10a6a feat(wave-61.1): 圆环后内联显示上下文占用百分比 files: .dev-team/contracts/wave-61-status-panel.md, .dev-team/wave61-red-receipts.md, docs/release/evidence/wave-61-status-panel/after-geometry.json, docs/release/evidence/wave-61-status-panel/after-valued-status-panel.png, docs/release/evidence/wave-61-status-panel/wave61-status-panel-fullpage.png, docs/release/evidence/wave-61-status-panel/wave61-status-panel-viewport.png
- ed6d35ef8b72 chore(wave-61-team): finalize v1.4 — single-flight driving, honest interrupt semantics, review receipts & meta retrospective files: .dev-team/contracts/wave-61-team-activation.md, .dev-team/green-phase-wave61-team.json, .dev-team/meta-retrospective-wave61-team.md, .dev-team/nfr-baseline.json, .dev-team/red-phase-wave61-team.json, .dev-team/review-plan-wave61-team.json
- cf783b000628 test(wave-61-status-panel): 用例缺陷重写 D1-D8（夹具/测量口径/缓动口径） files: tests/HomeSidebar.spec.ts, tests/acceptance/wave61/wave61-empty-values.spec.ts, tests/acceptance/wave61/wave61-harness.ts, tests/acceptance/wave61/wave61-layout.spec.ts, tests/e2e/wave61-status-panel.spec.ts
- 4c0fdc48fb5a chore(protocol): freeze wave-62 turn-cancellation field surface files: .dev-team/contracts/wave-62-tool-cancel.md, crates/pony-agent-core/src/agent/budget.rs, crates/pony-agent-core/src/agent/dispatcher.rs, crates/pony-agent-core/src/agent/execution_control.rs, crates/pony-agent-core/src/agent/runtime/mod.rs, crates/pony-agent-core/src/agent/tools.rs
- 922e960bb0ac feat(wave-61-team): real teammate execution + workspace-root propagation + honest 38-tool surface files: .dev-team/green-phase-wave61-team.json, .dev-team/wave61-team-delivery.md, crates/pony-agent-core/src/agent/context.rs, crates/pony-agent-core/src/agent/orchestration.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/subagent_runtime.rs
- 60e7cb84a20a feat(wave-61-team): expose team tools, wire host subagent engine, inject team policy prompt files: .dev-team/green-phase-wave61-team.json, crates/pony-agent-core/src/agent/context.rs, crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/runtime/builder.rs, crates/pony-agent-core/src/agent/runtime/trace_timeline.rs
- 056f8dff3cde chore(protocol): freeze wave-59 headless interface stubs & NFR baseline (contract v3) files: .dev-team/contract-matrix-wave59-headless.md, .dev-team/nfr-baseline-wave59.json, .dev-team/regression-baseline-wave59.json, .dev-team/state.json, crates/pony-agent-core/src/agent/headless.rs, crates/pony-agent-core/src/agent/mod.rs

## 机械红旗（判定必须逐条回应）
- impl_commit_modified_frozen_test:6930ef1c6437
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
