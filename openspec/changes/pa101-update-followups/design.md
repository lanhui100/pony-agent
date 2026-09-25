# Design: pa101-update-followups

## F1 open_url 白名单（双审定稿：http 永拒；ShellExecuteW，不引 opener 插件；强制 url crate）

- 新增 `platform.rs::is_allowed_open_url(url: &str) -> bool`（纯函数，可单测），强制使用 `url` crate（`src-tauri/Cargo.lock` 已有传递依赖 `url 2.5.8`，转直接依赖；**禁止手写解析备选**）：
  - 预检：trim；含 `\r\n\t` 控制符 → false；非 ASCII host（IDN/punycode 同形）→ false；解析失败 → false。
  - scheme：仅 `https`（`http` 永拒——全仓无 http 需求；`file/data/javascript/blob/vbscript` 大小写混淆永拒）。
  - host：`url.host_str()` 小写归一后精确匹配 `{github.com, api.github.com, exa.ai}`；字符串级预检尾点（`github.com.` 先拒，不依赖 crate 归一）；子域冒充（`github.com.evil.test`/`evil-github.com`）天然不精确匹配；`userinfo@`、显式端口（含 `:443`）、凭据存在即拒（即使 host 命中）。
  - 路径/query/fragment 不限制（发布页 tag 经 encodeURIComponent，前端已保证）。
- Windows 执行：`ShellExecuteW(NULL, "open", url, NULL, NULL, SW_SHOWNORMAL)`，锁死参数形态（verb=open、返回值 >32 校验、URL 不经 shell）；**`cmd /c start` 彻底删除**（含 `Command::new("cmd")`），验收 grep 门禁。macOS `open` / Linux `xdg-open` 保留，统一先过白名单（参数加 `--` 隔离说明）。
- `lib.rs::open_url` 签名改为 `Result<(), String>`；错误码口径：`"url_allowlist_rejected:<host>"`（白名单拒绝，fail-closed）vs `"open_url_unsupported"`（旧二进制缺命令）。拒绝日志字段仅 `scheme+host+reason`，禁止 path/query/fragment，不记全 URL。
- 前端（两处都要改，design 初版"调用点不变"已证伪）：
  - `ConfigGeneralSection.vue:60-77 openReleasePage`：白名单拒绝（`url_allowlist_rejected:*`）**禁止 `window.open` 回退**（否则 fail-closed 变 fail-open），仅告警；只有旧二进制缺命令才回退 `window.open`。
  - `:131 openExa`：补 catch + 类型 `safeInvoke<void>`，与 65 行错误语义一致。
  - 新增统一出口 `openExternalUrl()`（禁直接 `safeInvoke("open_url")`），+ grep 门禁（新增直调即红）；新增调用点必须同步扩展 Rust 白名单。

## F2 CSP + DOMPurify（双审定稿：dev/prod 双策略；矩阵钉死 jsdom 落点；倾向 DOMPurify）

