# Proposal: add-app-update-check（PA-099）

> 修订 v2（2026-08-24）：吸收双 reviewer 对抗审核意见——构造式跳转 URL、缓存反规范化消除、超时与并发幂等、错误分类、角标范围收敛为设置入口、隐私开关。采纳记录见同目录 `reviews.md`。

## Why

Pony Agent 当前没有任何软件更新感知能力：用户无法在应用内得知 GitHub 有新发版，只能自行去仓库查看。本变更为配置页新增"软件更新"区块，并在侧栏"设置"入口图标上提供角标提醒——当 GitHub 仓库 `lanhui100/pony-agent` 出现比当前版本更新的 release 时，用户打开应用即可看到提醒并一键前往发布页。

执行跟踪卡：`management/task-system/03_TASKS/PA-099-app-update-check.md`。

## What Changes

1. **更新检测纯逻辑层** `src/lib/update-check.ts` + 类型 `src/types/update.ts`：
   - 常量：仓库 `lanhui100/pony-agent`、Releases API URL（`https://api.github.com/repos/lanhui100/pony-agent/releases/latest`）、自动检查最小间隔（24h）、请求超时（8s）、缓存 key `pony-agent.update-check.v1`、偏好 key `pony-agent.update-prefs.v1`；
   - `parseVersionTag`：严格正则 `^[vV]?\d+\.\d+\.\d+$` 解析 semver 三元组，其余（预发布后缀、空白、畸形）一律 null；
   - `isNewerVersion(candidate, current)`：双方均可解析才做三元组数值比较，任一不可解析返回 false（宁漏报不误报）；
   - `buildReleasePageUrl(tagName)`：**构造式跳转 URL**——仅当 tagName 通过格式验证时返回 `https://github.com/lanhui100/pony-agent/releases/tag/<encoded>`，否则 null。API 返回的 `html_url` 全程不进入存储、状态与跳转链（消解缓存投毒钓鱼与前缀混淆面）；
   - `fetchLatestRelease()`：GET 上述 URL，`headers: { Accept: "application/vnd.github+json" }`、`credentials: "omit"`、`cache: "no-store"`，AbortController 8s 超时**同时覆盖 fetch 与 body 读取**；结果映射 `{tagName,name,publishedAtMs}`（`published_at` 缺失或非有限数值→null）；错误分类：404→`unpublished`、403→`rate-limited`、其他非 2xx→`http-error`、JSON 解析失败/非对象 payload/tag_name 缺失或不可解析→`malformed`、abort（含 body 阶段）→`timeout`、body 中断 TypeError 及其余→`network`；
   - 缓存读写：文件结构 `{ checkedAtMs, release: {tagName,name,publishedAtMs} | null }`（release=null 即 404 负缓存）；读取全程防御性校验（非法 JSON、字段类型错、tagName 不可解析→整体丢弃），`checkedAtMs` 由 store 在水合处对未来时间戳钳制为当前时间（同时约束节流判定与"上次检查"展示）；写入 try/catch 静默降级。
2. **Pinia store** `src/stores/update.ts`（`useUpdateStore`，options API 与 settings.ts 同风格）：
   - 状态机：`idle | checking | up-to-date | available | unpublished | error`；`currentVersion` 来自 package.json JSON import（与 bump-version 同步链一致）；
   - getter `hasUpdate`：由 `latest.tagName` 与 `currentVersion` **现算**（持久化数据只存原始快照，派生值永不落盘——同时消除"应用升级后旧角标残留最长 24h"的正确性 bug）；
   - getter `releasePageUrl`：经 `buildReleasePageUrl` 构造；
   - `initialize()`：读偏好→读缓存并按现算结果恢复 status/latest（不发网络）；随后仅当 `autoCheck` 开启且缓存缺失或超 24h 时，后台静默执行一次检查；
   - `checkForUpdates(manual)`：**in-flight 幂等守卫**（checking 中重复调用直接返回）；手动调用恒绕过 24h 节流，成功后重写 `checkedAtMs`；
   - 失败分路径：手动检查失败→`error` 态 + 人话文案（限流/网络异常/响应异常/超时四类，细节进 console）；后台静默检查失败→仅 `console.warn`，保留既有状态与角标；
   - 显式约束：**无自动重试、无 setInterval 定时器**（防止后人好心加出重试风暴）；
   - `setAutoCheck(enabled)`：持久化偏好，关闭后仅停止自动检查，手动检查不受影响。
3. **配置页 UI**（`ConfigGeneralSection.vue` 追加"软件更新"卡片，位于服务密钥之后）：
   - 显示当前版本（`v{packageJson.version}`）；"检查更新"按钮（checking 中禁用+旋转图标）；
   - 状态区五态：available（新版本号 + 发布名 + 发布日期 + "查看发布页"按钮）/ up-to-date / unpublished（仓库暂无发布）/ idle（尚未检查）/ error（分类文案，可重试）；
   - "查看发布页"跳转：Tauri 下走既有 `open_url` 命令（URL 为代码内构造常量值），reject 时回退；浏览器模式 `window.open(url, "_blank", "noopener,noreferrer")` 并判空（弹窗被拦截时 console.warn）——不沿用 openExa 的裸调用模式；
   - "自动检查更新"Switch（复用 ui/Switch.vue），持久化到偏好 key；卡片注明启动时会匿名访问 api.github.com（隐私披露）；
   - release name/tag 一律 Vue 插值渲染，**禁止 v-html**。
