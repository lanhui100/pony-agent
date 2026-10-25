# wave-44 机器收据（NFR + 可观测性 + 冒烟）

采集时间：2026-10-25 17:18–17:47 CST｜基线 `ab54b06`｜红相锚定 `ae5f30f`

## 1. NFR 机械化比对（`.dev-team/nfr-baseline-wave44.json`）

| 基准字段 | 代码/测试证据 | 命令 | 结论 |
| :--- | :--- | :--- | :--- |
| `hydration_read_timeout_ms=15000` | `constants.ts:15` 常量 15000；`runtime.ts:2696` `withTimeout(hostRead, HYDRATION_TIMEOUT_MS, "加载对话")`；PA115-T0 断言常量、T2 下界 14000 未收敛 / 16000 收敛 | `npx vitest run tests/acceptance/wave44/pa115-host-read-timeout.spec.ts` | PASS（EXIT 0） |
| `draft_preserved_after_timeout=true` | `runtime.ts:2723-2728` 回滚前保存 draft、回滚后恢复；PA115-T2a 断言草稿保留 + sessionError 非空 | 同上 | PASS |
| `settle_must_be_fulfilled_not_rejected=true` | PA115-T2 `elapsed.reason === "resolved"` | 同上 | PASS |
| `submit_turns_during_composition=0` / `undo_invocations_during_composition=1` | `HomeWorkspace.vue:1413-1428` 守卫收敛进 Enter 分支；PA135-T1/T2 | `npx vitest run tests/acceptance/wave44/pa135-composition-undo.spec.ts` | PASS（EXIT 0） |
| `console_info_calls_when_disabled=0` | `HomeWorkspace.vue:196-199` 前置短路；PA117-T1 | `npx vitest run tests/acceptance/wave44/pa117-scroll-debug-log.spec.ts` | PASS（EXIT 0） |
| `layout_reads_when_disabled=0` | `useTimelineAutoScroll.ts:187-190` 在 `collectMetrics()` 之前短路；PA117-T1 关闭态 rect=0 / 启用态>0 | 同上 | PASS |
| `runtime_reevaluation_no_cache=true` | `utils.ts:45-67` 每次调用读取 localStorage/URL/DEV | PA117-T3 | PASS |
| `resource_release` | `withTimeout` settle 后 `clearTimeout`（`runtime/utils.ts:3-18`）；`initializeSessions` finally `clearTimeout(watchdog)`（`runtime.ts:2733-2736`） | 代码核对 | PASS（无新增泄漏路径） |

## 2. 全量门禁

| 命令 | Exit | 结果 |
| :--- | :--- | :--- |
| `npm run test:unit` | 0 | 102 files / 1034 passed / 10 skipped |
| `npm run build`（vue-tsc --noEmit + vite build） | 0 | built in 7.33s |
| `npm run version:check` | 0 | 四处版本同步 |
| `node scripts/check-secrets.mjs --all` | 0 | 0 leaks |
| `npx vitest run tests/acceptance/wave44` | 0 | 8/8 passed（红相锚定前同命令 8/8 failed） |

## 3. 真实链路冒烟（浏览器）

| 场景 | 命令/证据 | 结果 |
| :--- | :--- | :--- |
| dev 态（DEV=true） | `agent-browser open http://127.0.0.1:4176/` + `console` | Console Error=0；`[workspace-scroll]` 按契约默认开（DEV 兜底） |
| 生产构建态（dist preview :4177） | `vite preview` + `agent-browser open` + `console` | Console Error=0；`[workspace-scroll]` 计数 **0**（PA-117 生产默认关生效） |
| 截图 | `.dev-team/evidence/wave44/{dev-browser-home,prod-preview-home}.png` | 已落盘 |
| console 原文 | `.dev-team/evidence/wave44/prod-preview-console.log` | 已落盘 |

## 4. 可观测性门禁

| 命令 | Exit | 说明 |
| :--- | :--- | :--- |
| `check-declaration.sh /home/dm/pony-agent` | 0 | 四维声明与实测一致（metrics/logs 结构性 N-A 有理由） |
| `audit-diff.sh --base ab54b06 --decl-approved` | 1（WARN） | 唯一 WARN：声明未声明收口点 → 规则集为空（已知结构性，见 `docs/observability.yaml` note 与审计报告 §6）；无违规 |
| `audit-project.sh /home/dm/pony-agent --json` | 2 | PARTIAL 口径来自 desktop 探测缺陷（审计报告 P3-9），非本波回归 |

## 5. 最终树复验（17:56–17:59，含 L3-B BLOCKING 修复后）

| 命令 | Exit | 结果 |
| :--- | :--- | :--- |
| `git stash push -- src/` → `npx vitest run tests/acceptance/wave44`（红相重锚，测试冻结后） | 1 | 8/8 failed（含 PA117-T1 Assert5 落盘键断言） |
| `git stash pop` → 同命令 | 0 | 8/8 passed |
| `npx vue-tsc --noEmit` | 0 | 无 diagnostics |
| `npm run test:unit` | 0 | 102 files / 1034 passed / 10 skipped |
| `npm run build` | 0 | built in 17.39s |

> 说明：`PA117-T1` 新增第 5 条断言（关闭态落盘 `streamDebugState` 无 `markdownRender*` 键）在“stash 实现”状态下 8/8 红已复验，满足 L3-P 对证据时效的要求。
