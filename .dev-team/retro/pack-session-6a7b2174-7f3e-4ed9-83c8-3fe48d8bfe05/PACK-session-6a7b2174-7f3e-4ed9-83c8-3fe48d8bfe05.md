# 证据包 session-6a7b2174-7f3e-4ed9-83c8-3fe48d8bfe05（pack_hash=02173c45b6ff7d24）

- 任务: 架构评估逐项复核并实施修复(wave-55)：编译门禁/并发授权上下文/沙箱诚实性/持久化原子性/渲染层外泄/事件sessionId/文档事实同步 / 分级 B / 源码 L=1843 文件 29
- 区间: 5fa0ca6..9001182

## 提交序列
- 9bf78627a3f1 fix(core): restore all-targets build by filling TurnInput.tool_authorization_mode in test constructors  3 files changed, 113 insertions(+)
  files: crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/runtime/tests.rs, crates/pony-agent-core/src/agent/sse_adapter.rs
- 4bc7a7c0a6ac test(arch-remediation): freeze red-phase acceptance tests for verified architecture defects  5 files changed, 816 insertions(+)
  files: crates/pony-agent-core/tests/concurrent_turn_context_isolation_red.rs, crates/pony-agent-core/tests/sandbox_isolation_honesty_red.rs, crates/pony-agent-core/tests/session_persistence_observability_red.rs, tests/markdown-external-resource-allowlist.red.spec.ts, tests/runtime-session-event-isolation.red.spec.ts
- 115ce83d48f0 docs(governance): sync ADR facts, whitelist truth and doc links; add ADR 0054 and debt proposals [decl-change-approved]  15 files changed, 327 insertions(+), 22 deletions(-)
  files: AGENTS.md, docs/INDEX.md, docs/analysis/builtin-tool-surface-comparison-2026-06-22.md, docs/analysis/trace-redteam-2026-06-01.md, docs/architecture/context-state-subsystem.md, docs/architecture/overview.md, docs/architecture/session-control-plane-and-audit-surface.md, docs/architecture/structural-debt-proposals.md
- 59a5df7e8aa5 fix(core): close verified concurrency/security/persistence defects (wave-55)  25 files changed, 1630 insertions(+), 209 deletions(-)
  files: crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/dispatcher_composites.rs, crates/pony-agent-core/src/agent/governed_executor.rs, crates/pony-agent-core/src/agent/mcp_client.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/runtime/mod.rs, crates/pony-agent-core/src/agent/runtime/turn_persist.rs, crates/pony-agent-core/src/agent/runtime/turn_stream.rs
- e7430ac1108d fix(web): close render-layer exfil channel and session-event fail-open (wave-55)  14 files changed, 599 insertions(+), 62 deletions(-)
  files: index.html, src-tauri/src/lib.rs, src-tauri/src/platform.rs, src-tauri/tauri.conf.json, src-tauri/tests/tool_router_regression.rs, src/components/MarkdownRenderer.vue, src/components/config/ConfigGeneralSection.vue, src/lib/markdown.ts
- 90011820dd07 chore(wave-55): close-out ledger, review receipts, NFR baseline amendment and meta-retrospective  12 files changed, 454 insertions(+), 322 deletions(-)
  files: .dev-team/meta-retrospective-wave54.md, .dev-team/meta-retrospective.md, .dev-team/nfr-baseline.json, .dev-team/regression-baseline-wave55.json, .dev-team/review-join.json, .dev-team/review-l3-concurrency.json, .dev-team/review-l3-security.json, .dev-team/review-plan.json

## 交付后 7 天相关提交
- 57c6412dfa80 feat(arch): complete wave-58 context and skills loading mechanism overhaul files: .dev-team/contract-matrix-wave58.md, .dev-team/meta-retrospective-wave58.md, .dev-team/review-join-a.json, .dev-team/review-join-a2.json, .dev-team/review-join-b.json, .dev-team/review-join-b2.json
- 0a1fdde203e8 feat(arch): complete wave-56 and wave-57-A structural hardening files: .dev-team/nfr-baseline.json, crates/pony-agent-core/src/agent/control_plane/capability_commands.rs, crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/dispatcher_composites.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/provider/openai_sse.rs
- d321153be20b perf(ipc): move hot-path Tauri commands off the IPC thread and drop dead IPC wrappers files: src-tauri/src/lib.rs, src-tauri/src/tauri_adapter.rs, src/components/HomeWorkspace.vue

## 机械红旗（判定必须逐条回应）
- frozen_test_drift:crates/pony-agent-core/tests/concurrent_turn_context_isolation_red.rs
- impl_commit_modified_frozen_test:59a5df7e8aa5
- impl_commit_modified_frozen_test:e7430ac1108d
- red_commit_after_impl
- self_signed_review:review-join-a.json
- self_signed_review:review-join-a2.json
- self_signed_review:review-join-b.json
- self_signed_review:review-join-b2.json
- self_signed_review:review-join.json
- self_signed_review:review-l3-concurrency.json
- self_signed_review:review-l3-security.json
- self_signed_review:review-plan-wave58.json
- self_signed_review:review-plan.json

## 无法从机器判定的未知项
- 每个门禁实际抓到多少缺陷（gate_caught）需从 review JSON/测试日志判定
- 用户是否口头授权了自扩范围（scope）需读会话原文
