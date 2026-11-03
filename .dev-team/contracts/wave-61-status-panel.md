# Wave-61 契约（冻结）：上下文圆环迁移 + 状态栏间距/对齐 + 空值隐藏与入场动效

- 契约作者：Lead（一票冻结）；实施：executor；验收：test-agent（只读实施代码，唯一测试编写方）
- 用户原始诉求（真值源）：
  1. 桌面端对话框里的「上下文占用圆环」→ 移到右侧「状态」面板第一行，与「轮次」等计数同行；
  2. 状态栏排列间距优化：图标与值之间没有亲密性、间距与对齐都不美观；
  3. 空值 / 零值不再显示 `—` / `0`，而是**不渲染**；有值时**平滑动效入场**。
- 影响面（唯一允许的写域）：
  - `src/components/HomeStatusPanel.vue`（主改）
  - `src/components/chat/WorkspaceComposer.vue`（移除圆环及其专属 computed）
  - `src/styles.css`（仅当组件 scoped 动效无法满足 reduced-motion 约定时）
  - `tests/**`（test-agent 独占）
- 非目标（Non-Goals，越界即 FAIL 打回）：
  - 不改 `docs/observability.yaml` 的 applicable 判定；不改 Rust / store / projection 语义；
  - 不重命名既有 token/计数 testid（`status-metric-*`），不改 `contextDisplayTokens` 计算口径；
  - 不动 `showContextUsage`（已确认是 Dead prop，属存量技术债，本波不清理）；
  - 不给 `HomeStatusPanel` 增加新业务数据源（一切数据沿用现有 props）。

---

## §1 布局契约（L-*）

**L-1 圆环归属**：上下文占用圆环的唯一渲染位置 = `HomeStatusPanel` 计数行（`[data-testid="status-metrics-counts"]`）第 4 个位点；
`WorkspaceComposer.vue` 内**不得**再存在该圆环（DOM、`data-testid`、专属 computed 一并移除）。

**L-2 同行**：计数行 4 个位点顺序固定 = 轮次 → 模型调用 → 工具调用 → 上下文圆环。
圆环位 testid = `status-context-usage`，**不得**使用 `status-metric-` 前缀（既有 3 位 testid 的计数断言语义必须保持独立）。
圆环语义（迁移自 composer，逐字保真）：百分比 = `clamp(round(tokens/window*100), 1, 100)`（有值且比例 >0 时下限 1%、上限 100%）；
`data-context-debug` 字面量 `in=…;win=…;pct=…;mode=…;speed=…`；`aria-label="上下文用量"`；
**圆环后内联展示百分比文本**（wave-61.1 用户追加诉求）：`<pct>%`，锚点 `data-testid="status-context-usage-percent"`，
与 SVG 同为该位点直接子节点、位于 SVG 之后、`gap-1.5` 分隔、`tabular-nums`；因 pct 已 clamp(1..100)，**永不出现 `0%`**；tooltip 文案
`上下文用量：{pct}% · {used} / {window} · 速度: {n} t/s ({mode})`（无 window 时退化为已用量）。

**L-3 两行同栅格**：计数行与 token 行**必须**同为 4 等列栅格（`grid grid-cols-4 items-center`），
使上下两行 4 个列中心物理对齐；token 行新增锚点 `data-testid="status-metrics-tokens"`。
> 裁决 R9（Lead，取代 R8）：位点缺席时**收列**，不留空列 —— 行内可见位点数为 N 时，
> 列模板为 `repeat(N, minmax(0,1fr))`（`grid-cols-4` class 保留供结构断言，<4 时以内联 style 覆盖）。
> 理由：R8 的显式列位（`col-start-1..4`）虽保证两行列中心对齐，但 `toolCallCount=0`（会话早期常态）时
> 第 3 列整列空置，实测第二组墨迹到圆环间 127.5px 空白（行宽 278px 的 46%，节律 50px vs 127.5px），
> 圆环被读成孤立徽标，**直接违背用户诉求②**（间距/对齐美观）。
> 取舍：两行**严格列中心对齐仅在两行可见位点数相同时成立**（`G-4c` 的 4/4 夹具即此态）；
> 缺席态下优先「无空洞的均匀节律」。
> 迁移说明：既有 wave-17/59 用 class 选择器（`div.flex.items-center.justify-around`）定位 token 行，
> 本契约以 testid 取代 class 锚点（Lead 授权迁移，原意图「token 行恰好 4 个承载位」不变）。

**L-4 图标↔值亲密性（核心）**：每个位点 = `inline-flex items-center justify-center gap-1.5`，内部只有「图标 + 值文本」两个子节点；
**禁止**在图标与值之间放置任何宽度预留盒（即删除值的 `min-w` / `text-right` 组合）。
宽度预留**上移到位点容器自身**（`min-w-*`），保证位数变化时相邻位点左边缘漂移仍 ≤ 0.5px（wave-17 C2-2c 口径不回归）。
预先保留（**Lead 实测校准**，Chromium 11px 文本宽：`1234`=28px、`12.5K`=31px、`1234 t/s`=44px、图标 12 + gap 6）：
右栏 `lg:w-[20rem]` / `xl:w-[21rem]`（`src/App.vue:247,254`），实测指标行可用宽 **278px** ⇒ `grid-cols-4 gap-x-2` 每列track **63.5px**（lg 下 59.5px）。