4. **侧栏角标**（`HomeSessionSidebar.vue`）：仅在"设置"两个入口（折叠态齿轮按钮 + 展开态"设置"行）上，`hasUpdate === true` 时渲染右上角 amber-500 圆点（static，带 title"发现新版本"）。颜色刻意避开 rose（既有语义=会话删除确认/任务失败）且与更新卡片所在 general tab 入口一致；点击行为不变。
5. **启动接入**（`App.vue`）：onMounted 启动任务数组追加 `runStartupTask("updateCheck", () => updateStore.initialize())`；`tests/App.spec.ts` 同步更新（beforeEach spyOn updateStore.initialize + 启动计数 6→7）。

## Scope

- 改动文件：新增 `src/types/update.ts`、`src/lib/update-check.ts`、`src/stores/update.ts` 及 3 个新测试文件；修改 `ConfigGeneralSection.vue`、`HomeSessionSidebar.vue`、`App.vue`、`tests/HomeSessionSidebar.spec.ts`、`tests/App.spec.ts`；
- 数据源固定 GitHub REST v3 `releases/latest`（语义排除 draft/prerelease），无鉴权无请求体；
- 检测本身**有意不做 isTauriAvailable 门控**（浏览器预览模式同样可用）——这是设计决定，实现者不得自行添加门控；仅跳转层区分 Tauri/浏览器路径；
- 测试环境注意：jsdom 无 `__TAURI__`，组件测试天然走浏览器分支；涉及真实 fetch 的入口在测试中一律 stubGlobal 或 spyOn 拦截，禁止测试触网。

## Non-Goals

- 不做自动下载、安装或差量升级（只检测 + 跳转发布页）；
- 不引入任何 Rust/后端改动或新依赖（不为 src-tauri 加 reqwest/http 插件）；未来若接 Tauri 官方 updater（签名/manifest）属独立 ADR，与本变更无关；
- 本批不修 `tauri.conf.json`(0.1.0) 与 package.json/Cargo.toml(0.1.84) 的失同步，但作为**前置跟进任务**登记（见任务卡 Follow-ups）；运行时比较基线取 package.json 版本，配套发布约定："release tag 必须等于打 tag 时点的 package.json 版本（可带 v 前缀）"，该约定写入 ADR；
- 不做代理/镜像源配置；不做除"自动检查开关"以外的频率配置。

## Risks

- **GitHub 匿名限流**（60 次/h/IP）：自动检查 24h 一次 + 手动兜底；403 映射为明确的限流文案；共享出口 IP 企业环境下可能常态失效——error 文案承认检测失效，不静默吞掉。
- **WebView2 TLS 拦截**：企业代理可信根可替换响应体（内容级污染而非连接失败）——这正是响应校验与构造式 URL 按"不可信输入"设计的原因；残余风险：伪造响应可令用户看到假版本号文本（跳转仍落在真实 github.com 域）。
- **时钟依赖**：仅影响 24h 节流判定（未来时间戳已钳制）；偏移最多多查或晚查一次。
- **tag 格式漂移**：非 semver tag → malformed/不提示（漏报优于误报）；tag↔package.json 一致性属运维约定（见 Non-Goals），失真风险已在 ADR 登记。
- **隐私**：每次自动检查向 api.github.com 发匿名 GET（暴露出口 IP、UA、时刻分布）——卡片内 Switch 提供退出通道 + 本文档披露；请求不带任何凭据。

## Rollback

全部为前端增量文件 + 三处小改；回滚即删新文件并 revert 三处修改。localStorage key 独立命名（`pony-agent.update-check.v1`/`pony-agent.update-prefs.v1`，符合仓库 `<domain>.v<N>` 惯例），残留无害，无数据迁移。

## 验收标准

1. 单元测试覆盖：
   - 版本解析/比较边界（v/V 前缀、相等、更老、预发布后缀拒绝、任一侧不可解析恒 false）；
   - fetch 映射（成功含 name/published_at 缺省、404、403 限流、500、HTML 错误页、tag_name 缺失/畸形、abort 超时）与请求选项断言（credentials/cache/Accept/signal）；
   - 构造式 URL（合法 tagName 编码、非法→null）；
   - 缓存往返 + 投毒矩阵（非法 JSON、类型错、坏 tagName 整体丢弃、未来 checkedAtMs 钳制）+ 偏好往返；
   - store 状态机：新鲜缓存恢复零网络、过期触发（含恰好 24h 边界）、available↔up-to-date 迁移（含"升级应用后旧角标消失"回归）、404→unpublished 负缓存、手动错误保角标、后台静默失败不改状态、in-flight 幂等（init 与手动并发单次请求）、autoCheck 关闭不拉网；
   - 侧栏角标随 hasUpdate 显隐（折叠 + 展开"设置"入口各一）；
   - 配置页更新卡片五态渲染、检查按钮流转、"查看发布页"调用参数（noopener）、隐私开关持久化；
   - `App.spec.ts` 启动任务计数更新（7 个）且全文件不触网。
2. `npm run test:unit` 全绿；`npm run typecheck` 通过。
3. 手动验收路径：篡改 localStorage 缓存 release.tagName 为更高版本 → 重启应用 → "设置"入口出现角标；配置页显示新版本信息，"查看发布页"落到真实 GitHub release 页；将缓存 htmlUrl 类字段注入任意值不影响任何行为（该字段根本不被消费）。
