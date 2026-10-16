# Wave-17 NFR 达标报告：IME 组合态回车守卫 + 动态数字抖动消除

- 日期：2026-10-16
- 分级：B-lite
- 基线 commit：`5d7d691`
- 红相锚定：`f060c8d`（tag `wave-17-red-anchor`）
- 绿相检查点：`49fd118`（tag `wave-17`）
- 契约：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md`
- NFR 基准：`.dev-team/nfr-baseline-wave17.json`（本波旁路 5 键，不改动 PA-118 冻结的 16 键 `.dev-team/nfr-baseline.json`）

---

## 1. NFR 逐项达标对照

| 基准字段 | 基准值 | 本波适用性 | 实现证据 | 结论 |
| :--- | :--- | :--- | :--- | :--- |
| `external_call_timeout_ms` | 3000 | `applicable: false` | union diff 70 行新增行 grep `fetch\|axios\|XMLHttpRequest\|.invoke(\|tauri\|WebSocket\|EventSource\|sendBeacon\|http(s)://\|AbortController\|signal\|timeout` → **0 命中**。新增面仅 `src/lib/ime.ts` 纯谓词 + Vue 模板 class/事件绑定。 | **PASS** |
| `retry` | `applicable: false` | `applicable: false` | 同上：无新增外部调用 → 无重试对象。未编造 `max_attempts` / `backoff_cap_ms`。 | **PASS** |
| `concurrency_lock` | `applicable: false` | `applicable: false` | 每个元素仅 1 条 `@keydown.enter` 绑定（`WorkspaceComposer.vue:885/934/955/1385`、`HomeSessionSidebar.vue:835`、`HomeSessionSidebarRow.vue:101`），全库 `@keyup.enter` 0 命中 → 单次回车 1 次提交。既有幂等闸门未动（`WorkspaceComposer.vue:778` `askIsBusy`、`HomeSessionSidebar.vue:132` `renamingBusy`）。`<button>` 上 `.prevent` 保留 → Enter 合成 click 仍被抑制（真实 Chromium 探针：`{enter:1, click:0}`）。 | **PASS** |
| `logging` | `format=json`, `trace_id=true` | `applicable: false` | union diff 全文 grep `catch` → **0 命中**（无裸 catch、无吞异常）；新增行 grep `console.\|logger\|.log(\|.warn(\|.error(` → **0 命中**。本波未新增任何日志面。 | **PASS** |
| `resource_release` | `applicable: false` | `applicable: false` | 新增行 grep `addEventListener\|setInterval\|setTimeout\|requestAnimationFrame\|new Worker\|.subscribe\|onMounted\|onBeforeUnmount\|watch(` → **0 命中**。`src/lib/ime.ts` 全文 29 行无全局监听、无定时器、无句柄。新增 3 个包装函数均为「谓词早退 + 委托原函数」。 | **PASS** |

**NFR-DEGRADED 欠账：0**（无豁免用例、无降级登记）。

---

## 2. L3-P 生产就绪五问否决制

L3-P 审查员对合并后 union diff 执行五问基准映射，逐项输出「基准字段 vs 代码行证据 vs PASS/FAIL」：

| # | 问 | 基准字段 | 机械可判定 | 结果 |
| :--- | :--- | :--- | :--- | :--- |
| 1 | 外部调用超时 | `external_call_timeout_ms` | ✅ | PASS |
| 2 | 日志规范与防吞 | `logging.format/trace_id` | ✅ | PASS |
| 3 | 资源泄漏防御 | `resource_release` | ✅ | PASS |
| 4 | 并发与幂等 | `concurrency_lock` | ✅（1 项 non-machine：浏览器 Enter→click 合成语义，已由真实 Chromium 门禁侧证） | PASS |
| 5 | 配置与自检 | 基准表无此键 | ✅ | PASS |

**verdict: PASS，findings = 0**。唯一备案项：`nfr-baseline-wave17.json` 未含 `config/self-check` 键（属基准表缺项，非实现违规）。

---

## 3. 客观事实清单（物理凭据）

| 门禁 | 命令 | EXIT | 收据 |
| :--- | :--- | :--- | :--- |
| 类型检查 | `npx vue-tsc --noEmit` | 0 | `.dev-team/evidence/wave-17/green-barrier-final.txt` |
| 冻结验收 C1+C2+C4 | `npx vitest run <3 spec>` | 0（31 passed） | 同上 |
| 全量单测回归 | `npx vitest run` | 0（897 passed / 10 skipped，基线 866） | 同上 |
| 真实浏览器门禁 | `npm run build && npx playwright test tests/e2e/wave17-browser-gate.spec.ts` | 0（4 passed，Console Error=0，pageerror=0） | `.dev-team/evidence/wave-17/green-browser-gate.txt` + Lead 独立复跑 |
| 版本四处同步 | `npm run version:check` | 0 | `.dev-team/evidence/wave-17/green-barrier-final.txt` |
| 本波 NFR 校验 | `node .dev-team/tools/validate-nfr-baseline.mjs` | 0 | 同上 |
| 既有 16 键冻结基准回归 | `python3 .dev-team/nfr-lint.py` | 0（零字节改动） | 同上 |
| Flake 复跑 | 冻结验收 × 3 | 3/3 EXIT=0 | `.dev-team/evidence/wave-17/green-flake-check.txt` |
| 红相隔离复核 | worktree @ `wave-17-red-anchor` + REV-2 后 spec | 1（12 failed / 2 passed，证明红相未被污染） | 本报告 §1 与 `.dev-team/wave17-green-receipts.md` |

关键旅程截图：`.dev-team/evidence/wave-17/wave17-browser-gate-home.png`
（sha256 `64d9de9638de88d7cb34476eb3695a9eedf77716dcd23d82827ab7fbf2c73786`，1280×720，85,132 B；画面含左侧栏 / 中间空态 / 底部 composer 内文本 / 右侧状态面板，启动闪屏已消失）

---

## 4. 交付边界（防夹带声明）

**实施**：7 文件 / 101 行最小闭包
- C1：`src/lib/ime.ts`（新增接缝）+ `WorkspaceComposer.vue` / `HomeSessionSidebarRow.vue` / `HomeSessionSidebar.vue`（6 处回车入口接入守卫）
- C2：`HomeStatusPanel.vue`（7 位点）/ `HomeTracePanel.vue`（turn + entry 指标行）/ `AskPanel.vue`（倒计时位）

**明确未做**：不新增 `@theme` / 设计令牌层 / 颜色迁移 / `@keyframes` / diff 视图 / 分栏 resizer / iframe bridge；不改 Rust；不新增 IPC；不删除孤儿组件；不 bump 版本。

**越界文件数 = 0**（机械写域审计）。

---

## 5. 存量缺陷与后续 backlog

| id | 内容 | 处置 |
| :--- | :--- | :--- |
| BASE-E2E-1 | `npm run test:e2e` 基线即 8 failed / 12 passed（`session-sidebar-session-list` 漂移） | PA-134 独立立项 |
| PA-135 | `HomeWorkspace.vue:1369` 守卫粒度过宽，组合态吞掉 Ctrl/Cmd+Z 撤销（存量） | 独立立项 |
| PA-136 | `src/lib/ime.ts:27` 严格判定 vs `HomeWorkspace.vue:1369` 真值判定（当前不可达） | 引入 iframe/webview 前须对齐 |
| PA-130 | 孤儿组件 `src/components/AskPanel.vue` 未被生产代码挂载 | 交 `code-hygiene` |
| PA-131/132/133 | 被拒的三阶决议动作 / Surface 令牌层 / Retina 发丝变体 | 各自独立立项 |
