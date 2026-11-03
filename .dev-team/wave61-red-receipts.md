# Wave-61 红相收据（status-panel：圆环迁移 / 间距对齐 / 空值隐藏 / 入场动效）

作者：test-agent（L2-T，独立测试分权方）
契约源（只读）：`.dev-team/contracts/wave-61-status-panel.md`
——已并入 Lead 二轮裁决：§1 L-2 圆环语义、L-4 min-w 校准表、L-5 `gap-y-0`、§2 E-2(R1)/E-4(R5)、§3 M-1(R4)、§5.1 追加授权、§5.2 判别力补偿(R7)
采集时间：2026-11-03 09:33–09:56（Asia/Shanghai）
采集时工作区状态：`src/components/HomeStatusPanel.vue` **未修改**（仍为 wave-59 版），`src/components/chat/WorkspaceComposer.vue` 仍保留 composer 圆环 —— 即「实现尚未落地」的纯净红相。

---

## 0. 结论先行

| 项 | 结果 |
| :-- | :-- |
| 新增 jsdom 验收用例 | **19 条**（L×6 + E×6 + M×5 + composer 走方 L-1×2） |
| 红相（当前实现） | **`npx vitest run tests/acceptance/wave61` → Exit code 1；18 failed / 1 passed（共 19）** |
| 迁移后既有套件（4 个 jsdom 文件） | **Exit code 1；5 failed / 25 passed / 2 skipped（共 32）** |
| R3 重锚验证（`v10-baseline-freeze`） | **V10-1 已由红转绿**；V10-2/V10-3（子进程实跑）红相期必红，属下游传导 |
| 红相纯度（机械判定） | `dev-red-purity.py` → **`[PASS] 有效纯净红相`，Exit code 0**（新增套件与迁移套件两份回执均 PASS） |
| 真实浏览器门禁 | **本阶段未运行**（需 `npm run build`，构建由 Lead 统一执行），见 §6 |
| 基线即绿（未伪造为失败） | 1 条：`M-2 无退场动画`（见 §3.3） |

---

## 1. 交付物清单

| 类别 | 文件 | 说明 |
| :-- | :-- | :-- |
| A · 新验收套件（jsdom） | `tests/acceptance/wave61/wave61-harness.ts` | 共享夹具：挂载 / CSS 面读取解析 / tooltip 文案读取，**非 spec**、无断言语义 |
| A | `tests/acceptance/wave61/wave61-layout.spec.ts` | §1 L-1..L-5 + L-2 圆环语义（6 条） |
| A | `tests/acceptance/wave61/wave61-empty-values.spec.ts` | §2 E-1..E-5，含 E-2(R1) 尾段迁移、E-4(R5) 整段折叠与拦截横幅防连坐（6 条） |
| A | `tests/acceptance/wave61/wave61-motion.spec.ts` | §3 M-1..M-5，按 R4 钉死 `.status-slot-enter(.--ring)` + `@keyframes`（5 条） |
| A | `tests/acceptance/wave61/wave61-composer-scope.spec.ts` | §1 L-1「离开方」：composer 无圆环（DOM 三路取证 + 源码符号级）（2 条） |
| B · 真实浏览器门禁 | `tests/e2e/wave61-status-panel.spec.ts` | §4 G-4a..G-4e + §5.2(R7) 判别力补偿（6 条），截图落盘 `docs/release/evidence/wave-61-status-panel/` |
| C · 迁移 | 7 处（§5）+ 3 处（§5.1） | 见 §5；每处文件头写明「原意图 / 迁移理由 / 等价断言」 |
| D · 红相收据 | 本文件 + `.dev-team/wave61-red-receipt.json`、`wave61-red-vitest.json`、`wave61-red-purity.json`、`wave61-red-receipt-migrated.json`、`wave61-migrated-vitest.json`、`wave61-red-purity-migrated.json`、`wave61-v10-receipt.json`、`wave61-v10-vitest.json` | 命令原文 / Exit code / 失败摘要 / 纯度判定 / R3 重锚验证 |

---

## 2. 命令原文与物理收据

### 2.1 新增套件（红相主证据）

```bash
$ npx vitest run tests/acceptance/wave61 --reporter=dot
# Exit code: 1
# Test Files  4 failed (4)
#      Tests  18 failed | 1 passed (19)
```

