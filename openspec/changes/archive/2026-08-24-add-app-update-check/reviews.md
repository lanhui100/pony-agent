# Reviews: add-app-update-check

> 2026-08-24。两路独立对抗审核（子智能体、隔离上下文、并行），对象为 proposal v1；结论均为**有条件通过**。以下为编排器去重后的逐条裁决。

## Reviewer A（安全边界与失败路径）

结论：有条件通过。P1×3、P2×6、P3×若干；建议升级 security consultant（范围超本功能边界）。

## Reviewer B（架构边界与项目一致性）

结论：有条件通过。P1×3、P2×7；无需 LLM consultant，唯一升级点是流程层（发布流水线确认）。

## 采纳记录（去重合并）

| # | 来源 | 问题 | 裁决 | 说明 |
|---|---|---|---|---|
| 1 | A#2③/A#4 | startsWith 前缀守卫可被 URL 规范化绕过；存储 htmlUrl 有投毒钓鱼面 | **采纳** | 弃用 htmlUrl 全链路；改构造式 `buildReleasePageUrl(tagName)`，tagName 过严格正则后才编码拼接 |
| 2 | A#2-A/B-P2.5 | 缓存持久化派生 hasUpdate：升级后角标残留 24h（确定性 bug）+ 投毒直驱 UI | **采纳** | 缓存只存原始快照，hydrate 用当前版本现算 |
| 3 | A#3/B-P1.2 | fetch 无超时 → checking 永久死锁 | **采纳** | AbortController + 8s；A 建议 8s/B 建议 10s，取 8s |
| 4 | A#5/B-P2.6 | init 与手动检查并发竞态、乱序覆盖 | **采纳** | store 级 in-flight 幂等守卫 + 测试 |
| 5 | B-P1.1 | App.spec.ts 启动数组变更后会真实触网 | **采纳** | beforeEach spyOn initialize；计数断言 6→7 |
| 6 | A#6/B-P2.9 | 错误分类学缺失（限流/404/非 JSON 混同） | **采纳** | 六分类错误码 + 人话文案四类；403 显式限流语义 |
| 7 | A#8 | 404 无负缓存 → 全体用户每次启动打 API | **采纳** | `{checkedAtMs, release:null}` 负缓存结构 |
| 8 | A#9 | window.open 沿用 openExa 裸调用模式 | **采纳** | noopener,noreferrer + 判空 + open_url reject 回退 |
| 9 | A Must#5/B-P2.4 | 手动是否绕过节流未写明；缺显式约束 | **采纳** | 手动恒绕过、成功重写 checkedAtMs、无自动重试无定时器写入 spec |
| 10 | A Should#7 | 隐私披露与退出通道缺失 | **部分采纳** | 卡片内"自动检查更新"Switch（localStorage 偏好）+ proposal/ADR 披露；不做全局设置项（避免扩 scope） |
| 11 | A Should#8/B 边界 | setItem 抛错、未来 checkedAtMs、published_at NaN | **采纳** | 全部防御性处理 |
| 12 | B-P2.8 | 角标 4 处 rose 与既有失败语义冲突、噪音大 | **采纳** | 仅"设置"两处入口、amber-500 静态圆点（贴合用户原意"配置的图标"） |
| 13 | B-P2.7 | 后台失败置 error 会打扰且掩盖真实态 | **采纳** | 后台静默 console.warn 保态；error 仅手动路径 |
| 14 | B-P2.10 | 测试文件名与实现方式矛盾 | **采纳** | 改名 `ConfigGeneralSectionUpdate.spec.ts`（区块内联于 ConfigGeneralSection） |
| 15 | B 关键假设②④ | "package.json 读版本即解决失同步"不成立；isTauriAvailable 门控缺失应明示 | **部分采纳** | 基线仍取 package.json（自洽比较域）；发布约定 tag==package.json 版本写入 ADR；"有意不加门控"写入 spec Scope |
| 16 | B-P1.3 | 版本基线语义错位：conf 0.1.0 失同步、tag 止于 v0.1.2、可能出厂即 unpublished | **部分采纳（登记前置跟进）** | 不在本批修 conf（避免 build 配置扩 scope）；F3 登记流水线确认 + conf 同步任务；功能对 unpublished/error 态优雅降级 |
| 17 | A Follow-up | Rust cmd/c start 注入面、CSP null、正则消毒器 | **登记后续任务** | F1/F2；系仓库级既有风险面而非本功能引入；本功能经构造式 URL 已将自身残余面压至"伪造响应文本"级 |
| 18 | A 升级 consultant / B 否决 | 分歧 | **裁决：不升级** | 两审在功能边界内无冲突；A 所请事项全部超出本特性文件边界且已有缓解与登记，按协议立独立任务足够 |
| 19 | B 并行边界 | §3/§4 须先冻结 §2 契约 | **采纳** | tasks.md 已标注契约冻结点 |
| 20 | B 杂项 | 缓存 schema 内嵌版本字段 | **不采纳** | key 级 `.v1` 后缀已是仓库惯例（runtime-history.v1 同款），演进时换 key 即可 |