| 位点 | min-w | 最宽内容 | 结论 |
| :--- | :--- | :--- | :--- |
| 计数位（3） | `min-w-[3rem]` (48) | 18+28=46 | 通过 |
| token 位（输入/输出/缓存） | `min-w-[3rem]` (48) | 18+31=49 | 通过（缓存尾段已移出，见 E-2 / R1） |
| 速度位 | `min-w-[3.5rem]` (56) | 18+44=62（≥1000 t/s 极端值） | 通过（≤63.5；`whitespace-nowrap`，溢出 min-w 亦不换行） |
| 圆环位（含百分比文本） | `min-w-[3rem]` (48) | 图标 14 + gap 6 + `100%` 30 = 50 | 通过（列宽 63.5 兜底，`min-w` 仅为下限） |

任何位点 `min-w` 不得超过 3.5rem（4.25rem=68px 会撑破 63.5px track，Reviewer-A 实测）。

**L-5 行距**：栅格 `gap-x-2`、`gap-y-0`；计数行与 token 行之间保留既有 `border-t` 分隔 + `mt-2/pt-2`；卡内 `px-3 py-2` 不变。

---

## §2 空值契约（E-*）—— 一律「不渲染」，禁止 `—` / `0`

**E-1 计数位**：`turnCount` / `modelCallCount` / `toolCallCount` 为 `0`（或非有限）时，该位点**整个 DOM 节点不渲染**。

**E-2 token 位**：`inputTokensTotal` / `outputTokensTotal` / `cacheHitTokensTotal` / `tokenGenerationSpeed`
为 `null` / 非有限 / `≤ 0` 时，该位点**整个 DOM 节点不渲染**。
> 裁决 R1（Lead，实测驱动）：缓存**比率尾段 `· 12%` 不再在行内渲染**，改为并入缓存位 tooltip 文案
> （实测 11px 文本宽：`12.5K`=31px、`· 12%`=31px ⇒ 含尾段需 86px > 4 等列 track 63.5px，必致换行/溢出；无测试依赖该尾段的 DOM）。

**E-3 圆环位**：`contextDisplayTokens == null` 或 `contextWindowTokens` 缺失/`≤ 0` 时，圆环位**不渲染**。

**E-4 整行折叠**：计数行可见位点数为 0 时，计数行不渲染；token 行可见位点数为 0 时，token 行**及其 `border-t` 一并**不渲染；
两行皆空且无拦截横幅时，卡内容区（`bg-[#f6f0e8]` 容器）**连同其外层 `<section class="border-t pt-3 mt-3">` 整段**不渲染（面板头部保留）。
> 裁决 R5（Lead）：**整段折叠**（不是保留空卡）——否则冷启动（`turnCount=0` 为常态）会在头部下方留 1px 分隔线 + 24px 空带。
> 裁决（防安全 UI 连坐）：**`governance-block-banner`（用量治理拦截横幅）不受 E-4 影响**，
> 横幅存在时必须照常渲染，此时卡容器保留、只折叠两行指标行。安全/拦截语义优先于视觉整洁。

**E-5 严禁**：任何位点出现 `—`、`0`、`≈—` 等占位文本；`formatCompactInteger` 的 `—` 分支随之作废（改为「不渲染」）。

---

## §3 动效契约（M-*）

**M-1 入场**：位点从「不存在」→「有值出现」时播放入场：`opacity 0→1` + `translateY(4px)→0`，时长 `200ms`，
缓动 `cubic-bezier(0.3, 0.7, 0.4, 1)`；圆环位额外 `scale(0.92)→1`。
> 裁决 R4（Lead，机制钉死，反对上游 `<TransitionGroup>` 方案）：**必须用「位点自身静态 class + `@keyframes`」**实现，
> 类名固定 `.status-slot-enter`（圆环位附加 `.status-slot-enter--ring`）。理由：位点外层是 `Tooltip`（根为 Fragment：trigger + Portal），
> `<TransitionGroup>` 对 Fragment 根只 `console.warn` 而不下发 enter 类（已复现）；且冻结 `tests/acceptance/wave61/wave61-motion.spec.ts`
> 只接受「静态 class + @keyframes」口径。「每次挂载即重播」是本契约的**预期语义**（`v-if` 挂载触发）。
**M-2 退场**：不做 leave 动画（即时移除），避免绝对定位幽灵节点与行高抖动。
**M-3 只动合成层**：入场动画只允许 `opacity` / `transform`；禁止动画 `width`/`height`/`margin`/`gap`。
**M-4 降级**：`@media (prefers-reduced-motion: reduce)` 与 `[data-pds-reduce-motion="true"]` 下入场动画关闭（直接出现）；
判定与既有 `src/styles.css:173-255` 的双轨降级约定一致。
**M-5 原地不动**：动效不得改变位点顺序、testid、`tabular-nums` 承载链（wave-17 C2 不回归）。

---

