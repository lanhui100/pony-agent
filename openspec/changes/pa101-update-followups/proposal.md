# Proposal: pa101-update-followups（PA-101）

> PA-099 Follow-ups F1~F3 收敛：F1 Rust `open_url` 白名单、F2 CSP 收紧与 DOMPurify 评估、F3 发版流水线约定与版本同步修复。执行跟踪卡：`management/task-system/03_TASKS/PA-101-update-security-and-release-followups.md`。

## Why

PA-099 收口时登记三个独立 follow-ups（见归档 `openspec/changes/archive/2026-08-24-add-app-update-check/tasks.md` §Follow-ups），当时明确"不阻塞本特性"。现状三处风险/债务仍然敞口：

1. `open_url` Tauri 命令对任意字符串直接 `cmd /c start`（Windows 仅转义 `&`），前端传入的 URL 虽为构造式，但 Rust 侧无第二道防线——任何未来调用点（或 IPC 伪造）都可执行任意 URL（含 `file:`/`javascript:` 语义，取决于 shell 解析）。
2. `tauri.conf.json` 的 `csp: null` 使 WebView 无内容安全策略；`sanitizeMarkdownHtml` 为手写正则消毒，未经红队矩阵验证，DOMPurify 评估当时只登记了意向。
3. `tauri.conf.json` version 恒为 `0.1.0`，从未进入 `bump-version.ps1` 同步链；release tag 与 package.json 一致性只有 proposal Non-Goals 一句话，无 ADR 约束、无校验脚本。

## What Changes

1. **F1**：`src-tauri/src/platform.rs` 新增 URL 白名单（`https` 唯一 scheme，`http` 永拒；`{github.com, api.github.com, exa.ai}` host 精确匹配，IDN/控制符/尾点/userinfo/显式端口/子域冒充拒绝），Windows 锁死 `ShellExecuteW`（不引 opener 插件），`lib.rs open_url` 改 `Result` fail-closed（白名单拒绝禁前端 `window.open` 回退）。统一出口 `openExternalUrl()` + grep 门禁。
2. **F2**：CSP 从 null 收紧为最小可用策略（prod enforce；dev 另行放宽本地 ws/HMR；浏览器预览走 `index.html` meta CSP；`connect-src` 限定 `api.github.com`，架构不变量：模型请求永不走前端 fetch）；DOMPurify 红队评估并给出采用/保留结论（矩阵钉死 `tests/markdown-sanitize.redteam.spec.ts` jsdom 前提）。
3. **F3**：`bump-version.ps1` 同步链纳入 `tauri.conf.json`（文本键级替换写回）；先盘点已发布产物/tag 再一次性修复 `0.1.0 → 0.1.91`；新增四处版本一致校验（CI MUST，服务端不可绕过）；ADR 0012 追加发版约定（tag == package.json 版本，可带 v 前缀）。

## Scope

- 修改：`src-tauri/src/platform.rs`、`src-tauri/src/lib.rs`（open_url 签名）、`src-tauri/tauri.conf.json`（csp + version）、`scripts/bump-version.ps1`、ADR 0012、发版约定文档；
- 可选新增：DOMPurify 依赖（评估后）、版本一致校验脚本；
- 明确不新增：tauri-plugin-opener 依赖（已否决，锁死 ShellExecuteW）；URL 解析强制 `url` crate（禁手写解析）。
- 测试：Rust 白名单矩阵 + 前端 update 回归 + 消毒红队矩阵 + vue-tsc + build 冒烟。

## Non-Goals

- 不做自动下载/安装/差量升级；不接 Tauri 官方 updater；不改更新检测状态机/缓存/角标逻辑。
- 双审已拍板（以 design/spec 为准）：http 永拒；执行器锁死 ShellExecuteW（不引 opener 插件）。

## Risks

- 白名单过严打断合法跳转 → 统一出口 + 调用点枚举 + 矩阵覆盖。
- CSP 过严白屏 → dev/prod 双策略 + 四态冒烟（tauri dev / build 产物 / vite preview / 浏览器预览），可回滚 null。
- 版本号一次性修复影响 bundle/OS 可见版本 → 先盘点已发布产物/tag 再修复。

## Rollback

各 F 独立回滚：platform.rs 恢复旧实现；tauri.conf.json csp 回 null、version 回滚；bump 脚本 revert。无数据迁移。

## 验收标准

1. 非白名单 URL 在 Rust 侧被拒绝（矩阵全绿），`cmd /c start` 无残留（grep 门禁）；白名单拒绝禁前端 `window.open` 回退（回归锁定）。
2. CSP 非 null 且四态冒烟通过（dev/build/preview/浏览器预览），更新检查可用；DOMPurify 评估有书面结论 + 红队矩阵全绿。
3. 四处版本一致（CI MUST 门禁），发版约定写入 ADR（含产物盘点记录）。
