# Tasks: pa101-update-followups

> 执行跟踪卡：`management/task-system/03_TASKS/PA-101-update-security-and-release-followups.md`
> 并行边界（双审修订）：F1 独立并行；F2/F3 对 `src-tauri/tauri.conf.json` 串行——F2 先合入 CSP 文本，F3 再做版本写回 + 全量冒烟（同一 JSON 文件两字段，并行必冲突；bump 写回若重排会覆盖 CSP）。§4 门禁串行。

## §1 F1 open_url 白名单

- [x] 1.1 `platform.rs` 新增 `is_allowed_open_url`（强制 `url` crate，禁手写解析）+ 单测矩阵（放行/拒绝/IDN/控制符/尾点/userinfo/显式端口/子域冒充/scheme 混淆/畸形）
- [x] 1.2 Windows 改 ShellExecuteW（锁死，不引 opener 插件；verb=open、返回值>32、不经 shell）+ macOS/Linux 白名单前置（`--` 隔离）；`cmd /c start` 彻底删除 + grep 门禁（`cmd.*/c.*start` + `Command::new("cmd")` 零命中）
- [x] 1.3 `lib.rs open_url` 改 `Result<(), String>` fail-closed（错误码：`url_allowlist_rejected:<host>` vs `open_url_unsupported`）+ 拒绝日志仅 scheme+host+reason
- [x] 1.4 前端：新增 `openExternalUrl()` 统一出口（禁直接 `safeInvoke("open_url")` + grep 门禁）；`openReleasePage` 白名单拒绝禁 `window.open` 回退（仅旧二进制缺命令可回退）；`openExa` 补 catch + `safeInvoke<void>` 类型收紧

## §2 F2 CSP + DOMPurify

- [x] 2.1 `tauri.conf.json` CSP 收紧（prod enforce；dev 另行放宽本地 ws/HMR；浏览器预览走 `index.html` meta CSP；删 report-only，直接四态冒烟：tauri dev / build 产物 / vite preview / 浏览器预览）
- [x] 2.2 消毒红队矩阵钉死 `tests/markdown-sanitize.redteam.spec.ts`（jsdom 前提；现行实现先跑，不通过项定去留；非 DOM 早返旁路契约先定）
- [x] 2.3 DOMPurify 采用/保留书面结论（不允许无结论；若采用含包体积/构建数据）

## §3 F3 版本同步与发版约定

- [x] 3.1 `bump-version.ps1` 纳入 `tauri.conf.json`（文本键级替换写回，禁 ConvertTo-Json 重排）+ dry-run 四处 diff 可见
- [x] 3.2 先盘点已发布产物/tag，再一次性修复 `0.1.0 → 0.1.91`
- [x] 3.3 新增 `check-version-sync.ps1`（只读不写，双态 exit code）+ CI MUST 接入（服务端不可被 `--no-verify` 绕过；pre-push 仅可选加速）
- [x] 3.4 ADR 0012 发版约定节（取代规则：新增 ADR + 互链，禁改写正文；verify-decisions 同步）

## §4 门禁

- [x] 4.1 Rust 白名单矩阵 + src-tauri lib 回归 + cmd 残留 grep 门禁
- [x] 4.2 前端 update 回归 + 消毒矩阵 + vue-tsc + build + 四态冒烟（dev/build/preview/浏览器预览）+ 新增 fetch/open_url 直调 grep 门禁
- [x] 4.3 bump dry-run + 版本一致校验双态
- [x] 4.4 收口：canonical spec 同步 + change 归档 + 任务板更新

## 收口记录（2026-09-30）

全部 15 项已完成并通过实测复核。本文件此前遗留未勾选，属收尾流程缺口，本次一并补齐。

实测证据（2026-09-30 复核）：

- `src-tauri/src/platform.rs:81` `ALLOWED_OPEN_HOSTS = ["github.com", "api.github.com", "exa.ai"]`、`:88 is_allowed_open_url`、`:192 ShellExecuteW` FFI 绑定；白名单矩阵测试在 `:516` 起。
- `src/lib/open-external-url.ts` 为唯一出口（`safeInvoke("open_url")` 仅出现在该文件）；`ConfigGeneralSection.vue:65/121` 两处调用点均走 `openExternalUrl`。
- 前端 `fetch(` 全仓仅 `src/lib/update-check.ts:140` 一处，符合"模型流量不走前端 fetch"不变量。
- `src-tauri/tauri.conf.json:28` CSP 非 null（`object-src 'none'`、`frame-src 'none'`）；`index.html:10` 有浏览器预览 meta CSP。
- `dompurify ^3.4.16` 在 dependencies；`tests/markdown-sanitize.redteam.spec.ts` 存在（29/29）。
- `scripts/check-version-sync.ps1` 存在并接入 `.github/workflows/ci.yml:38`；ADR `0017` 已落地并接替 0012。

canonical spec：`openspec/specs/update-security-and-release-followups/spec.md`（本次补齐）。