```bash
$ npx vitest run tests/acceptance/wave61 --reporter=json --outputFile=.dev-team/wave61-red-vitest.json
# Exit code: 1  →  numTotalTests=19 / numFailedTests=18 / numPassedTests=1
```

### 2.2 红相纯度机械判定（两份回执均 PASS）

```bash
$ python3 ~/.agents/skills/dev-team/scripts/dev-red-purity.py .dev-team/wave61-red-receipt.json \
    --json-out .dev-team/wave61-red-purity.json
[PASS] dev-red-purity 判定为有效纯净红相
# Exit code: 0

$ python3 ~/.agents/skills/dev-team/scripts/dev-red-purity.py .dev-team/wave61-red-receipt-migrated.json \
    --json-out .dev-team/wave61-red-purity-migrated.json
[PASS] dev-red-purity 判定为有效纯净红相
# Exit code: 0
```

判定三要件（脚本口径）：① `status = FAIL`；② `failures` 非空且含异常首行 + 栈头；③ 无 `[HARNESS-TIMEOUT]`。异常类型全部落在 `AssertionError`（允许集合）。

### 2.3 §5 / §5.1 迁移后既有套件

```bash
$ npx vitest run tests/acceptance/wave59/wave59-f2-status-metrics-layout.spec.ts \
    tests/acceptance/wave17/tabular-nums-contract.spec.ts \
    tests/HomeSidebar.spec.ts \
    tests/acceptance/wave54/token-speed-truth.spec.ts --reporter=dot
# Exit code: 1
# Test Files  4 failed (4)
#      Tests  5 failed | 25 passed | 2 skipped (32)
```

| 文件 | 用例 | 失败首行（归因） |
| :-- | :-- | :-- |
| `tests/acceptance/wave59/wave59-f2-status-metrics-layout.spec.ts` | F2-2 / F2-4 | `Unable to get [data-testid="status-metrics-tokens"]` —— 新锚点未实现 |
| `tests/acceptance/wave17/tabular-nums-contract.spec.ts` | C2-1 token 行 | 同上 |
| `tests/HomeSidebar.spec.ts` | 状态面板 token 标签 | `R6 条件等价断言：速度位不存在 ⇒ 其 tooltip 亦不得存在`：实际含 `生成速度` —— 速度位仍渲染 |
| `tests/acceptance/wave54/token-speed-truth.spec.ts` | T5-1 | `inputTokensTotal=null ⇒ 输入位整个 DOM 不渲染`：expected true to be false |

> 2 条 `skipped` 为**存量** skip（与 wave-61 无关，未新增任何 skip）。
> 「R6 正向半边」新增用例（速度位在场 ⇒ tooltip 含「生成速度」）**已绿**，证明原意图双向可验。

### 2.4 R3 重锚验证（§5.1）

```bash
$ npx vitest run tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts --reporter=dot
# Exit code: 1
# Tests  1 obtained: V10-1 通过；V10-2 / V10-3 失败（子进程实跑，红相期必红）
```

| V10 子项 | 状态 | 说明 |
| :-- | :-- | :-- |
| V10-1（SHA-256 锚定） | **PASS**（重锚前为 FAIL） | 仅重算被 §5 授权改动的 `wave59-f2-status-metrics-layout.spec.ts` 一个文件哈希，其余 7 个锚逐字未动 |
| V10-2（实跑 14 + 23 例） | FAIL | 下游传导：wave59 目录内 F2-2/F2-4 等价断言在等新实现 |
| V10-3（实跑既有全量单测） | FAIL | 同上；V10-3 本次实跑耗时 241s，退出码 1 |

### 2.5 未运行项（显式声明，不冒充）

| 项 | 原因 | 执行方 |
| :-- | :-- | :-- |
| `npx playwright test tests/e2e/wave61-status-panel.spec.ts`（G-4） | 需 `npm run build` 产物 + `vite preview:4175`；构建统一由 Lead 执行 | Lead |
| `npx vitest run`（全量 G-1） | 覆盖率门禁与回归基线属 Lead 门禁 | Lead |
| `node tests/acceptance/wave15/browser-smoke.mjs` | 需自起 vite dev server + 系统 Chrome（`channel: "chrome"`） | Lead / CI |

