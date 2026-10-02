# PA-103 implementation review — security & trust boundary

- Date: 2026-10-02
- Reviewer: independent security reviewer (two passes; one verified against local registry tauri-plugin-updater 2.13.1 source)
- Verdict: **CONDITIONAL PASS → PASS after fixes**

## Source-verified facts

- Empty/absent updater config parses fine; plugin is lazy, no startup panic; minisign verification rejects bogus keys.
- `plugin:updater|check` returns camelCase `{rid,currentVersion,version,date,body,rawJson}` — matches `parseSignedCandidate`.
- `plugin:updater|download_and_install` args `rid/onEvent/headers/timeout` — matches adapter; events `Started{contentLength}/Progress{chunkLength}/Finished`.
- Capabilities grant exactly `updater:allow-check`, `updater:allow-download-and-install`, `process:allow-restart`; no shell/execute; gen/schemas in sync.

## Findings adopted

- P1: removed empty placeholder `plugins.updater{endpoints:[],pubkey:""}` block from `tauri.conf.json`; aligned lib.rs comment and ADR 0019 to "config absent, plugin registered for capability resolution, frontend disabled".
- P1: enabled-path zero tests → added fake-adapter store tests and component tests (without flipping the production constant).
- P2: dead `allowDowngrades` parameter removed (not part of the plugin check command).
- P2: error copy sanitized (fixed message; raw plugin error with possible URLs only in console).
- P2: failed install now clears the uncommitted candidate (no stale installable-looking state); relaunch-failed keeps the committed candidate.

## Escalated / release-gate

Key rotation/revocation protocol, enterprise proxy policy, post-commit crash recovery, real signed fixture + Windows smoke, exact plugin version pinning, and `prepare_update_exit`/active-turn confirmation all remain release-owner gate items; local contract stays disabled and fail-closed.
