# Tasks: add-app-update-check

> 执行跟踪卡：`management/task-system/03_TASKS/PA-099-app-update-check.md`
> 修订 v2：吸收双 reviewer 审核意见（见 reviews.md）。
> 并行边界：§1 → §2（store 契约冻结点：status 枚举 / hasUpdate / releasePageUrl getter / initialize / checkForUpdates(manual) / setAutoCheck 签名）→ §3/§4 可并行 → §5 门禁串行。

## §1 纯逻辑层

- [x] 1.1 新增 `src/types/update.ts`：`UpdateStatus`、`AppReleaseInfo`、`UpdateCacheFile`、`UpdatePrefs` 类型
- [x] 1.2 新增 `src/lib/update-check.ts`：常量（仓库/API URL/24h/8s 超时/两个 storage key）、`parseVersionTag`（严格正则）、`isNewerVersion`、`buildReleasePageUrl`（构造式跳转 URL，htmlUrl 不入链）、`fetchLatestRelease`（credentials:"omit"、cache:"no-store"、Accept 头、AbortController 8s、错误六分类）
- [x] 1.3 缓存读写：`{checkedAtMs, release|null}` 结构、防御性校验（坏 JSON/类型错/坏 tagName 整体丢弃、未来 checkedAtMs 钳制）、setItem try/catch；偏好读写 `{autoCheck}`

## §2 状态层（契约冻结点）

- [x] 2.1 新增 `src/stores/update.ts`：状态机六态 + `hasUpdate`/`releasePageUrl` getter 现算 + `initialize()`（缓存恢复零网络、autoCheck 且过期才后台查）+ `checkForUpdates()`（in-flight 幂等守卫、手动绕过节流、手动失败置 error/后台失败 console.warn 保态、无自动重试无定时器）+ `setAutoCheck()`
- [x] 2.2 `App.vue` 启动任务数组追加 `updateCheck`；`tests/App.spec.ts` 同步：beforeEach spyOn initialize + 计数 6→7 + 全文件禁触网

## §3 配置页 UI

- [x] 3.1 `ConfigGeneralSection.vue` 追加"软件更新"卡片：当前版本 chip、"检查更新"按钮（checking 禁用+旋转）、五态状态区、查看发布页（open_url reject 回退 / window.open noopener,noreferrer+判空）、"自动检查更新" Switch（隐私披露文案）、禁止 v-html

## §4 侧栏角标

- [x] 4.1 `HomeSessionSidebar.vue`："设置"两处入口（折叠齿轮按钮 + 展开设置行）加 relative + hasUpdate 条件 amber-500 静态圆点 + title；不动 models 入口（避免 rose 语义冲突与噪音）

## §5 测试与门禁

- [x] 5.1 `tests/update-check.spec.ts`：解析/比较矩阵、构造式 URL、fetch 六分类映射 + 请求选项断言、超时 abort、缓存投毒矩阵、偏好往返（34 用例全绿）
- [x] 5.2 `tests/update-store.spec.ts`：新鲜缓存零网络、过期触发含 24h 边界、available↔up-to-date 迁移（升级后旧角标消失回归）、404 负缓存、手动错误保角标、后台静默失败、并发幂等单请求、autoCheck 关闭不拉网（13 用例全绿）
- [x] 5.3 扩展 `tests/HomeSessionSidebar.spec.ts`：设置两处入口角标显隐（折叠 + 展开，新增 describe 4 用例）
- [x] 5.4 新增 `tests/ConfigGeneralSectionUpdate.spec.ts`：卡片五态渲染、检查流转、跳转参数断言、开关持久化（9 用例全绿，含 Tauri reject 回退）
- [x] 5.5 门禁：全量 vitest 30 文件 / **504 passed** / 10 skipped / 0 failed（代码双审修复后净增 11 用例）；`npm run typecheck` 通过。注：ui-guard 分支覆盖率 79.04% 为存量失败（HEAD 基线 78.95% 同样不达 80%，拖累源为本次未改动的 HomeWorkspace.vue），本改动略有改善；沙箱环境运行 vitest 需 `--configLoader native --pool threads`（vite 配置打包与 forks 池需 spawn 子进程）
- [x] 5.6 收口文档：ADR 0012（implemented）✅ 已落盘并过机械校验；任务卡勾选 ✅；reviews.md 已回填 spec 双审 + 代码双审采纳表；变更目录待用户手动验收后归档

## 代码双审修复记录（2026-08-24）

双审"有条件通过"，必修项与全部建议项已修复并复验：P2 json() 超时作用域缺口（fetch+body 同一死线、body 阶段 abort→timeout/TypeError→network）、null payload 守卫、未来 checkedAtMs 水合钳制、后台失败恢复文案、name trim 口径统一、open_url reject 回退告警、重入注释。详见 reviews.md 代码双审采纳表。

## Follow-ups（独立后续任务，不阻塞本特性）

- [ ] F1 Rust `open_url` 加固：URL 解析 + scheme/host 白名单，替换 `cmd /c start`（tauri-plugin-opener 或 ShellExecuteW），顺带修 start 标题参数怪癖
- [ ] F2 CSP 从 null 收紧方案（connect-src 至少限定 api.github.com）；`sanitizeMarkdownHtml` 正则消毒 → DOMPurify 评估
- [ ] F3 发布工程 owner 确认 release/tag 流水线约定（tag == package.json 版本）；修复 `tauri.conf.json` version 失同步并入 bump-version.ps1 同步链

## 审核安排

- Spec 对抗审核：reviewer-A（安全边界/失败路径）+ reviewer-B（架构一致性），并行独立 —— 已完成，均"有条件通过"，修订已并入 proposal v2；
- 代码对抗审核：reviewer-C（正确性/回归）+ reviewer-D（边界/失败路径/安全落实），并行独立 —— 已完成，均"有条件通过"（无 P0/P1），必修项与建议项已全部修复并复验，采纳表见 reviews.md。
