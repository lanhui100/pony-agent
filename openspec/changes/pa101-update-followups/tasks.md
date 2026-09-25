# Tasks: pa101-update-followups

> 执行跟踪卡：`management/task-system/03_TASKS/PA-101-update-security-and-release-followups.md`
> 并行边界（双审修订）：F1 独立并行；F2/F3 对 `src-tauri/tauri.conf.json` 串行——F2 先合入 CSP 文本，F3 再做版本写回 + 全量冒烟（同一 JSON 文件两字段，并行必冲突；bump 写回若重排会覆盖 CSP）。§4 门禁串行。

## §1 F1 open_url 白名单

- [ ] 1.1 `platform.rs` 新增 `is_allowed_open_url`（强制 `url` crate，禁手写解析）+ 单测矩阵（放行/拒绝/IDN/控制符/尾点/userinfo/显式端口/子域冒充/scheme 混淆/畸形）
- [ ] 1.2 Windows 改 ShellExecuteW（锁死，不引 opener 插件；verb=open、返回值>32、不经 shell）+ macOS/Linux 白名单前置（`--` 隔离）；`cmd /c start` 彻底删除 + grep 门禁（`cmd.*/c.*start` + `Command::new("cmd")` 零命中）
- [ ] 1.3 `lib.rs open_url` 改 `Result<(), String>` fail-closed（错误码：`url_allowlist_rejected:<host>` vs `open_url_unsupported`）+ 拒绝日志仅 scheme+host+reason
- [ ] 1.4 前端：新增 `openExternalUrl()` 统一出口（禁直接 `safeInvoke("open_url")` + grep 门禁）；`openReleasePage` 白名单拒绝禁 `window.open` 回退（仅旧二进制缺命令可回退）；`openExa` 补 catch + `safeInvoke<void>` 类型收紧

## §2 F2 CSP + DOMPurify

- [ ] 2.1 `tauri.conf.json` CSP 收紧（prod enforce；dev 另行放宽本地 ws/HMR；浏览器预览走 `index.html` meta CSP；删 report-only，直接四态冒烟：tauri dev / build 产物 / vite preview / 浏览器预览）
- [ ] 2.2 消毒红队矩阵钉死 `tests/markdown-sanitize.redteam.spec.ts`（jsdom 前提；现行实现先跑，不通过项定去留；非 DOM 早返旁路契约先定）
- [ ] 2.3 DOMPurify 采用/保留书面结论（不允许无结论；若采用含包体积/构建数据）

## §3 F3 版本同步与发版约定

- [ ] 3.1 `bump-version.ps1` 纳入 `tauri.conf.json`（文本键级替换写回，禁 ConvertTo-Json 重排）+ dry-run 四处 diff 可见
- [ ] 3.2 先盘点已发布产物/tag，再一次性修复 `0.1.0 → 0.1.91`
- [ ] 3.3 新增 `check-version-sync.ps1`（只读不写，双态 exit code）+ CI MUST 接入（服务端不可被 `--no-verify` 绕过；pre-push 仅可选加速）
- [ ] 3.4 ADR 0012 发版约定节（取代规则：新增 ADR + 互链，禁改写正文；verify-decisions 同步）

## §4 门禁

- [ ] 4.1 Rust 白名单矩阵 + src-tauri lib 回归 + cmd 残留 grep 门禁
- [ ] 4.2 前端 update 回归 + 消毒矩阵 + vue-tsc + build + 四态冒烟（dev/build/preview/浏览器预览）+ 新增 fetch/open_url 直调 grep 门禁
- [ ] 4.3 bump dry-run + 版本一致校验双态
- [ ] 4.4 收口：canonical spec 同步 + change 归档 + 任务板更新
