# update-security-and-release-followups Specification

## Purpose

规范 Pony Agent 在应用内更新能力（见 `app-update-check`）之上补强的三组安全与发版契约：外部 URL 打开的 Rust 侧 fail-closed 白名单与统一前端出口、生产 CSP 强制与 Markdown 消毒的 DOMPurify 迁移、以及四处版本一致性同步链与发版 tag 约定。对应任务卡 `PA-101`、ADR [0017](file:///D:/Documents/pony-agent/docs/decisions/0017-release-tag-convention-and-tauri-version-sync.md)（接替 0012）。

## Requirements

### Requirement: open_url SHALL enforce an allowlist and fail closed
`open_url` Tauri 命令 SHALL 仅打开 scheme 为 `https` 的 URL（`http` 永久拒绝），且其 `host_str()`（必须经 `url` crate 解析，禁止手写解析）小写后精确匹配 `{github.com, api.github.com, exa.ai}` 之一。前置检查 SHALL 拒绝含 `\r\n\t` 的首尾空白异常、非 ASCII host（IDN/punycode 冒充）、尾点 host，以及任何携带 userinfo 或显式端口（含 `:443`）的 URL。非白名单调用 SHALL 返回 `"url_allowlist_rejected:<host>"` 并拒绝执行（fail-closed）；拒绝日志 SHALL 只记录 scheme+host+reason（绝不记录 path/query/fragment/完整 URL）。

#### Scenario: Non-allowlisted URL is rejected without execution
- **GIVEN** 一次携带 `file:///etc/passwd` 的 `open_url` 调用
- **WHEN** Rust 命令执行
- **THEN** 返回 `Err("url_allowlist_rejected:…")`
- **AND** 不产生任何浏览器/shell 进程

#### Scenario: Lookalike hosts are rejected
- **GIVEN** URL 为 `https://github.com.evil.test/`、`https://github.com./`、`https://github.com@evil.com/`、`https://github.com:443/`、`http://github.com/`
- **WHEN** 白名单校验运行
- **THEN** 全部被拒绝

### Requirement: Frontend SHALL NOT fall back to window.open on allowlist rejection
当 `open_url` 以 `url_allowlist_rejected:*` 失败时，前端 SHALL NOT 通过 `window.open` 重试（否则将 fail-closed 变成 fail-open），只 SHALL 告警。`window.open` 回退仅允许用于旧二进制缺命令的情况（`open_url_unsupported`）。所有前端 `open_url` 调用 SHALL 经唯一出口 `openExternalUrl()`（直接 `safeInvoke("open_url")` 被禁止并由 grep 门禁约束）。

#### Scenario: Rejected URL stays closed in UI
- **GIVEN** Rust 以 `url_allowlist_rejected` 拒绝了某 URL
- **WHEN** `openReleasePage` 处理该错误
- **THEN** 不打开任何新的浏览器窗口/标签页

### Requirement: CSP SHALL be enforced with dev/prod split
Tauri 应用 SHALL 在生产环境下发非 null 的 CSP（`default-src 'self'`；`connect-src` 限于 `api.github.com` 等；`object-src 'none'`；`frame-src 'none'`）；dev SHALL 使用独立的本地放宽策略（127.0.0.1 上的 ws/HMR）；浏览器预览 SHALL 由 `index.html` 的 meta CSP 覆盖。架构不变量：模型流量永不经过前端 fetch（仅 `update-check.ts` 访问 `api.github.com`）；`update-check` 之外任何新增前端 `fetch(` SHALL 触发门禁失败。

#### Scenario: Four-state smoke passes
- **GIVEN** CSP 变更已合入
- **WHEN** 分别运行 tauri dev / tauri build 产物 / vite preview / 浏览器预览
- **THEN** 四种形态下应用均可启动且更新检查可用

### Requirement: Version sync chain SHALL cover tauri.conf.json and release tags
`tauri.conf.json` 版本 SHALL 加入同步链，与 `package.json`、`src-tauri/Cargo.toml`、`.version.json` 的 tauri 段保持四处一致，由 `check-version-sync.ps1` 作为 CI MUST 门禁校验（不可被 `--no-verify` 绕过）；`bump-version.ps1` SHALL 以文本键级替换方式写回该文件（禁止 `ConvertTo-Json` 重排）。发布 tag SHALL 等于打 tag 时点的 `package.json` 版本（可带 `v` 前缀）。core 版本独立于 tauri 版本，由 `.version.json` 的 core 段与 `crates/pony-agent-core/Cargo.toml` 两处一致约束。一次性 `0.1.0 → 0.1.91` 修复 SHALL 以已发布产物/tag 盘点为前提。

#### Scenario: Version drift fails CI
- **GIVEN** `tauri.conf.json` 版本与 `package.json` 不一致
- **WHEN** CI 运行 `check-version-sync.ps1`
- **THEN** 脚本以非零退出码结束并阻断合并

### Requirement: Markdown sanitization SHALL be pinned by a red-team matrix
Markdown HTML 消毒 SHALL 由 `tests/markdown-sanitize.redteam.spec.ts`（jsdom 前提）钉死，覆盖 `javascript:` 混淆、事件处理器、svg/math 嵌套、`a ping`、`base` 劫持、`srcset`/`data:`/`blob:` 变体、mXSS 差分、DOM clobbering 与畸形嵌套。现行实现为 DOMPurify 迁移（`dompurify@3.4.16`），取代手写正则 `sanitizeMarkdownHtml`；迁移 SHALL 附带包体积/构建数据。

#### Scenario: XSS vector is neutralized
- **GIVEN** markdown 含 `<img src=x onerror=alert(1)>`
- **WHEN** 渲染
- **THEN** 输出 HTML 中不残留任何事件处理器

### Requirement: cmd /c start launcher and null CSP SHALL be removed
`cmd /c start` 的 URL 打开路径（含 `Command::new("cmd")`）与 `csp: null` SHALL 被移除；grep 门禁 SHALL 断言零命中。Windows 平台 SHALL 使用 `ShellExecuteW`（verb=open、返回值 > 32、不经 shell）。

#### Scenario: Residual launcher is caught
- **GIVEN** 在代码库中搜索 `cmd.*/c.*start` 或 `Command::new("cmd")`
- **WHEN** 门禁运行
- **THEN** 报告零命中
