# PA-103 C1 plan/spec adversarial review — security

- Reviewer: independent security reviewers (two passes)
- Scope: proposal/design/tasks/ADR before implementation
- Verdict: **PASS WITH FINDINGS → PASS after revision**

## Findings

- P1: HTTPS/public-key language did not specify fixed origin, runtime override prohibition, CI hard-fail, target/version binding or failure behavior.
- P1: source trust was not separated from the existing GitHub/localStorage state.
- P2: redirect/proxy policy, dev gating, commit/cleanup/relaunch semantics and exact ACL expansion were underspecified.

## Adopted changes

The revised design pins production trust anchors to repository configuration, forbids runtime endpoint override and unsigned fallback, separates signed candidates from GitHub/cache state, requires exact ACL auditing and no shell/process execute, defines release-owner hard-fail conditions, adds redirect/proxy/target/version restrictions, and distinguishes pre-commit failure from post-commit relaunch failure. It also records that SSH reference output is not remote success evidence.

## Escalation retained

Key rotation/revocation, enterprise proxy policy and post-commit recovery semantics remain explicit release-owner/consultant decisions before production enablement; local implementation must stay disabled/fail-closed until decided.