---

## 3. 失败归属 SUT（而非测试基建）的取证

### 3.1 当前实现的可观测事实（红相 dump 实测）

- 计数行 `class="grid grid-cols-3 items-center gap-2 …"`（契约：`grid-cols-4` + `gap-x-2` + `gap-y-0`）；
- token 行 `class="mt-2 flex items-center justify-around border-t …"`，**无** `data-testid="status-metrics-tokens"`；
- 位点容器 `gap-1`、无 `min-w`（契约：`gap-1.5` + 容器 `min-w-[3rem]`），值 span 携带 `min-w-[1.75rem] text-left` / `min-w-[2.25rem] text-right`（L-4 明令删除）；
- 缓存位行内仍渲染 `· 12%` 尾段（3 个子节点；裁决 R1 要求移出行内）；
- **不存在** `data-testid="status-context-usage"`（圆环未迁入）、无 `aria-label="上下文用量"`、无 `data-context-debug`；
- 空值仍渲染占位：全空夹具下面板文本实测为 `"状态000————"`（E-5 禁止）；
- 无 `.status-slot-enter` / `.status-slot-enter--ring`、`HomeStatusPanel.vue` 无 `<style>`、`src/styles.css` 无位点入场规则（M-1 缺失）；
- composer 仍存在 `workspace-context-usage-circle`、`contextUsagePercentage`、`contextDebugString`、`stroke-dashoffset`（L-1 要求一并移除）。

### 3.2 为什么不是测试基建缺陷

- 全部失败均为**断言在已挂载被测组件树上求值**所得（`AssertionError` / 带病因说明的锚点断言），无 TS/语法错误（`npx esbuild` 全绿）、无 import 失败、无 Harness 超时、无 skip；
- 每条断言配**反向哨兵**（同夹具下其他位点仍在 / 行仍在），并有负向守卫（E-3 正向对照、M-2 无 leave、R6 双向 tooltip），排除「空树恒真」与「结构性恒真」；
- 夹具不依赖实现私有符号：只用契约点名 testid、现有 props 面、`HomeStatusPanel.vue <style>` 与 `src/styles.css` 两处真实打包面。

### 3.3 基线即绿的一条（已登记 `already_green_in_baseline`，未伪造为失败）

| 用例 | 为什么基线即绿 |
| :-- | :-- |
| `M-2: 不做退场动画` | 基线既无 leave 规则/关键帧也无 `TransitionGroup`（因为还完全没有入场动效），属**负向守卫**；实现入场后此条才有区分力 |

---

## 4. 契约条目 ↔ 用例名映射

### §1 布局（L-1..L-5）

| 条目 | 用例 |
| :-- | :-- |
| L-1（面板侧） | `wave61-layout.spec.ts` → `L-1: 上下文圆环的唯一渲染位置 = status-metrics-counts 行内，且面板内恰好 1 处` |
| L-1（composer 走方） | `wave61-composer-scope.spec.ts` → `L-1(离开方): WorkspaceComposer 内不存在圆环 DOM（testid / aria-label / dashoffset 三路取证）`、`L-1(离开方): WorkspaceComposer.vue 源码不再包含圆环 testid 与专属 computed` |
| L-2（位点序 + 命名） | `wave61-layout.spec.ts` → `L-2: 计数行 4 位点顺序固定为 轮次→模型调用→工具调用→圆环；圆环 testid 不得带 status-metric- 前缀` |
| L-2（圆环语义，二轮新增） | `wave61-layout.spec.ts` → `L-2(圆环语义): 百分比 clamp(round(tokens/window*100),1,100) + data-context-debug 字面量 + aria-label + tooltip 文案` |
| L-3 | `wave61-layout.spec.ts` → `L-3: 计数行与 token 行必须同为 4 等列栅格（grid grid-cols-4 items-center），token 行锚点 status-metrics-tokens` |
| L-4 | `wave61-layout.spec.ts` → `L-4: 位点 = inline-flex/justify-center/gap-1.5 且 min-w 预留上移到容器；图标↔值之间无宽度预留盒`（含 min-w ≤ 3.5rem 上限断言） |
| L-5 | `wave61-layout.spec.ts` → `L-5: 栅格 gap-x-2；计数行/token 行之间保留 border-t 分隔与 mt-2/pt-2；卡内 px-3 py-2 不变`（含 `gap-y-0`） |

