# wave-pds-3 红相验收收据（L2-T / 独立测试者）—— **契约 v3 版**

- 契约（只读真值）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`（**v3**：v2 修订记录 + 本轮裁定 ①②；文档由 Lead 同步）
- 写域：`tests/acceptance/pds-streaming/`（**只新增本目录**；未改 `src/**`、未改 `tests/acceptance/wave59/**`、未改任何既有测试、未碰 git）
- 分级：**L3**（跨模块 + 真浏览器帧预算 + 计时器/异步竞争 + 需物理凭证）
- 执行时间：2026-11-02（红相，实现尚未落地）
- 环境：Linux / vitest 4.1.9（jsdom 26）/ Playwright 1.60 chromium（headless）；`dist/` 由 `npm run build`（exit 0）产出

---

## 1. v3 裁定落地（本轮，2 处）

| # | 裁定 | 落地点 |
|---|---|---|
| ① | **占位流参数写死进契约**：delta 120ms / 单次 chunk ≤50 字符 / 流式 ≥800 帧 / 空闲 ≥300 帧 | `e2e/*.pw.ts` 固化 `CONTRACT_CHUNK_INTERVAL_MS=120`、`CONTRACT_MAX_CHUNK_CHARS=50`、`CONTRACT_STREAM_FRAMES=900`、`CONTRACT_IDLE_FRAMES=320`；delta 序列由 `buildDeltas()` **在 Node 侧切片**（超 50 字符直接抛错，不靠约定）；`PDS_E2E_CHUNK_MS` / `PDS_E2E_STREAM_FRAMES` / `PDS_E2E_IDLE_FRAMES` 仅保留为调试口，**断言消息统一回显实际参数 + 是否偏离契约值**（`paramsBanner()`） |
| ② | **V8 改为「余量 + 稳定性」双条**：`p95_stream − p95_idle ≤ 6ms`、Long Task=0、**连续 3 次全部满足** | `V8` 重写为内部连跑 3 次的单条门禁（每次新页面：空闲 320 帧 → 流式 900 帧）；3 次实测表（含时间戳）**自动落盘** `receipts/e2e/v8-relative-runs.json`，失败消息打印三次明细；不允许挑最好的一次（全量 `runs` 一起判定） |

## 2. v2 相对 v1 的测试修订（上一轮 5 处，仍有效）

| # | 裁定 | 落地点 |
|---|---|---|
| ① | 撤销 `charsPerTick 4→2`，只改 tick | `contract-motion-rhythm-guards.spec.ts` **R2** 改为「双锁」：`tick === 33` **且** `charsPerTick === 4`（后者锁"不改它"的决定），另加"必须是正整数"的确定性护栏 |
| ② | V8 改相对基线 + Long Task，绝对阈值作废 | `e2e/*.pw.ts` **V8** 重写：同一页面同浏览器，先空闲 ≥300 帧、再流式 ≥800 帧，断言 `p95_stream ≤ p95_idle + 4ms` 且 Long Task(>50ms)=0；**V8-b** 转为**数据采集**（只断言采样有效，打印并落盘 `idle-baseline.json`） |
| ③ | 字面 computed-style 判据移入 Chromium | 新增 **V7-b**（Chromium）：三轨（正常对照组 / `prefers-reduced-motion` / `data-pds-reduce-motion="true"`）断言 `animation-name === "none"` 且尾巴 `mask-image: none`；违规一次性聚合上报 |
| ④ | 降级属性统一 `data-pds-reduce-motion` | `pds-streaming-harness.ts` 的 `REDUCE_MOTION_ATTR` 与规则匹配、`v7-reduced-motion.spec.ts`（V7-2 标题/断言）、V7-5/V7-6 全部改口径 |
| ⑤ | 数量/语义/锚点补齐 | V1：数量恒 **≤1**、有未闭合尾巴文本时 **=1**、块闭合帧 **=0**；V1-5 增「尾巴是源文本**严格后缀**（不 trim）+ `data-tail-len` == 尾文本长度」；R5 改为用 **`[data-pds-tool-row]`** 定位工具行 |
| — | V10 SHA 锚保留、真实子进程保留 | 未改动（`v10-baseline-freeze.spec.ts` 原样） |

---

## 3. 文件清单与用例数（共 43 例）

| 文件 | 覆盖 | 用例数 | 环境 |
|---|---|---|---|
| `pds-streaming-harness.ts` | 共享夹具（非 spec） | — | — |
| `v1-tail-dom-protocol.spec.ts` | **V1**（≤1 / =1 / =0 / 协议自洽 / 行内级 / 无第二尾巴） | 8 | jsdom |
| `v2-tail-reveal-switch.spec.ts` | **V2** + §2 开关（mount-only） | 6 | jsdom |
| `v3-carrier-identity.spec.ts` | **V3** | 3 | jsdom |
| `v7-reduced-motion.spec.ts` | **V7** + §3.5 双轨规则 | 7 | jsdom |
| `v10-baseline-freeze.spec.ts` | **V10**（SHA 锚 + 真实子进程执行） | 3 | vitest 子进程 |
| `contract-motion-rhythm-guards.spec.ts` | §3.1/3.2/3.3/3.4 + §4 + §5（R1–R8） | 8 | jsdom |
| `e2e/pds-streaming-visual.pw.ts` | **V4 / V5 / V6 / V7-b / V8 / V8-b / V9 / V9-b** | 8 | 真 Chromium |
| `e2e/playwright.pds.config.ts` | 本目录专用 playwright 配置（不改仓库根配置） | — | — |

jsdom 侧 35 例 + 浏览器侧 8 例 = **43 例**。

---

## 4. 实跑结果（物理收据）

### 4.1 jsdom（vitest）

```
$ npx vitest run tests/acceptance/pds-streaming/ --reporter=verbose
Test Files  5 failed | 1 passed (6)
     Tests  21 failed | 14 passed (35)
  Duration  58.5s
EXIT=1
```
原始输出：`receipts/jsdom-red-phase.log`

### 4.2 全量单测（含本目录，检查是否误伤既有）

见 `receipts/full-suite-red-phase.log`；失败项**全部**来自本目录的预期红相（21 例），既有用例 0 波及。

### 4.3 真 Chromium（playwright）

```
$ npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts
6 passed / 2 failed (2.5m)
```
- 稳定通过：**V4 / V5 / V9**（基线绿）+ **V8**（v3 新判据下连续 3 次全绿，见 §6）+ **V8-b**（数据采集）+ V9-b（凭据）
- 稳定失败（实现缺失）：**V6**（`tailFrames = 0`）、**V7-b**（降级 computed-style，见 §5）

逐帧聚合：`receipts/e2e/aggregate-summary.json`（900 帧、consoleErrors=[]、scrollHeight 回撤 0 处）
V8 空闲基线：`receipts/e2e/idle-baseline.json`
**V8 连续 3 次实测表（测试自动落盘）**：`receipts/e2e/v8-relative-runs.json`

## 5. 逐条状态（V1–V10 + V7-b）

图例：🔴 实现缺失红｜🟢 **基线绿（非回归护栏）**

| ID | 用例 | 状态 | 现状失败原因（实跑原文摘录） |
|---|---|---|---|
| **V1** | V1-1 数量 ≤1；有未闭合尾巴文本时 =1 | 🔴 | `存在未闭合尾巴文本时 [data-stream-tail] 必须恰好为 1；违规采样：9/9 帧 tailCount=0` |
| | V1-2 三类历史尾巴=0 且无第二尾巴 | 🔴 | `自检：采样窗口内必须至少有一帧存在 [data-stream-tail]` |
| | V1-3 尾巴声明行内级 | 🔴 | `仓库 CSS 中必须存在对 [data-stream-tail] 的 display 声明…命中的尾巴规则：[]` |
| | V1-4 注入真实 CSS 后 computed display 合法 | 🔴 | `内容区必须存在真实尾巴节点，expected null not to be null` |
| | V1-5 `data-tail-len` == 尾长 + 严格后缀 | 🔴 | `必须观测到非空尾巴节点（实际采样：[]）` |
| | V1-6 块闭合帧（无未闭合文本）⇒ 数量 =0 | 🟢 | 基线绿（v2 §1 已定义取值；当前无 tail 自然为 0，断言与自检保留） |
| | V1-7 尾巴机制不得吞内容 | 🟢 | 基线绿 |
| | V1-8 缺省=开启 | 🔴 | `缺省时尾巴必须开启 ⇒ 数量为 1，实际 0` |
| **V2** | V2-1 `off` ⇒ tail 恒 0（含正文完整自检） | 🟢 | 基线绿 |
| | V2-2 `off` ⇒ 全文可见、无历史尾巴 | 🟢 | 基线绿 |
| | V2-3 `off` ⇒ 承载节点身份不变（wave-59 等价） | 🟢 | 基线绿 |
| | V2-4 值非 `off` 不关闭 | 🔴 | `值 "on" ≠ "off" ⇒ 尾巴必须开启，实际 0` |
| | V2-5 开/关两态正文文本一致 | 🔴 | `自检：开启态必须存在尾巴节点，否则退化为「无尾巴 vs 无尾巴」` |
| | V2-6 `off` ⇒ 不挂载（而非隐藏） | 🟢 | 基线绿 |
| **V3** | V3-1 增长期承载节点身份不变 | 🟢 | 基线绿 |
| | V3-2 streaming→done 身份不变 + 内容完整 | 🟢 | 基线绿 |
| | V3-3 尾巴出现前后承载节点不被重建 | 🔴 | `采样窗口内必须出现 [data-stream-tail]（实际每帧尾巴数量：[0×15]）` |
| **V7** | V7-1 系统 reduced-motion ⇒ 尾巴不渲染（含对照组） | 🔴 | `对照组：非降级态必须渲染尾巴，expected +0 to be 1` |
| | V7-2 祖先 `data-pds-reduce-motion="true"` ⇒ 尾巴不渲染（含对照组） | 🔴 | 同上 |
| | V7-3 降级下全文可见 | 🟢 | 基线绿 |
| | V7-4 `primitives.css` 必须被 src 引用 | 🔴 | `命中：[]`（现状从未 import） |
| | V7-5 降级双轨覆盖 尾巴/reasoning/工具入场 | 🔴 | 缺 5 项：`data-stream-tail`（media+attribute）、`assistant-streaming-fade`（attribute）、`transition-item-enter`（media+attribute） |
| | V7-6 降级下 `mask-image: none` | 🔴 | `expected [] to not have a length of +0` |
| | V7-7 禁止"半降级" | 🟢 | 基线绿（拒绝式护栏） |
| **V7-b** | Chromium 三轨 computed-style | 🔴 | 违规 4 条：① 正常态尾巴无 `mask-image: linear-gradient`（实测 `none`）；②③④ 祖先 `data-pds-reduce-motion="true"` 下 **3 个节点仍有动画**（`.conversation-reasoning-panel` / `.conversation-tool-panel` 的 `item-smooth-appear-*`、`.assistant-streaming-fade` 的 `assistant-stream-fade-in`）。**媒体轨道已通过**（既有 `@media` 块覆盖了这三类） |
| **V10** | V10-1 冻结文件哈希未变（8 文件 SHA-256） | 🟢 | 基线绿（v2 裁定保留锚，未迁移） |
| | V10-2 实跑冻结基线 14 + 23 = 37 例 | 🟢 | 基线绿（子进程 exit 0，逐文件计数核对） |
| | V10-3 实跑既有全量单测（排除本目录） | 🟢 | 基线绿（子进程 exit 0） |
| **V4** | 逐帧 scrollHeight 不回撤 >1px | 🟢 | 基线绿（900 帧，回撤点 0） |
| **V5** | 首帧非空后无空白帧 | 🟢 | 基线绿（首内容帧=6 / 首非空帧=6，延迟 0 帧） |
| **V6** | tail 高度 ≤ 行高×1.2 且数量 ≤1 | 🔴 | `必须渲染过 [data-stream-tail]（实际 0 帧）` |
| **V8** | `p95` 增量 ≤6ms 且 LongTask=0，**连续 3 次全满足** | 🟢 | 基线绿（v3 判据下 3/3 pass：Δ=2.00 / 4.50 / 3.20ms，Long Task=0；见 §6）。**注意：实现落地后（tick 60→33ms）必须重测，届时若单次 Δ>6ms 即为真红** |
| **V9** | Console Error = 0 | 🟢 | 基线绿（`consoleErrors: []`，含 pageerror） |

### 附加护栏（§3/§4/§5）

| ID | 断言 | 状态 | 现状 |
|---|---|---|---|
| R1 | `STREAMING_PRESENTATION_TICK_MS = 33` | 🔴 | `expected 60 to be 33` |
| R2 | 双锁：tick 33 **且** `charsPerTick = 4` | 🔴（因 tick） | ① `expected 60 to be 33`；② `charsPerTick` 已是 4（绿）；③ 正整数（绿） |
| R3 | reasoning 尾巴 140ms + translateY(2px) + ease-out | 🔴 | 现 `animationDuration: "350ms"`，无位移 |
| R4 | `item-smooth-appear` 含 `blur(2px)` + 220ms | 🔴 | 关键帧无 blur、时长 200ms |
| R5 | 工具行带 `data-pds-tool-row` + 入场动画类 | 🔴 | `expected 0 to be greater than 0`（无该属性；行 class 仍为 `flex flex-col py-0.5 …`） |
| R6 | 状态行 rise/fall（240/180ms） | 🔴 | 组件内无 fall 规则（`[]`） |
| R7 | 尾巴用静态 `mask-image` 渐隐（非 animation） | 🔴 | 无 `[data-stream-tail]` 规则 |
| R8 | 卸载后遗留定时器 = 0 | 🟢 | 基线绿 |

---

## 6. V8：空闲基线 + 连续 3 次实测表（实现后对比的唯一基准）

**判据（v3 裁定 ②）**：`p95_stream − p95_idle ≤ 6ms` 且 `Long Task(>50ms) = 0`，**连续 3 次运行全部满足**才算通过。

**占位流参数（v3 裁定 ①，默认值即契约值）**：delta 间隔 **120ms**、单次 chunk **≤50 字符**、流式 **900 帧**（≥800）、
空闲基线 **320 帧**（≥300）。断言消息回显：`[params] deltaInterval=120ms maxChunk=50chars streamFrames=900 idleFrames=320 （契约值）`。

**空闲基线（V8-b 留档，`idle-baseline.json`）**

```
空闲基线（同一页面、零 delta、319 帧）：
  p50 = 16.70 ms
  p95 = 16.90 ms
```
（60Hz 物理节拍在 16.6–16.9ms 抖动，这正是不设绝对阈值的理由。）

**连续 3 次实测表（最新一次完整复跑，`v8-relative-runs.json` 原样摘录）**

| 次 | 开始时间 (UTC) | idle p95 | stream p95 | Δ | LongTask(>50ms) | 判定 |
|---|---|---|---|---|---|---|
| #1 | 2026-11-02T14:09:31.500Z | 16.80 | 18.80 | **2.00** | 0 | pass |
| #2 | 2026-11-02T14:09:55.996Z | 17.00 | 21.50 | **4.50** | 0 | pass |
| #3 | 2026-11-02T14:10:20.468Z | 16.80 | 20.00 | **3.20** | 0 | pass |

三次全部满足 ⇒ V8 在红相（实现未落地）为**基线绿**。

**顺带一个反证 v2 的证据**：上表 #2 的 Δ=4.50ms 在 v2 的 4ms 预算下会被判红，而在 v3 的 6ms 下通过 ——
说明 v2 的 4ms 确实是骰子，v3 的「6ms + 连续 3 次」才是有意义的门禁表述。
（历史记录：v2 时期 7 次单跑 Δ = 4.4 / 2.4 / 3.6 / 0.0(200ms 速率) / 5.0 / 4.4 / 2.7，3 fail/4 pass。）

**实现落地后的验收动作（Executors/Lead 必读）**：
1. `npm run build` 后用同一命令跑 V8；
2. 若任一次 Δ>6ms 或出现 Long Task，即为**真红**（不是噪声），需按 §4「允许回调 50ms」处置并写实测数据；
3. 3 次实测表会**自动覆盖** `receipts/e2e/v8-relative-runs.json`，交付说明必须引用最新那份（含时间戳）。

## 7. 实现者注意 / 仍需 Lead 确认的事项

1. **V7-b 的「媒体轨道已绿、属性轨道全红」信息量最大**：既有 `@media (prefers-reduced-motion: reduce)` 已覆盖
   `.conversation-tool-panel` / `.assistant-streaming-fade`（实测 media 轨 0 违规）；但**没有任何**
   `[data-pds-reduce-motion="true"]` 规则（实测 3 个节点仍为 `item-smooth-appear-*` / `assistant-stream-fade-in`），
   且尾巴 `mask-image` 在正常态实测为 `none`（§3.1 未实现）。只需补「属性轨道 + 尾巴遮罩 + import primitives」，媒体轨道别重复造。
2. **`data-pds-tool-row` 是新增必需属性**：R5 先自检锚点、再查该行的入场动画类，实现时一并落地。
3. **仍待 Lead 确认（低风险）**：`@media` 降级块与属性降级块是否都必须覆盖 §3.2/§3.3/§3.4 的全部三类动效。
   当前 V7-5 只锁三类锚点（`data-stream-tail` / `assistant-streaming-fade` / `transition-item-enter`），
   状态行 rise/fall 的降级由 R6 + V7-b 的节点集合覆盖；若你要求"状态行也必须进双轨降级断言"，请示下，我补一条即可。

## 8. 环境限制如实标注

| 项 | 结论 | 依据 |
|---|---|---|
| Chromium 可用性 | ✅（Playwright 1.60 / chromium headless） | 实跑 5 passed / 3 failed |
| jsdom `animation-name` / `mask-image` | ❌ 不可读 ⇒ 字面判据移 V7-b（Chromium） | 已按 v2 裁定 ③ 处理 |
| jsdom `getComputedStyle().display` | ⚠️ 仅对文档内已声明规则可见 | V1-4 显式注入仓库 CSS 后读取 |
| jsdom 布局（scrollHeight/rect） | ❌ 无布局 | V4/V5/V6 一律 Chromium |
| Playwright worker 重启导致产物散落 | ✅ 已修：证据目录改为固定 `$TMPDIR/dsh-pds-wave3-artifacts` | 本轮实测（3 个目录 → 1 个） |
| 帧预算噪声 | ⚠️ 本机空闲 p95 16.8–17.4ms 抖动；多 Agent 并发时 Δ 波动更大 | §5 表格 |

**没有伪造任何通过**：所有 🟢 都是实现前后均成立的护栏；所有 🔴 都是实跑失败并附原文原因。

---

## 9. 复现命令

```bash
# jsdom（V1/V2/V3/V7/V10 + R1–R8）
npx vitest run tests/acceptance/pds-streaming/ --reporter=verbose

# 真浏览器（V4/V5/V6/V7-b/V8/V8-b/V9）—— 需先 npm run build
npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts

# V8 / V8-b 单跑（3 次连续判定 + 空闲基线留档）
npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts -g "连续 3 次|空闲基线帧预算"

# 调试参数（**默认值即契约值**；偏离时断言消息会标注"⚠️ 非契约值：调试参数"）
PDS_E2E_CHUNK_MS=200 PDS_E2E_STREAM_FRAMES=900 PDS_E2E_IDLE_FRAMES=320 \
  npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts -g "连续 3 次"

# 全量
npx vitest run
```
