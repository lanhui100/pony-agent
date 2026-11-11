# PA-103 C1 plan/spec adversarial review — test and release

- Reviewer: independent test/release reviewer
- Scope: proposal/design/tasks/ADR before implementation
- Verdict: **PASS WITH FINDINGS → PASS after revision**

## Findings

- P1: No executable manifest/artifact/CI contract, platform matrix or real smoke evidence path.
- P1: UI selectors/state sequence, concurrent operation rules and failure injection seams were missing.
- P2: Commands, target artifacts, version/tag rules, plugin ACL schema and release runbook were not concrete.

## Adopted changes

The revised design adds local versus release-owner gates, an explicit browser/dev/Windows/macOS/Linux matrix, exact local commands, selector/state and fake-adapter requirements, signature/manifest/redirect/partial-download failure cases, version/target checks, signed fixture limits, and evidence requirements for a real Windows smoke. It explicitly distinguishes mock adapter evidence from real Tauri signature verification.

## Unadopted/deferred

No finding was rejected. A production release workflow, real endpoint/key, artifact upload and Windows smoke are deferred to the release-owner gate because the repository has no current signing infrastructure; the local contract must not pretend those checks passed.
