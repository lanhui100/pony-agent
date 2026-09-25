# PA-101 PA-099 安全与发版流水线三件套（F1 open_url 白名单 + F2 CSP/DOMPurify + F3 版本同步与发版约定）

- Task ID: PA-101
- 标题: PA-099 Follow-ups F1~F3——Rust open_url 白名单、CSP 收紧与 DOMPurify 评估、发版流水线约定与版本同步修复
- 状态: Ready（T3，已建卡；spec 见 `openspec/changes/pa101-update-followups/`，待双审）
- 复杂度: B（安全敏感：外部 URL 执行、CSP、HTML 消毒；跨前端/Rust/工程流程）
- 负责人: @orchestrator + implementation；审核：spec 双审（含安全视角）→ code 双审 + 安全审
- 创建时间: 2026-09-16
- Next Action: spec 双路对抗审核（安全边界 + 架构一致性）
- Resume Hint: 现场证据见本卡"基线"节；F1/F2/F3 可按任务拆解并行，门禁串行

## 背景

PA-099（应用内更新检测）已收口，但 tasks.md 登记三个独立 follow-ups（`openspec/changes/archive/2026-08-24-add-app-update-check/tasks.md` §Follow-ups），当时以"独立后续任务，不阻塞本特性"明确不堵 Done。本卡将其收敛为可执行任务。

## 基线（2026-09-16 实测）

- F1：`src-tauri/src/lib.rs:422 open_url(url: String)` 直通 `platform::open_url_in_browser`（`src-tauri/src/platform.rs:80-95`）；Windows 用 `cmd /c start <url>`（仅 `&`→`^&` 转义），macOS `open`、Linux `xdg-open`；无 scheme/host 校验、无返回值（调用方 `let _ =` 丢弃）。前端调用点 2 处：`ConfigGeneralSection.vue:65/131`（更新发布页构造式 URL + exa.ai 静态 URL）。
- F2：`src-tauri/tauri.conf.json:28 "csp": null`；`src/lib/markdown.ts:99-196 sanitizeMarkdownHtml` 为正则走查式消毒（script/style 整删、SAFE_TAGS unwrap、href/src 经 isSafeUrl）；`MarkdownRenderer.vue:308/322` 两处 `v-html`；无 dompurify 依赖。
- F3：`package.json 0.1.91` = `src-tauri/Cargo.toml 0.1.91` = `.version.json tauri 0.1.91` 三处一致；`tauri.conf.json "version": "0.1.0"` 从未进同步链（`scripts/bump-version.ps1` 只写 Cargo.toml + package.json + Cargo.lock，不管 tauri.conf.json）；core 0.1.89 独立；无 release/tag 约定文档（PA-099 proposal Non-Goals 只登记了意向）。

## 目标

1. F1：`open_url` Rust 侧 URL 白名单（fail-closed），替换 `cmd /c start` 脆弱调用。
2. F2：CSP 从 null 收紧到可用策略 + DOMPurify 评估结论（采用/不采用均需证据）。
3. F3：`tauri.conf.json` 版本入同步链 + 发版约定文档化（tag == package.json 版本）。

## 非目标

- 不做自动下载/安装/差量升级（PA-099 既定）。
- 不接 Tauri 官方 updater（签名/manifest，属独立 ADR）。
- 不改更新检测状态机/缓存/角标逻辑。

## 技术方案（spec 待审，可修订）

### F1 open_url 白名单
- `platform.rs` 新增 `is_allowed_open_url(url) -> bool`：解析 URL，scheme 白名单 `{https}`（http 是否放行待 spec 审定；file/data/javascript 一律拒绝），host 白名单 `{github.com, api.github.com, exa.ai}` + 前端调用点枚举（新增调用点必须同步加白名单，编译期注释约束）。
- Windows 用 `ShellExecuteW`（或 tauri-plugin-opener，若评估通过）替换 `cmd /c start`，消解转义与标题参数怪癖；macOS/Linux 保持 `open`/`xdg-open` 但同样先过白名单。
- `lib.rs:422` 改返回 `Result<(), String>`（拒绝时 Err，白名单外 fail-closed，日志脱敏只记 host 不记全 URL）。
- 单测：白名单矩阵（https 放行/github 域放行/http 待定/file/data/javascript/自定义 scheme 拒绝、畸形拒绝、大小写/尾点/子域混淆拒绝）。

### F2 CSP + DOMPurify
- CSP：`tauri.conf.json` 从 null 收紧为最小可用策略（`default-src 'self'`；`connect-src 'self' https://api.github.com`；`img-src`/`style-src` 按现有资源盘点；`script-src` 无内联——若 vite 产物需 inline 则登记豁免理由）。dev 与 prod 一致性说明。
- DOMPurify 评估：以红队用例集（`javascript:` href、`onerror` img、svg+onload、style expression、form action、嵌套畸形标签）对比现行 `sanitizeMarkdownHtml` vs DOMPurify；结论二选一（迁移/保留并补齐用例），不允许"评估了但无结论"。
- 单测/验证：CSP 不阻断现有页面（build 后冒烟）；消毒红队矩阵全绿。

### F3 版本同步与发版约定
- `bump-version.ps1` 同步链加 `tauri.conf.json` version（与 tauri 双向一致校验；dry-run 可见）。
- 本次一次性把 `tauri.conf.json 0.1.0` 修复为当前 tauri 版本（0.1.91），并补校验脚本（CI 或 pre-push 检查四处一致：package.json/src-tauri Cargo/tauri.conf.json/.version.json tauri）。
- ADR 0012 追加发版约定节：release tag 必须等于打 tag 时点 package.json 版本（可带 v 前缀），与 `parseVersionTag`/`isNewerVersion` 口径一致。

## 风险与回滚

- F1 白名单过严会打断 exa.ai 等合法跳转——调用点枚举+测试矩阵覆盖；回滚即恢复旧 platform.rs。
- F2 CSP 过严白屏——先 dry-run 采集违规再收紧；回滚即 csp:null。
- F3 版本号一次性修复——纯元数据，无行为影响。

## 测试计划

- Rust：`cargo test -p pony-agent-tauri` 白名单矩阵（或 src-tauri lib 相应目标）。
- 前端：vitest update 相关 spec 回归 + MarkdownRenderer 消毒矩阵 + vue-tsc。
- 工程：bump-version dry-run 四处一致；CI 校验脚本（如新增）跑通。

## 验收标准

1. 非白名单 URL 在 Rust 侧被拒绝（单测矩阵全绿），`cmd /c start` 无残留。
2. CSP 非 null 且应用可正常启动/更新检查可用；DOMPurify 评估有书面结论+红队矩阵。
3. `tauri.conf.json` 版本与同步链一致，发版约定写入 ADR。

## 审核记录

- （待）Spec 双审：安全边界 + 架构一致性。
- （待）Code 双审 + 安全审。
