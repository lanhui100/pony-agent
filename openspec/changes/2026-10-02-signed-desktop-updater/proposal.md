# Signed desktop self-update (PA-103)

## Why

The desktop currently detects GitHub Releases and opens a release page, but it cannot install a signed update. SSH discovery of `dev:~/pproxy/desktop` observed a Tauri updater pattern (plugin calls, configuration and permissions), but did not prove a successful remote release, signature, installation, cleanup or relaunch. Pony Agent will adopt the pattern only behind an explicit local-contract gate and a separate release-owner enablement gate.

## What Changes

- Add the Tauri updater and process plugins to the desktop host and grant only the exact updater/process permissions required by the UI.
- Add a repository-fixed, HTTPS-only production endpoint/public-key contract without committing a fake key; runtime configuration cannot override the trust anchor. If the Pony Agent endpoint/key/release workflow is not provisioned, production installation remains disabled and fails closed.
- Separate GitHub check-only metadata from signed updater candidates. Only a typed Tauri updater handle may reach download/install; browser, Vite preview and Tauri dev builds remain check-only.
- Extend the existing update store with a Tauri-only `check -> download -> install -> pending-restart/relaunch` path, progress and truthful failure states; do not use `html_url`, arbitrary assets or unsigned fallback.
- Add tests for source separation, state transitions, concurrency, unsupported/browser/dev behavior, signature/manifest failure injection and failure preservation; add release/config documentation and an ADR.
- Treat real key provisioning, signed artifacts, endpoint publication and Windows smoke as a separate release-owner gate; do not fabricate evidence in this change.

## Capabilities

### New Capabilities

- `signed-desktop-updater`: a user-triggered, signature-verified desktop update check and one-click installation flow.

### Modified Capabilities

- `app-update-check`: retains the existing check-only/browser behavior and exposes the installed update action in Tauri mode.

## Impact

Tauri Rust dependencies/config/capabilities, frontend update types/store/settings card, package lock, tests, release documentation and CI/release configuration. No changes to agent-core, sandbox policy, workspace permissions or generic external URL allowlists.

## Non-goals

- No unsigned installer download, shell command, self-replacement script or frontend `window.open` fallback for installation.
- No automatic silent installation, background timer, rollback engine or release signing-key generation.
- No widening of `open_url` host allowlists to support updater downloads.
- No claim that an update is available when only the browser-mode API check succeeded; Tauri installation metadata is accepted only from the signed updater plugin.
