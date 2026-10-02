# Signed desktop self-update design (PA-103)

## Decision gate and evidence boundary

C1 plan/spec review initially failed because this checkout has no production updater endpoint, public verification key, release workflow, or signed artifact fixture. SSH discovery is source-code/config reference only: remote output showed `desktop/src/composables/useUpdater.ts`, `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/capabilities/default.json`, and `desktop/src-tauri/src/lib.rs` containing the observed plugin/API pattern, but did not prove a successful remote release, signature, install, cleanup, or relaunch. Those are implementation references, not acceptance evidence.

Therefore PA-103 is split into two explicit gates:

- **Local contract gate (this change):** implement typed, fail-closed updater plumbing, UI/state tests, exact ACL/config shape, and release validation scaffolding. With no real Pony Agent signing key/endpoint available, installation stays disabled at runtime and browser/check-only behavior remains safe.
- **Release-owner gate (follow-up):** provision a Pony Agent signing key outside git, publish a fixed HTTPS manifest origin, configure CI secrets, produce signed artifacts, run Windows signed smoke, and flip the configuration to enabled. No fake key, borrowed pproxy key, unsigned fallback, or environment-controlled production endpoint is allowed.

The implementation must not claim one-click installation is production-ready until the release-owner gate has evidence.

## Trust anchors and release contract

When enabled, production uses a repository-fixed endpoint and public key checked into `src-tauri/tauri.conf.json`; runtime environment variables cannot override either. The endpoint is an HTTPS URL on a fixed origin/path owned by Pony Agent. The manifest's artifact URL must use the same HTTPS origin and the updater must reject unsupported targets, non-increasing versions, missing signatures, and signature/hash mismatch. Cross-origin or HTTP redirects are not accepted. The manifest and all target artifacts are published atomically only after the release tag, four synchronized versions, target matrix and signatures validate.

The release workflow (follow-up, not implied by this local change) must hard-fail when the signing secret is absent, signing fails, the public key does not match, a target artifact is missing, versions/tag diverge, or manifest validation fails. Private keys are CI secret material only; they may not occur in source, bundle, logs, fixtures used by production, or generated artifacts. Key rotation/revocation is a separate release-owner decision; until a protocol exists, only the single pinned key is accepted and rollback/downgrade is rejected.

## Source and state separation

The existing GitHub API/localStorage path is named `githubLatest` and is check-only metadata. It may drive a release-page CTA, never an install CTA. A signed updater candidate is a separate `signedCandidate` obtained from the Tauri updater adapter and is the only source allowed to drive `install`. Cache hydration cannot authorize installation. The adapter accepts/returns a typed updater handle; no URL string, `html_url`, asset URL, or user-controlled endpoint can reach install.

Production Tauri release builds may expose install only when the build has signed updater configuration. Tauri debug/dev builds, Vite preview and ordinary browser builds never expose install. A GitHub API result must not be converted into a signed candidate.

## User flow and state machine

1. Startup performs the existing non-blocking check-only path.
2. In a configured Tauri production build, `checkSignedUpdate()` obtains a signed updater candidate. It records candidate metadata separately from `githubLatest`.
3. Settings renders `立即升级` only for a signed candidate and only when no operation is active.
4. The user clicks once: `idle/available -> downloading -> installing -> pending-restart`; repeated check/install clicks are ignored. Progress is monotonic 0–100; unknown content length is represented as indeterminate.
5. The adapter performs plugin download/install. Before the host exits, the narrow `prepare_update_exit` command cleans only Pony Agent-owned runtime resources and is idempotent. The process plugin then relaunches.
6. A successful plugin commit is never reported as ordinary failure. If commit status cannot be distinguished from relaunch failure, the UI reports `pending-restart`/`relaunch-failed` and preserves the candidate for the next startup; it never claims `installed` merely because an in-memory promise resolved.
7. Signature, manifest, target, version, endpoint, network, disk, cleanup-before-commit, or download failures never call relaunch and never create an installable candidate. After commit, cleanup/relaunch failures are reported distinctly and are not retried as if no install occurred.

A durable pending marker/startup reconciliation is required before production enablement if the plugin cannot itself guarantee recovery after crash/forced termination. Active turns must be blocked or explicitly confirmed before install; this local contract does not silently abort user work.

## Boundaries and exact permissions

- `src/lib/update-check.ts` remains the pure GitHub compatibility path.
- `src/lib/tauri-updater.ts` contains dynamic imports and a typed adapter; it is unavailable outside Tauri and has no endpoint parameter.
- `src/stores/update.ts` owns source-separated state and operation token/concurrency guards.
- `src-tauri` registers only updater and process plugins plus an idempotent cleanup command; the command is scoped to the main window capability and never invokes a shell or arbitrary process API.
- Capability permissions are explicitly audited against the installed plugin schema: only updater check/download/install (or the plugin's exact combined operation) and `process:allow-restart`; no shell, process execute, or process default permission.
- Frontend CSP is not widened to arbitrary HTTPS. Updater networking remains in the Tauri plugin/Rust host; no updater `fetch`, `window.open`, or external URL fallback is permitted.

## Platform matrix

| mode/platform | check-only | signed install | evidence required |
|---|---:|---:|---|
| ordinary browser/Vite preview, all OS | yes (GitHub compatibility) | no | unit/component tests |
| Tauri debug/dev, all OS | optional check-only | no | debug gate test |
| Tauri production Windows x64 NSIS | yes | release-owner gate | signed fixture + real Windows smoke |
| Tauri production Windows arm64 | only if artifact exists | only if artifact exists | target artifact/installer evidence |
| Tauri production macOS/Linux | no install until target/signing/notarization contract exists | no | explicit disabled UI test |

## Test and release evidence

Local gate commands are exact: `npm run version:check`, `npm run test:unit -- --run tests/update-check.spec.ts tests/update-store.spec.ts tests/ConfigGeneralSectionUpdate.spec.ts`, `npm run typecheck`, `npm run build`, `npm run cargo:check:shared`, and `npm run verify` where unrelated baseline changes permit. Tests use a fake typed adapter to inject: no update, signed candidate, wrong key/manifest/artifact, non-increasing version, redirect/HTTP rejection, download truncation/unknown length, cleanup-before-commit failure, post-commit relaunch failure, concurrent clicks, browser/dev no-op, and late promise results. UI selectors cover check/install buttons, progress, pending-restart and error states.

A fake signed fixture proves adapter contract only; it does not prove Tauri signature verification. Release-owner evidence must include manifest/artifact hashes, pinned public-key fingerprint, Windows x64 installer, tampered-artifact rejection, wrong-key rejection, install/relaunch version evidence, user-data preservation, and logs redacted of secrets. No release evidence is fabricated in the local gate.

## Rollback and operations

No client-side downgrade/rollback is supported. Release rollback means withdrawing the manifest/release through the fixed release channel; key compromise requires a separately reviewed rotation/revocation protocol. Partial downloads are temporary and cleaned by the plugin/host. Endpoint, key, proxy and manifest failures are fail-closed with safe user-facing errors that do not expose tokens or sensitive URLs.
