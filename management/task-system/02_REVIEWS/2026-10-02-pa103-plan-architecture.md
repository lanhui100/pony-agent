# PA-103 C1 plan/spec adversarial review — architecture

- Reviewer: independent architecture reviewer
- Scope: proposal/design/tasks/ADR before implementation
- Verdict: **PASS WITH FINDINGS → PASS after revision**

## Findings

- P1: GitHub/cache metadata and signed installability were initially described too close together; a cache-derived `available` state must not authorize install.
- P1: Production trust anchors and release-owner enablement were not concrete enough to verify.
- P2: Dev/browser/release gates, platform matrix, install/relaunch lifecycle and exact permission audit needed explicit contracts.

## Adopted changes

The design now separates `githubLatest` from `signedCandidate`, makes a typed updater handle the only install input, adds a local-contract versus release-owner gate, freezes browser/Vite/Tauri-dev check-only behavior, defines the platform matrix, exact permission audit requirement, state/lifecycle boundaries, and evidence requirements. The proposal now says remote SSH evidence is source/config observation only and does not claim a successful remote release.

## Unadopted items

No reviewer finding was rejected. Key rotation and production release workflow remain a release-owner gate because no real Pony Agent signing key or endpoint is available in this checkout.