---

# 代码双审（实现 diff）

> 2026-08-24。两路独立子智能体审查实现：reviewer-C（正确性/回归/一致性）+ reviewer-D（边界/失败路径/安全落实）。结论均**有条件通过**，无 P0/P1。reviewer-C 实测 7 个相关 spec 文件全绿 + vue-tsc 干净；reviewer-D 全量 grep 确认隐私约束兑现。

## 兑现确认（两审一致）

构造式 URL 全链路成立（html_url 零存储零消费、url 来源唯一、scheme/host 编译期常量）；`credentials:"omit"`/`cache:"no-store"`/无 setInterval/无自动重试经 grep 验证；零 v-html、title 均静态绑定；状态机全在 Pinia store、组件卸载无残留；手动失败保角标、后台静默保态、404 负缓存均有测试覆盖；App.spec spyOn 拦截被证实有效（pinia 激活实例时序推理 + phased-order 计数旁证）。

## 采纳记录

| # | 来源 | 问题 | 裁决 | 落点 |
|---|---|---|---|---|
| C-P2/D-P2-1 | 双审共指必修 | `json()` 在超时作用域外：body 停滞 → checking 永久卡死；body 阶段 abort 误分类 malformed | **已修** | `fetchLatestRelease` 重构：fetch 与 json() 纳入同一 try/finally；body 阶段 aborted→timeout、TypeError→network |
| D-P3-1 | 边界审 | JSON null/原始值 payload 无守卫 | **已修** | 非对象 payload → malformed |
| A-P3-3/D-P3-2 | 双审 | 未来 checkedAtMs 展示位漏钳制 | **已修** | store 水合处 `Math.min(now)` 一处钳制（节流+展示同修）+ proposal 表述同步 |
| C-P3-1 | 正确性审 | 后台失败恢复 error 态丢失文案 | **已修** | priorErrorMessage 随 priorStatus 一并恢复 |
| C-P3-4 | 正确性审 | 缓存 sanitize name 判空不 trim（与 fetch 口径不一） | **已修** | 统一 trim 口径 |
| D-P3-3 | 边界审 | Tauri open_url reject 回退静默 | **已修** | 回退前 console.warn + 测试覆盖 |
| C-P3-5 | 正确性审 | initialize 重入安全隐式 | **已修** | 注释说明 + 重复 initialize ≤1 请求回归测试 |
| 补测试 | 双审清单 | body 阶段超时分类 / null payload / 未来时间戳投毒+shouldAutoCheck 单测 / 后台恢复 up-to-date 分支 / 投毒丢弃端到端补查 / Tauri reject 回退 | **全部补齐** | update-check +4、update-store +5、ConfigGeneralSectionUpdate +1（Tauri mock 分派） |
| 守卫注释 | C 建议 | App.spec spy 依赖 mount 不装 pinia 的隐式前提 | **已加** | beforeEach 注释写明失效条件 |

## 复验证据

修复后全量 vitest：30 文件 / **504 passed** / 10 skipped / 0 failed（净增 11 用例）；`npm run typecheck` 通过。

## 残余风险（知悉，登记于任务卡）

- 隐私 opt-out 在 localStorage 持久化故障时静默回默认开启（下次启动仍发一次匿名 GET）
- 企业共享出口 IP 常态 403 限流 → 显式限流文案（spec 已承认的失效模式）
- TLS 拦截最坏后果为假版本号文本，跳转恒落真实 github.com 域
- 手动检查遇 404 将 available 角标覆写为 unpublished 负缓存（"404=确定性结论"设计决定）

## 未尽事项

- F1/F2/F3 三项后续任务已在 tasks.md 登记，随 PA-099 任务卡归档追踪。
- openspec 变更目录暂留 active（未归档）：待用户完成手动验收路径（proposal 验收标准 3）后归档至 `archive/2026-08-24-add-app-update-check/`。
