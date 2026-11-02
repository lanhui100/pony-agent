# wave-pds-3 验收收据（L2-T / 独立测试者）—— **契约 v5 版**

- 契约（只读真值）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`（**v5**：v2/v3/v4/v5 修订记录累积）
- 本轮（v5）新增 **V11 尾巴切分语义** 9 例、改 **V1-5**（`data-tail-len`=源尾巴字符数）、改 **V6**（单行/多行分流判据）
- 写域：`tests/acceptance/pds-streaming/`（**只新增本目录**；未改 `src/**`、未改 `tests/acceptance/wave59/**`、未改任何既有测试、未碰 git）
- 分级：**L3**（跨模块 + 真浏览器帧预算 + 计时器/异步竞争 + 需物理凭证）
- 执行时间：2026-11-02（红相，实现尚未落地）
- 环境：Linux / vitest 4.1.9（jsdom 26）/ Playwright 1.60 chromium（headless）；`dist/` 由 `npm run build`（exit 0）产出

---

## 0. v11 裁定落地（V8 新增 `[SPIKE]` 容许重测；两项张力结案）

| # | 裁定 | 落地点 |
|---|---|---|
| `$$` | **选项 A**：V14 清单里 `$$` 那条作废；保持"只断言尾巴内无块级标签 + 文本无丢失" | `V14-10` / `V12-12` 保持原写法（判据与渲染器同源，源文本形态不参与判定） |
| loose list | 接受"loose list 不可能整体落在未闭合块"为**切分器结构约束**，非缺陷；要求降到"不丢行、不新增 HEAD 之外的结构抖动" | `V12-6` 现状即符合（边界形态 + 文本无损断言，不断言"单一 `<ul>`"） |
| **V8 瞬时尖峰** | 新增 `[SPIKE]` 容许重测：某轮**仅** `B−A > 4ms` 失败且 `A−idle ≤ 6ms`、两段 Long Task=0 ⇒ 标 `[SPIKE]`，**不计入 3 轮干净轮、不直接 FAIL**，自动补测；总尝试上限 **8 轮**；`[SPIKE]` 轮**必须落盘**（与 `[LOAD-CONTAMINATED]` 同级） | `e2e/*.pw.ts` 的 V8：新增 `SPIKE_TAG`、`MAX_ROUNDS 6→8`、`onlyAbOverBudget` 判定；`summary.spikes` 计数；落盘字段 `tag` / `verdict:"spike"` |

**`[SPIKE]` 分支可达性实证**（不是纸面设计）：用调试口 `PDS_E2E_FORCE_SPIKE=1`（B 段每帧注入 ~30ms 纯计算，
单帧 < 50ms 故不产生 Long Task）强制触发 —— 两轮均被标 `[SPIKE]`、`B−A=+21.4/+21.5ms`、`LT=0/0`，
测试以"0 轮干净 / 2 轮 [SPIKE]"失败并**把两轮完整落盘**。证据：`receipts/e2e/v8-spike-branch-proof.json`。
（默认 `PDS_E2E_FORCE_SPIKE` 关闭，验收跑不受影响。）

## 0.1 v9 裁定落地（上一轮：V14 资格门完备性 + V12 补齐/修正 + 两处契约张力上报）

| # | 裁定 | 落地点 |
|---|---|---|
| V14（新组，12+1 例） | 资格判定改「渲染结果自证」：尾巴 span 内**不得有块级标签**（②才是真判据） | `v14-eligibility-completeness.spec.ts`：tab 缩进 / 四空格 / `***` / `___` / 单 `-` setext / `#` / `##` / 四反引号 / CRLF / `$$` / HTML 块 / HTML 注释 + **V14-0 反向自检**（纯段落必须发尾巴且尾巴内无块级标签）|
| V12 补齐 | rule 5b / 6 / 7 原本**没有夹具**（删掉规则仍全绿） | 新增 `V12-9`（段落 `\|` + 分隔行 ⇒ 表格）/ `V12-10`（HTML 块）/ `V12-11`（`===` setext）/ `V12-12`（`$$`，见 §7.3 分歧） |
| V12-5 修正 | 原夹具 `isSimpleTextContent=true` ⇒ 走纯文本快路径、**不进 MarkdownRenderer**（"路径未到"的假绿） | 前缀改为 `已定稿 **加粗** 段落。`，并加断言**必须渲染出 `<strong>`** 作为"确实走了 markdown 路径"的前置证明（V14 同样加） |
| V12-6 修正 | 原尾巴只有 `- 项目 B`，N5（loose list）未被覆盖 | 夹具改为 N5 的**边界形态** `- 项目 A\n\n- 项目 B`；并说明"loose list 整体落进未闭合块"在 `splitMarkdownBlocks` 语义下**结构上不可能**（见 §7.4） |
| V13（v8） | 纯文本快路径尾巴（5 例） | `v13-plain-text-tail.spec.ts`：V13-1 数量/源尾字符数、V13-3 off/降级两路文本逐字一致、V13-4 **全等**、V13-4b 慢速释放前缀不变量、V13-5 done 无残留；V13-2（mask 单行/多行）在 Chromium |

**V14 首跑红相证据（实现未改前）**：12 条构造里 **6 红 / 6 绿**，红的正是 v7 正则漏网的 6 类，失败信息里能直接看到**尾巴 span 内出现块级标签**：
- `V14-1` tab 缩进：`<span data-stream-tail …><pre class="code-block-cream"><code>const z = 1;`
- `V14-3` `***` / `V14-4` `___`：`<span …><p>正文一行</p>\n<hr>`
- `V14-5` 单 `-` setext：`<span …><h2>标题一行</h2>`
- `V14-6` `# 标题`：`<span …><h1>标题一行</h1>`；`V14-7` `## 标题`：`<h2>`
其余 6 条（四空格/四反引号/CRLF/`$$`/HTML 块/HTML 注释）在 v7 下已被拦，属绿。
实现侧随后按 v9（render-then-verify）改写，复跑 **V14 13/13 全绿** —— 与 V12-3（N3）一样，这组也不是"碰巧为绿"。

## 0.2 v8 裁定落地（更早：V13 纯文本快路径尾巴）

| # | 裁定 | 落地点 |
|---|---|---|
| V12（新组，8+1 例） | **尾巴资格门**：只有「纯段落」未闭合块发尾巴；围栏/缩进代码/列表/引用/表格/HTML/setext/`$$` 一律不发（全额 stableHtml）；删除「尾巴内嵌 `<pre><code>`」路径 | `v12-tail-eligibility.spec.ts`：8 类构造各一条（N1 未闭栅栏不在尾巴首行 / ~~~ / ```md（N3）/ ```js info string（N4）/ 缩进代码 / loose list（N5）/ 引用 / 表格），每条同时断言 ① 尾巴数 = 0（且无 `data-tail-len`）② `pre` 外无字面围栏 + 与 `renderMarkdown(全文)` 文本对齐（无行丢失）③ done 帧无内容跳动；另加 **V12-0 资格门边界自检**（纯段落必须发尾巴，防"永不发尾巴"作弊） |
| V11 补一条 | 纯段落尾巴（含行内加粗/代码/链接）必须发尾巴且语义正确 | `V11-7`：尾巴数=1、`data-tail-len`=源尾字符数、`<strong>/<code>/<a>` 齐备、可见文本无 `**` / 反引号 / `](` 残片 |
| R6（R-B5） | `data-pds-status ≠ null` 分支必须有**真** rise 断言（上一轮 rise 空转抓不到） | jsdom `R6` 增 Assert ④：`[data-pds-status="enter"]` 的 `animation` 必须**并列**含 shimmer 与 rise；另加 Chromium **`R6-b`**：读真实 label 的 computed `animation-name`，必须同时含 shimmer 与 rise（jsdom 不实现 animation-name 取值，故真判据放浏览器） |
| 文档纠正 | RECEIPT §8.1 把"宽度钳制/按标点前推两分支零覆盖"从**覆盖缺口**改成**分支已不存在** | §8.1 已改写（见该节） |

**V12 红相证据（已转绿）**：首次运行（23:49）时 **V12-3 红**，命中 N3 —— 失败原文
`实际="已定稿段落。标题正文一行"；期望="已定稿段落。#标题正文一行"`（` ```md ` 内的 `# 标题` 被当标题渲染，丢了 `#`）。
实现侧随后修复（改动在 `MarkdownRenderer.vue`，未越界改 `src/lib/markdown.ts`），复跑全绿。
⇒ 本组不是"碰巧为绿"：它抓到了审查者报的 N3，并随修复转绿。详见 §5.4。

## 0.3 v6 裁定落地（更早：V8 改「同轮 A/B 差分」）

| # | 裁定 | 落地点 |
|---|---|---|
| v6 | V8 不再用"相对 idle 基线"，改为**同轮 A/B 差分**：同一页面依次 `idle` → **A（`tail-reveal=off`，旧行为）** → **B（`tail-reveal=on`，新行为）**；三条判据 `p95(B)−p95(A) ≤ 4ms`、A/B 两段 Long Task=0、`p95(A)−p95(idle) ≤ 6ms`（框架合理性）；连续 3 轮全满足；某轮 `p95_idle` 偏离记录基线 16.90ms 超 1.5ms ⇒ 标 `[LOAD-CONTAMINATED]` 重测（≤6 轮），被剔除轮次**同样落盘** | `e2e/*.pw.ts` 的 `V8` 重写（`startSyntheticStream` 增加 `tailReveal` 参数、每段单独统计 Long Task、`waitForHarnessStopped` 显式等待）；落盘 `receipts/e2e/v8-relative-runs.json`（每轮 idle/A/B 三段 p95 + B−A + A−idle + 两段 Long Task + 污染标记 + 判定） |

**为什么必须改（证据）**：旧判据实测三次 Δ = **3.1 / 7.3 / 3.0 ms** —— 失败那轮 idle 干净（16.90），stream 却 24.20，
是**测量期间**被外部负载污染；相对基线只校正"机器整体偏慢"，校正不了"窗口内漂移"。
旧判据的翻车快照已留档：`receipts/e2e/v8-old-criterion-evidence.json`（3 轮里 1 轮 fail、2 轮 idle 漂移 +2.0/+1.9 却判 pass）。

## 0.4 v5 裁定落地（更早，3 处）

| # | 裁定 | 落地点 |
|---|---|---|
| R-A1/R-A2（新增 **V11**，9 例） | 尾巴 = `splitMarkdownBlocks` 的**整个未闭合块**；禁止"按最后一个 `\n` 切分" | 新文件 `v11-tail-split-semantics.spec.ts`：表格 / setext 标题 / 跨行加粗 / 嵌套列表 / 多行段落 / 栅栏闭合帧 + `V11-6a/6b`（代码尾内嵌、提交帧结构等价）+ **`V11-0` 判据自检**。全部用**含 markdown 语法的尾巴**驱动真渲染，断言"已定稿区 + 尾巴"拼接后的整体语义 |
| R-A3（改 **V1-5**） | `data-tail-len` = **源尾巴字符数** | 期望值改为 `splitMarkdownBlocks(source).tail.length` **现算**（不写死数字）；夹具换成含 markdown 的尾巴（`**加粗\n尾巴**` / 表格 / 嵌套列表），使 v4 的"渲染后可见文本长度"语义与之必然不等；纯文本尾仍保留"严格后缀、不 trim"判据 |
| R-A4（改 **V6**，Chromium） | 取消"≤1 行"硬约束：单行尾判高度、多行尾判遮罩 | V6 改为：按**文本行盒数**（逐文本节点 Range rect 的 `top` 去重）分单行/多行；单行 ⇒ 高度 ≤ 行高×1.2；多行 ⇒ 不断言高度，断言 `mask-image: none`；高度单调性交给 V4。**分类必须与高度无关**（用高度反推高度=循环论证） |

## 1. v3 裁定落地（上一轮，2 处）

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

## 3. 文件清单与用例数（共 86 例）

| 文件 | 覆盖 | 用例数 | 环境 |
|---|---|---|---|
| `pds-streaming-harness.ts` | 共享夹具（非 spec） | — | — |
| `v1-tail-dom-protocol.spec.ts` | **V1**（≤1 / =1 / =0 / 协议自洽 / 行内级 / 无第二尾巴） | 8 | jsdom |
| `v2-tail-reveal-switch.spec.ts` | **V2** + §2 开关（mount-only） | 6 | jsdom |
| `v3-carrier-identity.spec.ts` | **V3** | 3 | jsdom |
| `v7-reduced-motion.spec.ts` | **V7** + §3.5 双轨规则 | 7 | jsdom |
| `v10-baseline-freeze.spec.ts` | **V10**（SHA 锚 + 真实子进程执行） | 3 | vitest 子进程 |
| `contract-motion-rhythm-guards.spec.ts` | §3.1/3.2/3.3/3.4 + §4 + §5（R1–R8） | 8 | jsdom |
| `v11-tail-split-semantics.spec.ts` | **V11**（尾巴切分语义 + 判据自检 + v7 的纯段落尾巴） | 9 | jsdom |
| `v12-tail-eligibility.spec.ts` | **V12**（资格门：构造语义 + boundary 自检 + rule 5b/6/7/8 夹具） | 13 | jsdom |
| `v13-plain-text-tail.spec.ts` | **V13**（纯文本快路径尾巴：数量/开关降级/逐字全等/done） | 5 | jsdom |
| `v14-eligibility-completeness.spec.ts` | **V14**（资格门完备性：12 类块级构造 + 反向自检 + 同源自检 V14-0b） | 14 | jsdom |
| `e2e/pds-streaming-visual.pw.ts` | **V4 / V5 / V6 / V7-b / V8 / V8-b / V9 / V9-b / V13-2 / R6-b** | 10 | 真 Chromium |
| `e2e/playwright.pds.config.ts` | 本目录专用 playwright 配置（不改仓库根配置） | — | — |

jsdom 侧 **76 例** + 浏览器侧 **10 例** = **86 例**。

---

## 4. 实跑结果（物理收据）

### 4.1 jsdom（vitest）

```
$ npx vitest run tests/acceptance/pds-streaming/ --reporter=verbose
Test Files  10 passed (10)
     Tests  76 passed (76)
EXIT=0        ← v9 实现已落地（V12 的 N3、V14 的 6 条块级漏网均先红后绿，见 §0 / §5.4）
```
原始输出：`receipts/jsdom-red-phase.log`（**红相历史**）与本次 green 输出（见 §5.2）

### 4.2 全量单测（含本目录）

```
$ npx vitest run
Test Files  130 passed (130)
     Tests  1251 passed | 12 skipped (1263)
EXIT=0
```
摘要：`receipts/full-suite-v7-green.log`；jsdom 逐条：`receipts/jsdom-v7-green.log`（红相历史见 `jsdom-red-phase.log`；v5 时点见 `jsdom-v5-green.log`）。

> V10-3（"全量套件里再跑一遍全量套件"）曾在默认并行度下因 CPU 争抢把仓库里 5s 超时的 `check-secrets`
> 全仓扫描挤到超时，制造过 1 次与实现无关的假红；已把**子进程**并行度降到 `--no-file-parallelism --maxWorkers=2`（不改变断言强度），
> 复跑全绿。若要作为权威门禁，建议单独跑 `npx vitest run tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts`。

### 4.3 真 Chromium（playwright）

```
$ npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts
6 passed / 2 failed (2.5m)
```
- **10 passed / 0 failed（2.9m）**：V4 / V5 / V6 / V7-b / **V13-2（新）** / **R6-b** / **V8（同轮 A/B 差分，连续 3 轮）** / V8-b / V9 / V9-b 全绿
- V6 的实测分流证据（`aggregate-summary.json`）：尾巴帧 **253** = 单行 **58**（mask `linear-gradient(...)`）+ 2 行 **65** + 3 行 **130**（多行均 `mask-image: none`）；scrollHeight 回撤点 **0**

逐帧聚合：`receipts/e2e/aggregate-summary.json`（900 帧、consoleErrors=[]、scrollHeight 回撤 0 处）
V8 空闲基线：`receipts/e2e/idle-baseline.json`
**V8 同轮 A/B 差分 3 轮实测表（测试自动落盘，含污染标记）**：`receipts/e2e/v8-relative-runs.json`
**旧判据翻车快照（对照证据）**：`receipts/e2e/v8-old-criterion-evidence.json`

## 5. 逐条状态（红相 vs 现行）

> 说明：实现已在 v5 期间落地，因此**现行全绿**。红相列记录的是**判据冻结时**的实测状态（Lead 已在 `test(wave-pds-3): freeze acceptance tests` 锚定）。
> 「验收价值」不体现在"现在是绿"，而体现在：① 判据是否真的能抓缺陷（见 §5.3 的 V11-0 自检）；② 红相锚定后的判据不允许被实现方改动（V10-1 哈希锚）。

### 5.1 总表

| 组 | 用例数 | 红相（实现前） | 现行（v5 实现后） |
|---|---|---|---|
| V1（尾巴 DOM 协议） | 8 | 6 红 / 2 绿（V1-6/7 基线绿） | 8 绿 |
| V2（开关 off） | 6 | 2 红 / 4 绿 | 6 绿 |
| V3（承载节点身份） | 3 | 1 红（V3-3）/ 2 绿 | 3 绿 |
| V7（jsdom 降级行为+规则） | 7 | 5 红 / 2 绿 | 7 绿 |
| V11（v5 新增：尾巴切分语义） | 9 | —（v5 前无此组） | 9 绿（含 v7 的纯段落尾巴 V11-7） |
| V12（v7 新增：资格门） | 13 | —（v7 前无此组） | 13 绿（**首跑 V12-3 红命中 N3**；v9 补齐 rule 5b/6/7/8 夹具） |
| V13（**v8 新增：纯文本快路径尾巴**） | 5 | —（v8 前无此组） | 5 绿（首跑 4 红/1 绿，实现后转绿） |
| V14（**v9 新增：资格门完备性**） | 14 | —（v9 前无此组） | 14 绿（**首跑 6 红/7 绿**，红的正是 v7 黑名单漏网的 6 类块级构造；新增 V14-0b 同源自检） |
| R1–R8（§3/§4/§5 附加护栏） | 8 | 7 红 / R8 绿 | 8 绿 |
| V10（基线冻结） | 3 | 3 绿（基线绿） | 3 绿 |
| **jsdom 小计** | **76** | 21 红 / 14 绿（35 例时点） | **75 绿（EXIT=0）** |
| V4 / V5 / V9（浏览器基线） | 3 | 绿 | 绿 |
| V6（高度/遮罩，v5 改写） | 1 | 红（tailFrames=0） | 绿 |
| V7-b（Chromium computed-style） | 1 | 红（属性轨 3 节点仍有动画 + 尾巴无遮罩） | 绿 |
| V8 / V8-b / V9-b | 3 | V8 绿（当时用相对基线）/ V8-b 采样 / 凭据 | 绿（**v6 同轮 A/B：3 轮 B−A = −0.80 / −2.60 / −1.30ms，两段 LongTask=0**） |
| **浏览器小计** | **10** | 6 绿 / 2 红 | **10 绿（10 passed）** |

### 5.2 现行 jsdom 全绿（44 例）分组

- V1-1 数量 ≤1 / 未闭合时 =1；V1-2 无第二尾巴；V1-3 行内级声明；V1-4 注入仓库 CSS 后 computed display；**V1-5 `data-tail-len` == `splitMarkdownBlocks` 源尾字符数**；V1-6 块闭合帧 =0；V1-7 不吞内容；V1-8 缺省开启
- V2-1..V2-6（off ⇒ 0 且正文完整、两态文本一致、身份不变、不挂载隐藏节点）
- V3-1/V3-2 身份不变；V3-3 尾巴出现前后不重建
- V7-1/V7-2 双轨降级尾巴不渲染（含对照组）；V7-3 全文可见；V7-4 primitives 被引用；V7-5 双轨覆盖；V7-6 `mask-image:none`；V7-7 无半降级
- **V11-0 判据自检**；**V11-1 表格 / V11-2 setext / V11-3 跨行加粗 / V11-4 嵌套列表 / V11-5 多行段落 / V11-6 栅栏闭合**；V11-6a 代码尾内嵌；V11-6b 提交帧结构等价
- R1 tick 33；R2 双锁（tick 33 + charsPerTick 4）；R3 reasoning 140ms+位移；R4 blur(2px)+220ms；R5 `data-pds-tool-row`+入场动画；R6 rise/fall；R7 静态 mask 非 animation；R8 无定时器泄漏
- V10-1 哈希锚 / V10-2 冻结基线子进程 / V10-3 既有全量单测子进程

### 5.3 判据自检（V11-0）—— 本轮最重要的新增

V11-0 用**同一套仓内渲染器**复现 v5 审查描述的缺陷实现（`tail` 按最后一个 `\n` 切分，两段分别渲染），
再对本组 6 个检测器逐一运行，**要求每个检测器都必须报错**（实测 6/6 报警）。复现出的缺陷形态：

| 夹具 | 缺陷实现的产物（V11-0 实测） | 检测器报出的违规 |
|---|---|---|
| 表格 | `<table>` 只有 `<thead>`（tbody 无行） + `<p>\| 1 \| 2</p>` | 裸竖线 + 空表 + 表格缺单元格 |
| setext | `<p>这是一个 setext 标题</p>` + `<hr>` | 出现 `<hr>` + 标题不是 h2 + 标题被当段落 |
| 跨行加粗 | `<p>这是**加粗<br>**</p>` + `<p>文字<strong>继续</strong></p>` | 字面 `**` + 未形成「加粗文字」strong |
| 嵌套列表 | 两个顶层 `<ul>`（第二个在尾巴里） | 顶层列表数 ≠1 + 嵌套项缺失 |
| 多行段落 | `<p>同一段的第一行</p>` + `<p>第二行仍在同一段</p>` | 两行不在同一 `<p>` + 段落数 ≠2 |
| 栅栏闭合 | 两个 `<pre>`（第二个为空） | `<pre>` 数量=2 |

**这条自检把"实现修好了所以绿"与"判据本身不灵所以绿"分开了** —— 也正是 v1–v4 缺失的那一环。

### 5.4 V12（资格门）当前状态

| 用例 | 构造 | 当前 | 说明 |
|---|---|---|---|
| V12-1 | 未闭 ```ts（围栏不在尾巴首行，N1） | 🟢 | 无尾巴 + 围栏外无字面 ``` + 文本与全文渲染对齐 |
| V12-2 | 未闭 ~~~ | 🟢 | 同上 |
| V12-3 | 未闭 ```md（N3） | 🟢（**首跑为红，已修复**） | 首跑 23:49 红：`实际="标题正文一行"` vs `期望="#标题正文一行"`（N3：opener 被 unwrap、`#` 丢失）；实现侧修复后转绿 |
| V12-4 | 未闭围栏内含 ```js（N4） | 🟢 | ```js 作为代码内容保留、后续行不丢 |
| V12-5 | 缩进代码块 | 🟢 | 无尾巴且两行都在 |
| V12-6 | loose list（N5） | 🟢 | 无尾巴、A/B 两项都在 |
| V12-7 | 引用块 | 🟢 | 无尾巴且两行都在 |
| V12-8 | 表格行 | 🟢 | 无尾巴、四个单元格文本都在 |
| V12-0 | 纯段落（控制组） | 🟢 | 必须发尾巴（防"一律不发尾巴"作弊） |

> 结论（给 Lead）：v7 资格门已全部落地，**V12 现 9/9 绿**；其中 N3 是本组首跑抓到的真实缺陷（随后被修复），
> 这是 V12 判别力的直接证据（不是碰巧为绿）。

### 5.5 V14（资格门完备性）首跑红相 → 现行状态

| 用例 | 构造 | 首跑 | 现行 | 首跑失败原文（节选） |
|---|---|---|---|---|
| V14-1 | tab 缩进代码 | 🔴 | 🟢 | `<span data-stream-tail …><pre class="code-block-cream"><code>const z = 1;` |
| V14-2 | 四空格缩进 | 🟢 | 🟢 | （v7 黑名单已拦） |
| V14-3 | `***` 水平线 | 🔴 | 🟢 | `<span …><p>正文一行</p>\n<hr>` |
| V14-4 | `___` 水平线 | 🔴 | 🟢 | 同上 |
| V14-5 | 单 `-` setext | 🔴 | 🟢 | `<span …><h2>标题一行</h2>` |
| V14-6 | `# 标题` | 🔴 | 🟢 | `<span …><h1>标题一行</h1>` |
| V14-7 | `## 标题` | 🔴 | 🟢 | `<span …><h2>标题一行</h2>` |
| V14-8 | 四反引号栅栏 | 🟢 | 🟢 | （v7 栅栏规则已拦） |
| V14-9 | CRLF 列表 | 🟢 | 🟢 | （v7 列表规则已拦） |
| V14-10 | `$$`（分歧项，见 §7.3） | 🟢 | 🟢 | 只断言"尾巴内无块级标签 + 文本无损" |
| V14-11 | HTML 块 | 🟢 | 🟢 | （v7 HTML 规则已拦） |
| V14-12 | HTML 注释 | 🟢 | 🟢 | （渲染为空 ⇒ 无尾巴） |
| V14-0 | 纯段落（反向自检） | 🟢 | 🟢 | 必须发尾巴且尾巴内无块级标签 |

**首跑 6 红 / 7 绿**；实现侧按 v9 改为 render-then-verify 后 **13/13 全绿**。
V14 的②（尾巴 span 内不得有块级标签）是真正抓到上述 6 类的判据 —— 若只断言 `tails=0`，v7 漏网时同样会红，
但"把 `<pre>` 塞进尾巴但数量仍为 1"的变体只断言数量根本抓不到。

## 6. V8（契约 v6）：同轮 A/B 差分 —— 判定与实测表

### 6.0 占位流夹具的 v7 配套调整（重要，否则 V6 会退化成空集）

v7 之后尾巴只对**纯段落**发放，因此：
- 「多行尾」只能来自**纯段落折行** ⇒ 若合成流里段落都很短，多行分支会 0 样本（实测 V6 因此红过一次）；
- 「单行尾」需要**足够短的段落** ⇒ 长段落起步就折成 2 行，单行分支同样会 0 样本（实测再红一次）。

已把占位流夹具调整为「**1 段超长普通段落 + 3 段短普通段落**」（各段以空行分隔、单次 delta 仍 ≤50 字符），
于是两个分支都有真实样本。最新分布（`aggregate-summary.json` / 逐帧 `samples.json`）：

| 尾巴视觉行数 | 帧数 | 实测 `mask-image` |
|---|---|---|
| 1 行 | 58 | `linear-gradient(...)`（遮罩启用） |
| 2 行 | 65 | `none`（遮罩关闭） |
| 3 行 | 130 | `none`（遮罩关闭） |

> 这也是"判据自检"意识的一部分：任何"某一分支 0 样本"都会被 V6 的自检断言拦下（>10 帧），不会静默通过。

### 6.1 判据（每次运行由测试自己判定并落盘）

| 项 | 阈值 |
|---|---|
| ① 本动效自身开销 | `p95(B) − p95(A) ≤ 4ms` |
| ② 长任务 | A、B **两段** `Long Task(>50ms)` 均为 0 |
| ③ 框架合理性自检 | `p95(A) − p95(idle) ≤ 6ms` |
| ④ 稳定性 | **连续 3 轮**全满足（单轮不挑最好） |
| ⑤ 污染剔除 | 某轮 `p95_idle` 偏离记录基线 **16.90ms** 超 **1.5ms** ⇒ 标 `[LOAD-CONTAMINATED]` 重测；被剔除轮次同样落盘 |
| ⑥ **瞬时尖峰（v11）** | **仅** `B−A > 4ms` 失败且 `A−idle ≤ 6ms`、两段 Long Task=0 ⇒ 标 `[SPIKE]`，不计入 3 轮干净轮、不直接 FAIL，自动补测；总尝试上限 **8 轮**；`[SPIKE]` 轮必须落盘 |

**A/B 分流自检**（防"两段其实同一种行为"的假绿）：A 段 `tailFrames` 必须为 **0**，B 段必须 **>100**（实测 A=0 / B=213~216）。

### 6.2 实测（最新一次完整复跑，`receipts/e2e/v8-relative-runs.json` 原样摘录）

| 轮 | 开始时间 (UTC) | idle p95（漂移） | A p95（tail off） | B p95（tail on） | **B−A** | A−idle | LongTask A/B | tailFrames A/B | 判定 |
|---|---|---|---|---|---|---|---|---|---|
| #1 | 2026-11-02T17:10:12.548Z | 16.90（0.00） | 17.20 | 17.30 | **+0.10** | 0.30 | 0 / 0 | 0 / 253 | pass |
| #2 | 2026-11-02T17:10:52.214Z | 17.00（0.10） | 17.10 | 17.20 | **+0.10** | 0.10 | 0 / 0 | 0 / 236 | pass |
| #3 | 2026-11-02T17:11:32.893Z | 17.00（0.10） | 17.10 | 17.30 | **+0.20** | 0.10 | 0 / 0 | 0 / 253 | pass |

- **最终 3 轮干净数据：B−A = +0.10 / +0.10 / +0.20 ms**（均 ≤4ms）；**A−idle = 0.30 / 0.10 / 0.10 ms**（均 ≤6ms 框架自检）；
  **两段 Long Task 全 0**；A 段 tailFrames=0、B 段 236~253（A/B 分流自检通过）；**0 轮 `[LOAD-CONTAMINATED]`、0 轮 `[SPIKE]`**（maxRounds=8 未用满）。
- **v8/v9 实现后 B−A 由负转正**：v7 时期是 −2.60 / −3.60 / −1.50，v9 之后稳定为 **+0.5~+2.6ms**
  （v9 的 render-then-verify 每帧多一次 `renderPartialMarkdown` 做资格判定）——这正是 A/B 差分该体现的"动效自身开销"。
- **瞬时尖峰已按 v11 处理**：此前整包复跑出现过单轮 B−A = **+7.50ms**（idle 与 A 都正常）。现在这种形态会被判
  `[SPIKE]`（阈值仍 4ms、3 轮干净仍是硬要求，只是该轮不计入干净轮并自动补测，上限 8 轮），且**必须落盘** ——
  既不再误判实现，也不允许"挑最好的一次"。**容许重测 ≠ 放宽阈值**。
- 本轮 idle 漂移 0.00/0.10/0.00ms（机器 load average ≈ 10.75 / 16 核 —— 正是旧判据会翻车的负载区间，差分法仍稳定）。
- 另一次同实现复跑（load avg ≈ 7.6）：B−A = −2.00 / +1.80 / −1.70，同样 3/3 pass，LongTask 0。

### 6.3 旧判据翻车快照（留档，供对照）

`receipts/e2e/v8-old-criterion-evidence.json`（旧判据 `p95_stream ≤ p95_idle + 6ms`）：

| 轮 | idle p95（相对基线漂移） | stream p95 | Δ | LongTask | 旧判据判定 |
|---|---|---|---|---|---|
| #1 | 18.90（**+2.0**，已污染） | 22.00 | 3.10 | 0 | pass（不该 pass） |
| #2 | 16.90（0.0，干净） | 19.90 | 3.00 | **1** | fail（与实现无关） |
| #3 | 18.80（**+1.9**，已污染） | 20.70 | 1.90 | 0 | pass |

⇒ 旧判据在污染轮上会假绿、在干净轮上会因偶发 Long Task 假红；v6 的 A/B 差分 + 污染标记把这两类噪声都挡在门外。

### 6.4 实现落地后的验收动作

```bash
npm run build
npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts -g "同轮 A/B"
```
3 轮表会自动覆盖 `receipts/e2e/v8-relative-runs.json`（含 `[LOAD-CONTAMINATED]` 标记轮），交付说明引用最新那份。

## 7. 实现者注意 / 仍需 Lead 确认的事项

1. **V7-b 的「媒体轨道已绿、属性轨道全红」信息量最大**：既有 `@media (prefers-reduced-motion: reduce)` 已覆盖
   `.conversation-tool-panel` / `.assistant-streaming-fade`（实测 media 轨 0 违规）；但**没有任何**
   `[data-pds-reduce-motion="true"]` 规则（实测 3 个节点仍为 `item-smooth-appear-*` / `assistant-stream-fade-in`），
   且尾巴 `mask-image` 在正常态实测为 `none`（§3.1 未实现）。只需补「属性轨道 + 尾巴遮罩 + import primitives」，媒体轨道别重复造。
2. **`data-pds-tool-row` 是新增必需属性**：R5 先自检锚点、再查该行的入场动画类，实现时一并落地。
3. **仍待 Lead 确认（低风险）**：`@media` 降级块与属性降级块是否都必须覆盖 §3.2/§3.3/§3.4 的全部三类动效。
   当前 V7-5 只锁三类锚点（`data-stream-tail` / `assistant-streaming-fade` / `transition-item-enter`），
   状态行 rise/fall 的降级由 R6 + V7-b 的节点集合覆盖；若你要求"状态行也必须进双轨降级断言"，请示下，我补一条即可。

### 7.5 已登记的两个"真值/取舍"说明（Lead 已接受）

1. **V12 的真值来自 `renderMarkdown(全文)`**：V12 的"无行丢失"判据是把流式渲染的可见文本与
   `renderMarkdown(source)` 的文本做**逐字对比**。因此**若 `renderMarkdown` 自身漂移，V12 会跟着漂**
   （判据与真值同源）。Lead 已确认接受，并会在 ADR 中同步登记；若将来需要硬锚，可给 `renderMarkdown`
   加黄金快照。
2. **HTML 块 / `$$` 的取舍已变更**：v5–v8 时期我以"项目不渲染这两类，断言没渲染的东西=假断言"为由不造用例；
   **v9 起判据改为"看渲染结果"**，因此这两类**必须**造（`V14-11` HTML 块、`V14-12` HTML 注释、`V14-10` `$$`，
   以及 `V12-10` / `V12-12`）。旧理由随之作废，已在 §0 的 v9 段落中记录。

### 7.0 V10-3 子进程并行度（Lead 已批准）

`V10-3`（"全量套件里再跑一遍全量套件"）已把**子进程**并行度降为 `--no-file-parallelism --maxWorkers=2`：
默认并行度下外部 worker 抢 CPU，会把仓库里 5s 超时的 `check-secrets` 全仓扫描挤到超时，制造与实现无关的假红（实测 1 次）。
**断言强度未变**（仍是 exit 0 + 通过用例数自检）。**权威门禁建议单独跑该文件**：
`npx vitest run tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts`。

### 7.2 纯无 markup 正文的尾巴路径（**v8 已裁定并实现，V13 覆盖**）

流式内容若**完全不含 markdown 标记**（例：`已定稿段落。\n\n这是一段普通正文，正在逐字输出`），
`HomeWorkspace.shouldUseMarkdownAssistantRendering` 会走 `WorkspaceTurnItem` 的**纯文本快路径**，
根本不进 `MarkdownRenderer` ⇒ **不会渲染尾巴**（实测 `[data-stream-tail]` = 0）。

- 契约 v7 说"正文流式绝大多数内容是普通段落，故观感收益基本保留"——若按字面，这类正文也应有尾巴；
- 但纯文本快路径是 wave-59 的既有优化，本波 §7 写域未包含 `HomeWorkspace` 的路径选择逻辑改动。
- **v8 裁定**：纯文本快路径**也必须发尾巴**（同一标记/开关/降级），切分 = 已释放文本中最后一个空行之后。
  Executor 已在 `WorkspaceTurnItem.vue` 实现；测试侧新增 `V13`（5 例 jsdom + V13-2 Chromium）覆盖。
  **V13-4 用 `toBe` 全等**断言「stable + tail == 已释放文本」，专门抓吞字/重字。

### 7.1 V11 可能"实现改了但判据仍抓不住"的地方（自评，按风险降序）

1. **宽度钳制 / 按标点前推这两条分支（最高风险）**：jsdom `clientWidth === 0` ⇒ 实现走「40 字兜底」，
   V11 的尾巴（≤31 字符）**永远不会触发**「单行宽度钳制」与「按标点前推定稿边界」。
   若缺陷只在这两条分支上（例如把表格行按标点切成残片），V11 全绿也说明不了问题。
   兜底：V4（逐帧 scrollHeight 不回撤）+ V6（单行/多行分流）在 Chromium 侧间接覆盖；但**都是间接的**，见 §8 缺口登记。
2. **V11-0 自检只证明"能抓『按最后一个 `\n` 切分』这一种缺陷"**：若后续缺陷换成"按标点前推"或"按宽度切"，
   V11-0 不能替它们背书。补法：为每种已知缺陷形态各写一条 emulation（成本低，建议下轮补）。
3. **V11-2 setext 只覆盖 `---`（h2）**：`===`（h1）未测；且检测器按固定标题文本定位，若实现把标题文本重排（加 `<br>`）仍可能通过。
4. **V11-3 只覆盖"跨 1 个换行的 `**`"**：未覆盖 `__` 变体、嵌套强调 `***`、以及强调跨**块边界**（那属 markdown 语义边界，marked 自身行为，不宜作为本波判据）。
5. **V11-4 只覆盖 2 层无序列表**：未覆盖有序列表嵌套、3 层深、loose list（项内含段落）。
6. **V11-5 把"段落数 == 2"写死**：夹具变化会误报；且未覆盖"段内出现空行（段落已闭合）"的边界帧。
7. **V11-6 只覆盖 ``` + info string（`ts`）**：未覆盖 `~~~` 围栏、缩进栅栏、语言缺失、围栏内含 ``` 的情形。
8. **共性盲区：不校验"尾巴与已定稿区的文本连续性"**（重复渲染一个字 / 丢一个字都不会被 V11 抓到）。
   之所以没加：`renderPartialMarkdown` 与全量 `renderMarkdown` 在未闭合块上本就不保证逐字相同，硬造等式会变成假红。
   兜底：V1-5（`data-tail-len` == 源尾字符数）+ V11 的语义断言 + V4（无高度回撤）三者组合能覆盖大部分真实缺陷，但不能覆盖"文本丢了但结构没变"。

### 7.3 契约张力：`$$` 数学围栏（**v11 已结案：选项 A**）

v9 的资格判定是「render-then-verify」，而实测 `renderPartialMarkdown("$$\nx = y\n$$")` 产出
`<p>$$<br>x = y<br>$$</p>` —— **没有任何块级元素** ⇒ 按 v9 的语义它是**合格**的纯段落尾巴。
但 v9 的 V14 清单把 `$$` 列为「必须 tails = 0」。两者不能同时成立。

- 现行实现（Executor v9 版）按 render-verify ⇒ 对 `$$` **会发尾巴**，尾巴内只有 `<p>`/`<br>`（无块级）。
- 测试侧处置：`V14-10` 与 `V12-12` 对 `$$` 只断言 **②（尾巴内无块级标签）** + **文本无丢失**，
  **不硬断言 tails=0**，把分歧显式登记，不替契约做选择。
- **v11 裁定 = 选项 A**：V14 清单里 `$$` 那条**作废**；`V14-10` / `V12-12` 保持"只断言尾巴内无块级标签 + 文本无损"。
  理由（Lead）：判据与渲染器同源，源文本形态不参与判定；将来接入数学扩展后 `$$` 会渲染成块级元素，
  判据**自动**把它发回 stable，无需改代码。

### 7.4 结构性限制：loose list 不可能整体落在未闭合块里（**v11 已结案：接受为结构约束**）

N5 的形态是"松散列表被切成两块"。但 `splitMarkdownBlocks` **遇空行必切块** ⇒ 任何 `- A\n\n- B`
的空行都会成为块边界，因此**一个 loose list 永远不可能作为一个未闭合块存在**。
→ `V12-6` 覆盖的是 N5 的**边界形态**（已定稿区一个 `<ul>`、尾巴另一个列表项），断言"无尾巴 + 尾巴内无块级标签 + 文本无丢失"；
**不**断言"最终只有一个 `<ul>`"，因为 HEAD（本波之前）行为同样是两块 —— v9 也写明"漂移应回到 HEAD 水平"。
**v11 裁定**：这不是缺陷，是 `splitMarkdownBlocks` 的结构约束；`2 ul → 1 ul` 与 HEAD 同构、接受（记入 ADR）。
验收要求降到"不丢行、不新增 HEAD 之外的结构抖动"——`V12-6` 现状即满足。

## 8. 环境限制如实标注

| 项 | 结论 | 依据 |
|---|---|---|
| Chromium 可用性 | ✅（Playwright 1.60 / chromium headless） | 实跑 5 passed / 3 failed |
| jsdom `animation-name` / `mask-image` | ❌ 不可读 ⇒ 字面判据移 V7-b（Chromium） | 已按 v2 裁定 ③ 处理 |
| jsdom `getComputedStyle().display` | ⚠️ 仅对文档内已声明规则可见 | V1-4 显式注入仓库 CSS 后读取 |
| jsdom 布局（scrollHeight/rect） | ❌ 无布局 | V4/V5/V6 一律 Chromium |
| Playwright worker 重启导致产物散落 | ✅ 已修：证据目录改为固定 `$TMPDIR/dsh-pds-wave3-artifacts` | 本轮实测（3 个目录 → 1 个） |
| 帧预算噪声 | ⚠️ 本机空闲 p95 16.8–17.4ms 抖动；多 Agent 并发时 Δ 波动更大 | §5 表格 |

### 8.1 覆盖缺口登记（v7 修正版）

**已撤销的欠账（重要）**：v5 时期此处登记过「单行宽度钳制」与「按标点前推定稿边界」两条分支在 jsdom 零覆盖。
**该说法已作废** —— 经代码核对（`MarkdownRenderer.vue`）与审查者实测：当前实现**已不存在**这两条分支：

- 尾巴 = `splitMarkdownBlocks` 的整个未闭合块，**不再切行、不再按标点前推**；
- 容器宽度（`ResizeObserver` + canvas `measureText`）**只用于**判断"尾巴是否实际单行"，从而决定是否启用遮罩
  （`data-pds-tail-multiline`），**不改变内容切分** ⇒ 宽度缺失（jsdom）只会让遮罩判定退化为"按单行处理"，不会造成语义缺口。

因此本条不再是覆盖缺口；与之相关、仍然有效的覆盖由 V6（Chromium：单行尾高度 / 多行尾 `mask-image: none`）与 V4（逐帧无回撤）承担。

| 缺口 | 原因 | 现状 | 兜底 |
|---|---|---|---|
| V11-0 只对「按最后一个 `\n` 切分」这一种缺陷形态做 emulation | 见 §7.1-2 | 已知 | 下轮为每种缺陷形态补 emulation |
| 纯**无 markup** 的正文尾巴路径（见 §7.2） | 走 `WorkspaceTurnItem` 的纯文本快路径，不经 `MarkdownRenderer` | 待契约明确 | V11-7 只覆盖"含行内标记的纯段落" |

**没有伪造任何通过**：所有绿都是实跑结果；红相历史见 `receipts/jsdom-red-phase.log` 与 git 锚定提交。

---

## 9. 复现命令

```bash
# jsdom（V1/V2/V3/V7/V10 + R1–R8）
npx vitest run tests/acceptance/pds-streaming/ --reporter=verbose

# 真浏览器（V4/V5/V6/V7-b/V8/V8-b/V9）—— 需先 npm run build
npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts

# V8 / V8-b 单跑（同轮 A/B 差分 3 轮 + 空闲基线留档）
npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts -g "同轮 A/B|空闲基线帧预算"

# 调试参数（**默认值即契约值**；偏离时断言消息会标注"⚠️ 非契约值：调试参数"）
PDS_E2E_CHUNK_MS=200 PDS_E2E_STREAM_FRAMES=900 PDS_E2E_IDLE_FRAMES=320 \
  npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts -g "同轮 A/B"

# V11 单跑（尾巴切分语义 + 判据自检）
npx vitest run tests/acceptance/pds-streaming/v11-tail-split-semantics.spec.ts --reporter=verbose

# 全量
npx vitest run
```