### §2 空值（E-1..E-5）

| 条目 | 用例 |
| :-- | :-- |
| E-1 | `E-1: 计数位为 0 / 非有限值时，该位点整个 DOM 节点不渲染（逐位点隔离验证）` |
| E-2 | `E-2: token 位为 null / 非有限 / ≤0 时不渲染（逐位点隔离验证）` |
| E-2（R1） | `E-2(裁决 R1): 缓存比率尾段不再行内渲染（并入缓存位 tooltip）—— 行内只留图标+值，比率信息不丢失` |
| E-3 | `E-3: contextDisplayTokens 为 null 或 contextWindowTokens 缺失/≤0 时，圆环位不渲染`（含正向对照） |
| E-4（R5） | `E-4: 计数行/token 行各自为空时整行折叠；两行皆空且无拦截横幅时外层 section 整段折叠（裁决 R5）` |
| E-5 | `E-5: 任何位点不得出现 — / 0 / ≈— 等占位文本（有值态与全空态双向验证）` |

### §3 动效（M-1..M-5，R4 钉死机制）

| 条目 | 用例 |
| :-- | :-- |
| M-1 | `M-1: 入场动画存在且参数精确（200ms / cubic-bezier(0.3,0.7,0.4,1) / opacity 0→1 / translateY(4px)→0 + 圆环 scale(0.92)）` —— 断言位点根元素携带静态 class `status-slot-enter`、圆环另加 `status-slot-enter--ring`，规则用 `@keyframes` 承载 |
| M-2 | `M-2: 不做退场动画（位点相关 CSS 面不得出现 leave/exit 规则或关键帧）`（含 `<TransitionGroup>` 负断言） |
| M-3 | `M-3: 只动合成层（入场声明与关键帧不得驱动 width/height/margin/gap 等布局属性）` |
| M-4 | `M-4: reduced-motion 双轨降级（@media prefers-reduced-motion 与 [data-pds-reduce-motion="true"] 均关闭入场）` |
| M-5 | `M-5: 动效不改变位点顺序 / testid / tabular-nums 承载链（wave-17 C2 不回归）` |

### §4 机器门禁（G-4a..G-4e）+ §5.2（R7）→ `tests/e2e/wave61-status-panel.spec.ts`

| 条目 | 用例 |
| :-- | :-- |
| G-4a | `G-4a: 真实 Chromium 中圆环位于 status-metrics-counts 行内，且 composer 区域无圆环`（含 `aria-label="上下文用量"`） |
| G-4b | `G-4b: 空数据 → 位点/DOM 不存在（count=0）；注入数据 → 位点出现且入场动画 getAnimations() 命中` |
| G-4c | `G-4c: 两行同为 4 列时，上下 4 个列中心的 x 坐标差 ≤ 0.5px` |
| §5.2(R7) | `§5.2(R7): 同一 token 位点在 9 / 12 / 1234 三档文本下自身宽度 Δ ≤ 0.5px（min-w 预留真实生效）` |
| G-4d | `G-4d: 首页加载 + 数据注入 + 状态面板渲染全程 Console Error = 0 且未捕获异常 = 0` |
| G-4e | `G-4e: 关键旅程截图落盘 docs/release/evidence/wave-61-status-panel/（存在且 > 10KB）` |

---

## 5. 迁移清单（§5 七处 + §5.1 三处，原意图保真）

> 铁律：**只改授权列出的条目**。§5 之外（wave-17 C2-2a/b/c/d 阈值、wave-54 `≈` 语义、`agent-runtime-status` 全量）一律未动。