## §4 机器门禁（G-*，全部需物理收据）

- **G-1** `npx vitest run` Exit 0（基线：134 files / 1286 passed / 12 skipped，见 `.dev-team/regression-baseline-wave61.json`）。
- **G-2** `npx vue-tsc --noEmit` Exit 0。
- **G-3** `npm run build` Exit 0。
- **G-4** `npx playwright test tests/e2e/wave61-status-panel.spec.ts` Exit 0，且：
  - G-4a 真实 Chromium 中圆环位于 `status-metrics-counts` 行内、且 composer 区域无圆环；
  - G-4b 空数据 → 位点/DOM 不存在（`count === 0`），有数据 → 出现且入场动画 `getAnimations()` 命中；
  - G-4c 两行 4 列中心物理对齐（列中心 x 差 ≤ 0.5px）；
  - G-4d `console error = 0` + `pageerror = 0`；
  - G-4e 关键旅程截图落盘 `docs/release/evidence/wave-61-status-panel/`（非空，> 10KB）。
- **G-5** 可观测性：`bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base <baseline>` 与
  `check-declaration.sh .` 收据入档（WARN 需在交付说明逐条说明，不得静默放行）。

## §5 验收测试迁移授权（Lead 一票）

下列冻结断言因本契约的**新需求**而失效，由 Lead 授权 test-agent 迁移，**原意图保真**（迁移须逐条写明原意图与等价断言）：

| 文件 | 原断言 | 迁移方式 |
| :--- | :--- | :--- |
| `tests/acceptance/wave59/wave59-f2-status-metrics-layout.spec.ts` | 计数行恰好 3 个 `span.inline-flex` / `div.flex.items-center.justify-around` 定位 token 行 | 计数位 testid 集合仍为 3；token 行改用 `status-metrics-tokens` 锚点 |
| `tests/acceptance/wave17/tabular-nums-contract.spec.ts` | 计数行 `span.inline-flex` 数 = 3；token 行 4 个直接承载 | 3 个计数**位点**；token 行 4 个承载（锚点改 testid） |
| `tests/e2e/wave17-browser-gate.spec.ts` | `measurements.length === 7`；`document.querySelector("div.justify-around")` | 位点总数改 8（3 计数 + 4 token + 1 圆环），token 行改 testid 锚点；0.5px 阈值与单调性断言**原样不变** |
| `tests/e2e/metrics-live-verification.spec.ts` | 圆环可见 + `data-context-debug` 字面量 | 位置随迁移，`data-context-debug` 字面量**原样保留** |
| `tests/acceptance/wave15/browser-smoke.mjs` | 圆环可见性 + `stroke-dashoffset` | 同上，迁移定位 |
| `tests/HomeSidebar.spec.ts` | 速度位空值 `textContent === "—"` | 改为「位点不存在」 |
| `tests/acceptance/wave54/token-speed-truth.spec.ts` | 输入位空值 `text === "—"` | 改为「位点不存在」 |

未被 §5 列出的冻结断言（含 wave-17 C2-2a/b/c/d 阈值、wave-54 估算 `≈` 语义、`agent-runtime-status` 全量）**一律不得改动**。

### §5.1 追加授权（Lead 二轮裁决，Reviewer-A/B BLOCKING 处置）

| 文件 | 原断言 | 迁移方式 | 理由 |
| :--- | :--- | :--- | :--- |
| `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts` | `FROZEN_FILES` 里 wave59-f2 等文件的 SHA-256 锚 | **重锚**：仅重算 §5 授权修改过的文件哈希，`note` 标注 `wave-61 授权重锚` | 该文件用哈希冻结防「改测试让它绿」；§5 授权已成立，锚必须随授权同步，否则 G-1 永红 |
| `tests/e2e/wave17-browser-gate.spec.ts` | 夹具 `toolActivities: []` ⇒ `toolCallCount=0` ⇒ 位点总数上限 7（≠8） | **补一条 `call_tool` 工具活动**（只加数据，不改任何断言与阈值） | E-1 隐藏零值位点后「8 位点」在算术上不可达；补数据可同时验证「3 计数位全可见」 |
| `tests/HomeSidebar.spec.ts` | `:573` 断言面板文本含「生成速度」 | 改为「速度位缺席 ⇒ 其 tooltip 亦缺席；速度位在场 ⇒ tooltip 存在」 | E-2 使速度位在无值时不渲染，其内层 Tooltip 随之消失；原意图（tooltip 语义可达）保留 |

### §5.2 判别力补偿（不得用「结构性恒真」冒充通过）

栅格化后「相邻位点左边缘不漂移」天然成立，会削弱 wave-17 C2-2c 原口径的判别力。故 `tests/e2e/wave61-status-panel.spec.ts` **必须**补一条等价强度的判别断言：
同一 token 位点在 `9 / 12 / 1234` 三档文本下，**位点自身宽度 Δ ≤ 0.5px**（证明 `min-w` 预留真实生效，而非被栅格掩盖）。
wave17 C2-1 迁移后仍须保留结构探针：token 行经 `status-metrics-tokens` 锚点断言**恰好 4 个直接承载子节点**。
