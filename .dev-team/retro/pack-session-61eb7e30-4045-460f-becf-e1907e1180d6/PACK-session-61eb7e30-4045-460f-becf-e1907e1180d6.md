# 证据包 session-61eb7e30-4045-460f-becf-e1907e1180d6（pack_hash=53fb96df57408171）

- 任务: TUI ask overlay visibility, stop sink registration and JSON log leak fixes / 分级 B / 源码 L=206 文件 3
- 区间: d710bb34..7a5269ed0c36112fc01b10d97c70ece4dcb8db5d

## 提交序列
- dd8159200872 test(tui): freeze ask overlay and stop-cancel acceptance  51 files changed, 2299 insertions(+)
  files: .dev-team/contract-matrix-tui-overlay-cancel.md, .dev-team/l2at-done-tui-overlay-cancel.json, .dev-team/nfr-baseline-tui-overlay-cancel.json, .dev-team/observability-tui-overlay-cancel.json, .dev-team/regression-baseline-tui-overlay-cancel.json, .dev-team/review-plan-tui-overlay-cancel.json, crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs, docs/decisions/0063-tui-ask-user-overlay-and-cancel-feedback.md
- 450d804cb9a4 test(tui): correct O1 metrics fixture and CJK assertions  35 files changed, 211 insertions(+), 106 deletions(-)
  files: crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs, tests/acceptance/receipts/l2at-tui-overlay-cancel/bin.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/failed-assertions.txt, tests/acceptance/receipts/l2at-tui-overlay-cancel/green.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/o1-fixture-defect-evidence.txt, tests/acceptance/receipts/l2at-tui-overlay-cancel/run.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock-requests.jsonl, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock.out
- 4b9e241af205 test(tui): assert PTY screen has no structured log JSON leak  29 files changed, 107 insertions(+), 43 deletions(-)
  files: tests/acceptance/l2at-tui-overlay-cancel.sh, tests/acceptance/receipts/l2at-tui-overlay-cancel/bin.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/o8-structured-log-leak-evidence.txt, tests/acceptance/receipts/l2at-tui-overlay-cancel/run.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-prefix-structured-log-screen-red-phase.txt, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock-requests.jsonl, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock.out, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/s1-0-streaming.ansi
- 818cf22e3e8b fix(tui): keep ask context visible and surface stop feedback  18 files changed, 228 insertions(+), 53 deletions(-)
  files: .dev-team/contract-matrix-tui-overlay-cancel.md, .dev-team/l2at-done-tui-overlay-cancel.json, crates/pony-agent-core/src/bin/tui.rs, docs/decisions/0063-tui-ask-user-overlay-and-cancel-feedback.md, docs/observability.yaml, tests/acceptance/receipts/l2at-tui-overlay-cancel/bin.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/run.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock-requests.jsonl
- 9ba49b4ce3f4 test(tui): judge C9 sink registration on the runtime log surface  22 files changed, 192 insertions(+), 53 deletions(-)
  files: crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs, tests/acceptance/l2at-tui-overlay-cancel.sh, tests/acceptance/receipts/l2at-tui-overlay-cancel/bin.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/c9-runtime-log-red-direction-evidence.txt, tests/acceptance/receipts/l2at-tui-overlay-cancel/green.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/run.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock-requests.jsonl, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock.out
- b477c090a1c3 docs(tui): ratify ask overlay contract revision 2 [decl-change-approved]  4 files changed, 19 insertions(+), 10 deletions(-)
  files: .dev-team/contract-matrix-tui-overlay-cancel.md, .dev-team/nfr-baseline-tui-overlay-cancel.json, docs/decisions/0063-tui-ask-user-overlay-and-cancel-feedback.md, docs/observability.yaml
- 3c6823100d79 test(tui): cover history_tool_detail contract and relabel log-leak judge as C10  20 files changed, 181 insertions(+), 35 deletions(-)
  files: crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs, tests/acceptance/l2at-tui-overlay-cancel.sh, tests/acceptance/receipts/l2at-tui-overlay-cancel/bin.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/green.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/o8-structured-log-leak-evidence.txt, tests/acceptance/receipts/l2at-tui-overlay-cancel/run.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock-requests.jsonl, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock.out
- 8d18d15700f6 fix(tui): tighten stop sink lifecycle and suppress structured payloads  3 files changed, 54 insertions(+), 20 deletions(-)
  files: crates/pony-agent-core/src/agent/otel/log.rs, crates/pony-agent-core/src/bin/cli.rs, crates/pony-agent-core/src/bin/tui.rs
- a6756d8410b9 test(tui): refresh l2at receipts for 8d18d157 verification  21 files changed, 31 insertions(+), 28 deletions(-)
  files: tests/acceptance/receipts/l2at-tui-overlay-cancel/bin.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/run.log, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock-requests.jsonl, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock.out, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/s1-0-streaming.ansi, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/s1-0-streaming.png, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/s1-0-streaming.squeezed.txt, tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/s1-0-streaming.txt
- 7a5269ed0c36 chore(tui): archive green evidence and independent reviews  4 files changed, 229 insertions(+), 4 deletions(-)
  files: .dev-team/l2at-done-tui-overlay-cancel.json, .dev-team/meta-retrospective-tui-overlay-cancel.md, .dev-team/review-l3a-tui-overlay-cancel.json, .dev-team/review-l3p-tui-overlay-cancel.json

## 交付后 7 天相关提交
- 无

## 机械红旗（判定必须逐条回应）
- frozen_test_drift:tests/acceptance/receipts/l2at-tui-overlay-cancel/bin.log,tests/acceptance/receipts/l2at-tui-overlay-cancel/run.log,tests/acceptance/receipts/l2at-tui-overlay-cancel/s1-stop-esc/mock-requests.jsonl,te
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
- self_signed_review:review-l3a-tui-overlay-cancel.json
- self_signed_review:review-l3p-tui-overlay-cancel.json
- self_signed_review:review-l3p-wave60.json
- self_signed_review:review-l3p.json
- self_signed_review:review-plan-extensibility.json
- self_signed_review:review-plan-pds3.json
- self_signed_review:review-plan-tui-overlay-cancel.json
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