| # | 文件 | 迁移动作 | 等价性 |
| :-- | :-- | :-- | :-- |
| 1 | `tests/acceptance/wave59/wave59-f2-status-metrics-layout.spec.ts` | `TOKEN_ROW`：`div.flex.items-center.justify-around` ⇒ `[data-testid="status-metrics-tokens"]` | 原意图「token 行恰好 4 个直接承载位、位于卡内、位于计数行之后」一字不改 |
| 2 | `tests/acceptance/wave17/tabular-nums-contract.spec.ts` | C2-1 计数位 `span.inline-flex` ⇒ `[data-testid^="status-metric-"]`（排除圆环位）；token 行改 testid 锚点 | 位点集合仍为既有 3 个计数位 + 4 个 token 位；逐位点 tabular-nums + 数值自检原样 |
| 3 | `tests/e2e/wave17-browser-gate.spec.ts` | `measurements.length 7 ⇒ 8`；token 行改 testid 锚点；圆环位纳入测量（走既有 PROBE-NODE 探针） | 0.5px 阈值（C2-2b/c）与宽度单调不减（C2-2d）**原样不变** |
| 3b | 同上（§5.1 / R2） | 夹具补一条 `call_tool` 工具活动（`traceTimeline` + `toolActivities`） | 只加数据：E-1 隐藏 0 值位点后「8 位点」在算术上必须可达 |
| 4 | `tests/e2e/metrics-live-verification.spec.ts` | 圆环锚点 ⇒ `status-context-usage` | `data-context-debug` 三处字面量、可见性、`circle` 结构、dashoffset 容差 0.6 原样保留 |
| 5 | `tests/acceptance/wave15/browser-smoke.mjs` | 圆环探针改为「composer 旧锚点计数=0 / 面板圆环锚点计数 / 有则读 dashoffset」并随 E-3 调整门禁组合 | 原意图拆成两条可判定不变量（无数据⇒不渲染；出现⇒dashoffset 可读） |
| 6 | `tests/HomeSidebar.spec.ts`（§5） | 速度位空值 `textContent === "—"` ⇒ 位点不存在 + 面板无 `—` | 原意图「无值不显示数字」强化为「既无数字也无占位符」 |
| 7 | `tests/acceptance/wave54/token-speed-truth.spec.ts` | T5-1：`text === "—"` ⇒ 位点不存在 + 反向哨兵 + 面板无 `—` | 原意图「禁止渲染 0」保持 |
| 8 | `tests/HomeSidebar.spec.ts`（§5.1 / R6） | `:573` tooltip 名单去掉「生成速度」，改为 `arrayContaining(["输入","输出","缓存读取"])`；追加条件等价断言（速度位缺席⇒tooltip 缺席）；新增正向用例（速度位在场⇒tooltip 在） | 原意图「tooltip 语义可达」**双向可验**（正向用例本次已绿） |
| 9 | `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts`（§5.1 / R3） | `FROZEN_FILES` 中 wave59-f2 的 SHA-256 重锚（`note: "wave-59 F2（4 例）· wave-61 授权重锚"`）+ 文件头登记授权 | 仅重算**被 §5 授权改动的 1 个文件**；其余 7 个锚逐字未动；V10-2/V10-3 判据不变。V10-1 已由红转绿 |

---

## 6. Lead 二轮裁决 R1–R7 落地证据

| 裁决 | 落地位置 | 落地证据（当前红相下的实测行为） |
| :-- | :-- | :-- |
| **R1**（E-2 尾段移出） | `wave61-empty-values.spec.ts` → `E-2(裁决 R1)`；`wave61-layout.spec.ts` L-4 同步改为「缓存位行内只剩 2 子节点」 | 当前实现 **3 个子节点** → 断言红：`E-2（R1）：缓存位行内必须只剩「图标 + 值」两个子节点；实际 3 个` |
| **R2**（wave17 夹具补 call_tool） | `tests/e2e/wave17-browser-gate.spec.ts` 的 `seedTokenMetricFixture`（traceTimeline + toolActivities 各一条） | 夹具已补；文件头登记 §5.1 授权；断言/阈值零改动（需 G-3 构建后由 Lead 跑） |
| **R3**（v10 重锚） | `v10-baseline-freeze.spec.ts` FROZEN_FILES 单条重锚 | **V10-1 由红转绿**（见 §2.4），其余锚逐字未动 |
| **R4**（M-1 机制钉死） | `wave61-motion.spec.ts` M-1（静态类 + `@keyframes`）、M-2（`TransitionGroup` 负断言）、harness 常量 `SLOT_ENTER_CLASS` / `RING_ENTER_CLASS` | 当前实现无 `.status-slot-enter` → M-1 红；机制已从「容忍 transition」收紧为「必须 `@keyframes`」 |
| **R5**（E-4 整段折叠） | `wave61-empty-values.spec.ts` E-4 第三/四段 | 断言「外层 `pt-3 mt-3` section 不存在」+「无残留 `border-t`」+「拦截横幅在场时卡容器保留、仅折叠两行」。当前实现两者皆渲染 → 红 |
| **R6**（HomeSidebar tooltip） | `tests/HomeSidebar.spec.ts` `:573` 名单 + 条件等价断言 + 新增正向用例 | 条件断言当前红（速度位仍渲染 ⇒ tooltip 仍在）；正向用例已绿 |
| **R7**（§5.2 判别力补偿） | `tests/e2e/wave61-status-panel.spec.ts` → `§5.2(R7)` | 三档文本改写位点内数字节点，断言自身宽度 Δ ≤ 0.5px + 无溢出（≤0.5px）。需真实 Chromium（Lead 跑） |

