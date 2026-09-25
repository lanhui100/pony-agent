# 0017 发版流水线约定、四处版本一致性同步与安全加固

Status: implemented

## 背景

[0012](superseded/0012-app-update-check-via-github-releases.md) 建立了基于 GitHub Releases 的前端检测与构造式跳转，但当时留存三个关键技术敞口（任务卡 `PA-099` 遗留，由 [PA-101](file:///D:/Documents/pony-agent/management/task-system/03_TASKS/PA-101-update-security-and-release-followups.md) 与 OpenSpec 变更 `pa101-update-followups` 承接）：
1. `tauri.conf.json` 中的版本号恒为 `0.1.0`，从未纳入 `bump-version.ps1` 同步链，与 `package.json` 及 `Cargo.toml` 严重失同步；
2. 缺乏形式化的发版约定与流水线硬约束，Git tag 与版本口径可能漂移导致更新检测误报或漏报；
3. `open_url` 命令在 Rust 侧无协议与域名校验（直通 Windows `cmd /c start`），且前端 WebView 缺少 CSP 策略保护，Markdown 消毒依赖脆弱的手写正则。

## 候选方案

**方案 A：四处版本严格同步 + CI 服务端只读门禁 + 发版 tag 对齐（选定）**——
- 同步链包含 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 四处，`bump-version.ps1` 采用文本正则行级替换写回 `tauri.conf.json`（避免 `ConvertTo-Json` 打乱格式与缩进）；
- 新增 `scripts/check-version-sync.ps1` 并在 CI 流程中作为 MUST 门禁执行（不可被客户端 `--no-verify` 绕过）；
- 发版 tag 严格约束为 `vX.Y.Z`，对应打 tag 时点四处一致的版本号；
- Rust 侧实施基于 `url` crate 的严格白名单校验（仅 https、精确 host 匹配），Windows 锁死 `ShellExecuteW`；
- 前端收紧 CSP 策略并实测引入 DOMPurify 替换手写正则。
- 优点：版本单一事实源明确，构建与打包版本完全闭环，消除 URL 注入与 XSS 绕过风险。
- 缺点：新增依赖 `dompurify`（体积增量约 27KB / gzip 10KB，已实测评估通过）。

**方案 B：仅客户端 pre-push 钩子保证同步，不加 CI 服务端门禁**（落选）——
客户端钩子可被 `--no-verify` 或非标准终端环境绕过，无法保证合并至主干的代码必定四处一致。

**方案 C：tauri.conf.json 保持解耦独立发版**（落选）——
桌面安装包版本号与前端关于页面版本号脱节，给用户诊断与更新提醒带来混淆。

## 决策

1. **接替 [0012](superseded/0012-app-update-check-via-github-releases.md)**：承接其前端更新检测设计，重述并强化版本与流水线约束。
2. **四处版本一致性**：仓库内 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 保持严格一致。一次性修复历史残留的 `0.1.0` 为 `0.1.91`。
3. **版本工具链升级**：
   - `scripts/bump-version.ps1` 在更新 tauri 目标时，以键级正则替换同步修改 `tauri.conf.json` 的 `"version"` 字段；
   - 增加只读检查脚本 `scripts/check-version-sync.ps1`，并在 `.github/workflows/ci.yml` 与 `npm run verify` 中接入。
4. **发版 tag 契约**：发布 Release 时的 Git tag 必须严格等于 `v${version}`（其中 `${version}` 与打 tag 时点的 `package.json` 版本一致）。
5. **外部 URL 与内容安全防御**：
   - `open_url` 在 Rust 侧强制经 `url` crate 解析，严格限定 scheme 为 `https`，host 精确限定为 `github.com`、`api.github.com`、`exa.ai`；Windows 平台使用 `ShellExecuteW` 替代 `cmd /c start`；
   - 前端通过 `openExternalUrl()` 统一收口，Rust 白名单拒绝时禁止 `window.open` 回退（fail-closed）；
   - `tauri.conf.json` 收紧生产环境 CSP 策略；
   - Markdown HTML 消毒经红队 29 项测试验证，正式迁移至 DOMPurify。

## 影响

- 彻底消除应用桌面包与内部配置的版本脱节现象。
- 阻断任意外部 URL 调用的命令注入风险。
- CI 增加四处版本同步的硬性校验。
