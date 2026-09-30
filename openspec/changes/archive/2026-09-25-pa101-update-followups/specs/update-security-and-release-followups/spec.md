# Delta spec: update-security-and-release-followups

## ADDED Requirements

### Requirement: open_url SHALL enforce an allowlist and fail closed

The `open_url` Tauri command SHALL only open URLs whose scheme is `https` (`http` is permanently rejected) and whose lowercased `host_str()` (parsed by the `url` crate; hand-rolled parsing is forbidden) exactly matches one of `{github.com, api.github.com, exa.ai}`. Pre-checks SHALL reject leading/trailing whitespace anomalies containing `\r\n\t`, non-ASCII hosts (IDN/punycode lookalikes), trailing-dot hosts, and any URL carrying userinfo or an explicit port (including `:443`). Non-allowlisted calls SHALL return `"url_allowlist_rejected:<host>"` and refuse execution (fail-closed); rejection logs SHALL contain only scheme+host+reason (never path/query/fragment/full URL).

#### Scenario: Non-allowlisted URL is rejected without execution

- **GIVEN** an `open_url` call with `file:///etc/passwd`
- **WHEN** the Rust command executes
- **THEN** it returns `Err("url_allowlist_rejected:…")`
- **AND** no browser/shell process is spawned.

#### Scenario: Lookalike hosts are rejected

- **GIVEN** URLs `https://github.com.evil.test/`, `https://github.com./`, `https://github.com@evil.com/`, `https://github.com:443/`, `http://github.com/`
- **WHEN** the allowlist check runs
- **THEN** every one is rejected.

### Requirement: Frontend SHALL NOT fall back to window.open on allowlist rejection

When `open_url` fails with `url_allowlist_rejected:*`, the frontend SHALL NOT retry via `window.open` (that would turn fail-closed into fail-open); it SHALL only warn. `window.open` fallback is allowed solely for the legacy-binary case (`open_url_unsupported`). All frontend `open_url` invocations SHALL go through a single `openExternalUrl()`出口 (direct `safeInvoke("open_url")` is forbidden and gated by grep).

#### Scenario: Rejected URL stays closed in UI

- **GIVEN** Rust rejects a URL with `url_allowlist_rejected`
- **WHEN** `openReleasePage` handles the error
- **THEN** no new browser window/tab opens.

### Requirement: CSP SHALL be enforced with dev/prod split

The Tauri app SHALL ship a non-null CSP in production (`default-src 'self'`, `connect-src` limited to `api.github.com` et al, `object-src 'none'`, `frame-src 'none'`); dev SHALL use a separate local-relaxed policy (ws/HMR on 127.0.0.1); browser-preview SHALL be covered by an `index.html` meta CSP. Architectural invariant: model traffic NEVER goes through frontend fetch (only `update-check.ts` fetches `api.github.com`); any new frontend `fetch(` outside update-check SHALL fail the gate.

#### Scenario: Four-state smoke passes

- **GIVEN** the CSP change merged
- **WHEN** running tauri dev / tauri build artifact / vite preview / browser preview
- **THEN** the app boots and update check works in all four.

### Requirement: Version sync chain SHALL cover tauri.conf.json and release tags

`tauri.conf.json` version SHALL join the sync chain with package.json, src-tauri Cargo.toml and `.version.json` tauri (four-way consistent, gated by `check-version-sync.ps1` as a CI MUST that cannot be bypassed with `--no-verify`); `bump-version.ps1` SHALL write it via text-level key replacement (no `ConvertTo-Json` rewrite); release tags SHALL equal the package.json version at tag time (optional `v` prefix). The one-time `0.1.0 → 0.1.91` fix SHALL be preceded by a shipped-artifact/tag inventory.

#### Scenario: Version drift fails CI

- **GIVEN** `tauri.conf.json` version differs from package.json
- **WHEN** CI runs `check-version-sync.ps1`
- **THEN** it exits non-zero and blocks merge.

## MODIFIED Requirements

### Requirement: open_url command signature SHALL return Result

`open_url` SHALL change from fire-and-forget `()` to `Result<(), String>` with machine-readable codes (`url_allowlist_rejected:<host>` vs `open_url_unsupported`); both frontend call sites (`openReleasePage`, `openExa`) SHALL handle the codes and use `safeInvoke<void>`.

#### Scenario: Legacy binary still opens known URLs

- **GIVEN** an old host binary without the command
- **WHEN** the frontend invokes it
- **THEN** it gets `open_url_unsupported` and MAY fall back to `window.open`.

### Requirement: Markdown sanitization SHALL be pinned by a red-team matrix

Markdown HTML sanitization SHALL be pinned by `tests/markdown-sanitize.redteam.spec.ts` (jsdom; covers `javascript:` obfuscations, event handlers, svg/math nesting, `a ping`, `base` hijack, `srcset`/`data:`/`blob:` variants, mXSS differentials, DOM clobbering, malformed nesting). If the hand-rolled `sanitizeMarkdownHtml` is kept, every matrix case SHALL be locked as regression (including the non-DOM early-return contract decided first); DOMPurify adoption SHALL include bundle-size/build data.

#### Scenario: XSS vector is neutralized

- **GIVEN** markdown containing `<img src=x onerror=alert(1)>`
- **WHEN** rendered
- **THEN** no event handler survives in the output HTML.

## REMOVED Requirements

### Requirement: cmd /c start launcher and null CSP SHALL be removed

The `cmd /c start` URL path (including `Command::new("cmd")`) and `csp: null` SHALL be removed; grep gates SHALL assert zero hits.

#### Scenario: Residual launcher is caught

- **GIVEN** a codebase search for `cmd.*/c.*start` or `Command::new("cmd")`
- **WHEN** the gate runs
- **THEN** it reports zero matches.