- CSP（`tauri.conf.json app.security.csp`，prod enforce）：`default-src 'self'; connect-src 'self' https://api.github.com; img-src 'self' data: https:`（任意 https 图床为已知放宽，登记）；`style-src 'self' 'unsafe-inline'`（tailwind 所需，永久豁免登记）；`script-src 'self'`（vite 产物若需 inline 则登记豁免，不默认加 unsafe）；`object-src 'none'; frame-src 'none'`。
- 架构不变量（安全审实证，前端 fetch 全仓仅 `update-check.ts:140`，模型出网走 Rust reqwest/IPC）：**模型请求永不走前端 fetch**。新增前端 fetch 必须同步评审 connect-src，+ grep 门禁（新增 `fetch(` 即红，除 update-check 外）。
- dev 策略（`tauri dev`，devUrl `http://127.0.0.1:4176` + HMR）：放宽本地 `ws://127.0.0.1 http://127.0.0.1` + vite client 所需项，仅本地生效并注释；prod 不得放宽。浏览器预览模式（非 Tauri）走 `index.html` meta CSP 另行覆盖。
- 删除"report-only 过渡"（Tauri v2 支持无证据）；改为：CSP 落地后四态冒烟（tauri dev / tauri build 产物 / vite preview 4175 / 浏览器预览）+ 更新检查可用。
- DOMPurify 评估矩阵钉死 `tests/markdown-sanitize.redteam.spec.ts`（jsdom 前提；`markdown.ts:100-102` 非 DOM 早返直通是结构性旁路，保留手写则必须先修旁路契约——抛错 vs 直通二选一）：
  - `javascript:`（含实体/大小写/空白混淆 `JaVaScRiPt:`/`&#106;`）、`onerror`/`onload`（img/svg）、`style` 属性与 expression、`form/button`、`iframe/embed/object`、`a ping`、`base` 劫持、`srcset/poster/background/data-*`、`svg/math` 畸形嵌套、`noscript` 嵌套、mXSS（`</noscript>` 解析差分类）、DOM clobbering（id/name）、`data:/blob:` 变体、嵌套畸形/大小写混淆标签；
  - 现行 `sanitizeMarkdownHtml` 先跑矩阵；倾向迁移 DOMPurify（正则已证伪为高维护债；若采用需评估包体积 ~60KB 级 + vite 构建增量 + @types + SSR 路径行为）；若保留手写，矩阵全部锁定为回归，不允许无结论。
  - **DOMPurify 评估结论（2026-09-25 实测采纳）**：
    - 实测手写正则在实体混淆（`&#106;`/`&#x6A;`）与斜杠分隔属性（`<a/href=...>`）用例中直接被击穿（3 failed / 26 passed）。
    - 迁移至 `dompurify`（3.4.16 + `@types/dompurify` 3.0.5）后，红队 29 项测试全绿（29 passed / 0 failed），原 markdown 渲染单测全绿。
    - 构建与体积实测数据：生产 bundle 从 898.99 kB 增至 926.27 kB（+27.28 kB），gzip 体积从 260.30 kB 增至 270.72 kB（+10.42 kB）；构建耗时 16.25s（基线 18.26s）；vue-tsc 0 错误。
    - 结论：正式采纳 DOMPurify 替换手写正则，彻底解除正则 XSS 结构性安全风险与长期维护债。

## F3 版本同步（双审定稿：与 F2 串行；键级替换写回；CI MUST 门禁；先盘点再修复）

- 串行约定：F2 先合入 CSP 文本并冒烟通过，F3 再做版本写回 + 全量冒烟（同一 `tauri.conf.json`，禁止并行合入）。
- `bump-version.ps1`：tauri 目标同步写 `tauri.conf.json $.version`，走**文本键级替换**（只改 `"version": "x.y.z"` 行，参考 `Update-CargoVersion` 正则思路；禁止 `ConvertTo-Json` 全文件重排——`$schema`/键序/缩进/BOM 必须保持）；dry-run 必须打印四处 diff（含 tauri.conf.json）。
- 一次性修复前加**已发布产物/tag 盘点**：确认无对外分发的 0.1.0 bundle 版（或记录其 OS 升级判定影响），再执行 `tauri.conf.json 0.1.0 → 0.1.91`（与当前 package.json/src-tauri Cargo/`.version.json` tauri 对齐）。
- 新增校验 `scripts/check-version-sync.ps1`（四处：package.json、src-tauri/Cargo.toml、tauri.conf.json、`.version.json` tauri；core 独立校验一致性；只读不写；一致/篡改双态 exit code），**CI verify 追加为 MUST**（服务端不可被 `--no-verify` 绕过；pre-push 仅作可选加速）。
- ADR 0012 修订走取代规则（`docs/decisions/README.md`：新增 ADR + 旧篇互链，禁改写正文；`verify-decisions` 门禁同步）：发版约定 release tag == 打 tag 时点 package.json 版本（可带 `v` 前缀），与 `parseVersionTag` 口径一致；tag 漂移 → 更新检测 malformed 漏报（已知 fail-closed 行为，回归锁定）。

## 测试矩阵

- Rust `platform::tests`：白名单矩阵（放行/拒绝/混淆/畸形）+ ShellExecuteW 参数构造（Windows 下 dry-run 断言或 cfg 隔离）。
- 前端：update 三 spec 回归（update-check/update-store/ConfigGeneralSectionUpdate）+ 消毒红队矩阵 + vue-tsc + vite build。
- 工程：bump dry-run 四处一致；check-version-sync 在一致/篡改两种状态下的 exit code。