### 6.1 需要 Lead 裁决的一条口径冲突（R1 × R6）

- R6 要求 `tests/HomeSidebar.spec.ts` 的 tooltip 名单为 `arrayContaining(["输入","输出","缓存读取"])`（**精确**元素）；
- R1 要求缓存比率尾段并入「缓存位 tooltip 文案」。
- 若 executor 把比率**拼进**「缓存读取」这一条文案（如 `缓存读取 · 命中率 12%`），精确匹配会失败（该元素不再等于 `缓存读取`），而 E-2(R1) 的「某条 tooltip 含 12%」仍成立。
- 当前实现是**两条独立 tooltip**（`缓存读取` 与（迁移后新增的）比率文案），因此两者可以同时满足。**建议 executor 保持两条独立 tooltip**；若选择合并文案，请 Lead 授权把 R6 该行改为 `startsWith` 前缀匹配（意图不变）。

---

## 7. jsdom 无法覆盖、必须由 Lead 在真实浏览器把关的条目

> 依据：vitest/jsdom 不执行 Tailwind、不注入 SFC `<style>`、无布局引擎（`getBoundingClientRect` 恒 0）。

| 条目 | 为什么 jsdom 证不了 | 把关位置 |
| :-- | :-- | :-- |
| L-3 列中心物理对齐 | 需真实盒模型 | `wave61-status-panel.spec.ts` G-4c（≤0.5px + 等列宽度哨兵） |
| L-4 的物理后果（`min-w` 真生效） | 需真实排版 | `wave61-status-panel.spec.ts` §5.2(R7)；`wave17-browser-gate.spec.ts` C2-2（阈值未改） |
| M-1 物理动效 | jsdom 不跑 CSS 动画 | G-4b（`animationstart` 内同步 `getAnimations()` + `getKeyframes()` 取样 200ms / easing / opacity / translateY / scale(0.92)） |
| M-4 降级的**真实生效** | jsdom 不解析媒体查询 | 本波 jsdom 仅断言「双轨规则存在且覆盖入场 class」。**真实生效建议 Lead 追加 `page.emulateMedia({ reducedMotion: "reduce" })` 后断言 `getAnimations().length === 0`**（未擅自扩写契约之外的 e2e 条目，待裁决） |
| E-4 整段折叠的视觉后果 | 需真实间距/边框 | G-4b 的 DOM 侧 + `docs/release/evidence/wave-61-status-panel/*.png` 视觉复核 |
| E-3 / 空值态真实渲染 | 与 M-4 类同（需真实计算样式） | G-4b 空值段（`count === 0`）+ 截图 |
| G-4d console/pageerror = 0 | jsdom 无浏览器运行时 | G-4d（收集器在 `page.goto` 前挂载） |

---

## 8. 数据与环境清理

- 未写 `src/**`；未做任何 git 写操作（仅 `git status` 只读查看）；`tests/**` 之外无改动（除本文件与 `.dev-team/wave61-*.json` 收据）；
- 临时产物全部落在 `/tmp`（`/tmp/w61-*.log`、`/tmp/wave61-*-vitest.json`、`/tmp/w61e2e.js`），仓库内只保留 §1 列出的证据文件；
- 未新增任何 `skip` / `.only` / 吞异常；临时探针文件 `tests/acceptance/wave61/__tmp_probe.spec.ts` 已删除（用于验证 `collectTooltipTexts` 可读到 Tooltip 文案：实测返回 11 条真实文案）。
