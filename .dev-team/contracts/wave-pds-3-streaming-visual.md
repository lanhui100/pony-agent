# 契约：wave-pds-3 — 流式视觉层接入（PDS-SM 落地）

> 冻结方：Lead。实施者（Executor）与测试者（Test Agent）均只读本契约。
> 分级：**B 级**（跨 4 个 src 模块 + 修订既有 ADR 部分决定 + 冻结门禁迁移 + 需真浏览器帧预算证据）。
> 依据：`docs/decisions/0057`、`0058`、`0056`。

## 修订记录 v3（累积：v2 的 9 条 + v3 的 2 条）

| # | 缺陷 | 裁定 |
|---|---|---|
| ① | §4 的 `charsPerTick 4→2` 与 V10（14 例冻结基线全绿）**互斥**：基线与 parity 黄金表都硬编码每 tick 4 字 | **撤销 charsPerTick 改动**。`DEFAULT_REVEAL_BUDGET` 保持 `4`，`logic/budget.ts` 移出写域；只改 `HomeWorkspace` 的 tick `60→33`。理由见 §4 |
| ② | V8 阈值（p95 ≤16.7ms）本机物理不可达：空闲 p95=16.90ms | **重定义 V8**为「相对空闲基线」判据 + Long Task 计数；绝对阈值作废（§5） |
| ③ | V7 在 jsdom 无法断言 `animation-name`/媒体查询 | **接受**行为证据写法；字面 computed-style 判据移入 Chromium，新增 **V7-b** |
| ④ | `data-reduce-motion` 与 primitives.css 的 `data-pds-reduce-motion` 命名分裂 | **统一为 `data-pds-reduce-motion`**（§2/§3.5） |
| ⑤ | tail 数量在块闭合帧/空正文首帧未定义 | 定义为「**恒 ≤1**；有未闭合尾巴文本时 =1；否则 =0」（§1） |
| ⑥ | 尾巴文本语义缺接口 | 定义 `data-tail-len === 尾文本长度`，尾为源文本的**严格后缀**、不做 trim（§1） |
| ⑦ | 开关热切换未定义 | 明确为 **mount-only**，热切换不在本波范围（§2） |
| ⑧ | 工具行无稳定锚点 | 新增必需属性 **`data-pds-tool-row`**（§1） |
| ⑨ | 是否保留冻结文件 SHA 锚 | **保留**。本波不改任何既有测试，故锚点无需迁移 |

## 0. 契约演进登记（本波推翻什么、不推翻什么）

| 既有决定 | 本波处置 |
|---|---|
| `D-59-1`：未渲染尾巴**不以任何节点**呈现 | **修订**：允许**恰好一个** inline 尾巴节点，必须打 `data-stream-tail`；仍禁止块级尾巴、禁止多个 |
| `F3-1`：三类尾巴/字符节点数量必须为 0 | **修订**：三类继续为 0；新增「除 `[data-stream-tail]` 外，内容区不得有第二个尾巴节点」 |
| `D-59-2`：移除逐字符淡入 | **保持**。本波**不建字符节点**，reveal 由「tail 级遮罩」承担（§3.1） |
| `F3-2b` / `F3-3a` / `F3-4`：`.markdown-body` 身份不变 | **保持**，改为更强断言（V3） |

**为什么不是逐字 span**：那需要恢复 D-59-2，与 0057 反模式 §1 冲突，且每 tick O(N) 重建必然闪跳。本波用**遮罩在尾巴右缘做渐变**达到"新字符从透明区浮现"的等效观感，零字符节点。

## 1. DOM 协议（唯一接口）

```
<div class="markdown-body" v-html="stableHtml" />          ← 身份必须稳定，永不重建
<span data-stream-tail data-tail-len="N">tailHtml</span>   ← 唯一尾巴；display: inline
```

- 尾巴**必须 inline**（`display` ∈ {inline, inline-block}），与最后一段文本同流，不得产生新行。
- **数量规则**：内容区 `[data-stream-tail]` 数量**恒 ≤ 1**；存在未闭合尾巴文本时 = 1；无尾巴时 = 0。
- **文本语义**：尾巴 = 源文本的**严格后缀**（自最后一个已闭合块边界之后），**不 trim、不重排**；`data-tail-len` 必须等于尾巴文本的字符数。
- 尾巴只承载**当前未定稿块**；该块闭合后其内容移入 `stableHtml`。
- 尾巴长度上限：超过 **1 行**（容器实测）时按标点（`。！？；，\n`）向前推进定稿边界。
- 关闭开关时：**不渲染** `[data-stream-tail]`，全部走 `stableHtml`。
- 工具行必须带 **`data-pds-tool-row`**（供入场动效与验收定位）。

## 2. 回退开关

- `localStorage["pony-agent.stream-render.tail-reveal"]`：缺省 = **on**；`"off"` = 关闭。
- **读取时机：mount-only**（挂载时读一次并缓存）。运行期热切换不在本波范围。
- 系统 `prefers-reduced-motion: reduce` 或祖先 `data-pds-reduce-motion="true"` ⇒ **强制关闭**（不渲染尾巴 + 全部动画 `animation:none`）。

## 3. 动效规格

### 3.1 尾巴浮现（核心）
```css
[data-stream-tail] {
  --pds-tail-fade: 1.5em;
  display: inline;
  mask-image: linear-gradient(to right, #000 calc(100% - var(--pds-tail-fade)), transparent 100%);
}
```
新字符自右端透明区进入，随吐字左移渐成实色 ⇒ 连续浮现。**不是 animation**，内容推进时不会重启，故与 RC2 闪跳有本质区别。

### 3.2 reasoning 尾巴（`.assistant-streaming-fade`）
`opacity 0→1 / 350ms` ⇒ `opacity 0→1 + translateY(2px)→0 / 140ms`，`ease-out`。

### 3.3 工具行 / 工具面板入场（一次性节点）
`item-smooth-appear` 由 `opacity+translateY(4px) / 200ms` ⇒ 加 `filter: blur(2px)→0`，`220ms`。工具行（`[data-pds-tool-row]`）当前无任何入场动画，需补同类规则。

### 3.4 状态行交接（`AgentRuntimeStatusBar` label）
原位替换时加 `rise`（`opacity 0→1 + translateY(6px)→0`，240ms）；退场 `fall`（180ms）。保持 label 原位，不重建节点。

### 3.5 降级（双轨，强制）
`@media (prefers-reduced-motion: reduce)` 与 `[data-pds-reduce-motion="true"]`：上述全部 `animation: none`、`transition: none`，`[data-stream-tail] { mask-image: none; }`。

## 4. 节奏

- `HomeWorkspace.STREAMING_PRESENTATION_TICK_MS`：`60` → **`33`**（30Hz）。
- **`charsPerTick` 保持 `4`**（`DEFAULT_REVEAL_BUDGET` 不动）。
- 理由：冻结基线 14 例与 parity 黄金表都以「每 tick 4 字」为真值；改它必须迁移冻结断言，收益却由 §3.1 的遮罩承担大半。"平滑"的来源是**更高的 tick 频率 + 新字符落在渐隐区**，而不再是靠缩小步长。
- 若 V8 不达标：允许回调到 `50ms`；须在交付说明写明实测值与最终取值。

## 5. NFR（本波唯一硬指标，v2 重定义）

| 指标 | 阈值 | 测法 |
|---|---|---|
| **V8 流式期帧时间** | `p95_stream − p95_idle ≤ 6ms` 且 **Long Task（>50ms）= 0**，且**连续 3 次运行全部满足** | 同一页面同一浏览器：先采空闲基线 ≥300 帧，再采流式 ≥800 帧；占位流参数见下 |
| 新增 DOM 节点 | ≤ 1（tail） | V1 |
| 新增定时器/监听器泄漏 | 0 | 卸载后计数归零 |
| Console Error | 0 | V9 |

> **v3 补：占位流参数（V8 可复现的前提）**：delta 间隔 **120ms**、单次 chunk **≤ 50 字符**、空闲基线 ≥300 帧、流式 ≥800 帧。harness 以这些值作默认；环境变量仅作调试覆盖，断言失败消息必须回显实际参数。
> **v3 补：为什么是 6ms + 连续 3 次**：p95 在 60Hz 上受 rAF 调度噪声支配（实测同一实现 7 次运行 Δ = 0.0–5.0ms，3 fail / 4 pass）。单次运行既会假红也会假绿；「连续 3 次全过」才是可判定的稳定性表述。这是收紧而非放宽——**禁止只报 3 次里最好的一次**。

> **绝对阈值 16.7ms 作废**：60Hz 屏上空闲 rAF 间隔本身 p95≈16.9ms，绝对阈值会把"不做事"判红（Test Agent 实测证据）。相对基线才是可判定的物理量。
> 本波无外部网络调用 / 无后台任务 / 无 LLM 调用 ⇒ NFR 基准表的超时、重试、并发锁、结构化日志四项 **N-A**（纯渲染层变更，不引入出网或跨进程路径）。

## 6. 验收（V1–V10，v2 冻结；Test Agent 独占写权）

| ID | 断言 | 环境 |
|---|---|---|
| V1 | `[data-stream-tail]` 数量恒 ≤1；`data-tail-len` == 尾文本长度；tail 非块级；三类旧尾巴/字符节点 = 0 | jsdom |
| V2 | 开关 `"off"` ⇒ tail 数量恒 0，且 14 例基线行为不变 | jsdom |
| V3 | `.markdown-body` 承载节点跨内容增长与 streaming→done 身份不变 | jsdom |
| V4 | 逐帧 `scrollHeight` 不回撤 >1px | Chromium |
| V5 | 正文首帧非空后无空白帧 | Chromium |
| V6 | 流式全程 tail 高度 ≤ 行高 × 1.2 | Chromium |
| V7 | reduced-motion：tail 不渲染 + 行为证据 + 双轨规则存在 | jsdom |
| **V7-b** | Chromium 下 computed `animation-name === "none"` 且 `mask-image: none` | Chromium |
| V8 | 见 §5（相对基线 + Long Task=0） | Chromium |
| V9 | Console Error = 0 | Chromium |
| V10 | 14 例基线 + wave59(23) 全绿（SHA 锚定） | vitest 子进程 |

## 7. 写域（WIP=1 串行）

| 角色 | write_scopes |
|---|---|
| Test Agent | `tests/acceptance/pds-streaming/` |
| Executor | `src/components/MarkdownRenderer.vue`、`src/components/chat/WorkspaceTurnItem.vue`、`src/components/HomeWorkspace.vue`、`src/components/chat/AgentRuntimeStatusBar.vue`、`src/styles.css`、`src/lib/useStreamingPresentationState.ts`（**仅限** `assistantDisplayedReasoningFadeStyle` 的 `animationDuration` 350ms→140ms 一行，理由：该时长是内联样式而非 CSS，R3 只能从这取；其余一律不许动） |
| Lead | `.dev-team/contracts/`、`docs/decisions/`、`.dev-team/state.json`、红相锚定 commit |

**禁止**：Executor 改 `tests/**`；Test Agent 改 `src/**`；任何人改 `tests/acceptance/wave59/**` 与 `tests/useStreamingPresentationState.spec.ts`。

## 8. 红相定义

1. V1/V6/V7 必红（当前无 tail、无 CSS）；V4/V5/V9/V10 与 V1-6/7、V2-1/2/3/6、V3-1/2、V7-3/7、R8 属**基线绿（非回归护栏）**，不得算作本波成果。
2. V8 需在实现后重新采集相对基线；红相阶段只记录环境基线（V8-b）。
3. Lead 执行红相锚定提交：`test(wave-pds-3): freeze acceptance tests`。

## 9. 明确不做

- 不建任何字符级 DOM 节点（D-59-2 保持）。
- 不改 `DEFAULT_REVEAL_BUDGET`（保持 4 字/tick），不改 `useTimelineAutoScroll` 控制流。
- 不改 markdown 解析器本身（`marked` 用法不变），只改「定稿/尾巴切分」与「渲染」。
- 不做运行期热切换开关。

## 修订记录 v4（实施期发现的 6 处契约不严谨，由 Lead 采纳并追认）

| # | 问题 | v4 裁定 |
|---|---|---|
| 1 | §1 `data-tail-len` 语义歧义：尾巴必须按真 markdown 渲染（既有冻结用例要求流式 HTML 含 `<strong>/<p>/<pre>`），纯文本转义会打红 6 例 | **取"渲染后可见文本长度"**（`textContent` 长度）而非源片段长度。纯文本下两者等值；`**b**` 这类会不等，以可见长度为准 |
| 2 | §1 的"1 行上限"未覆盖代码块尾巴：栅栏闭合那一帧块级盒子合并导致 `scrollHeight` 回撤（实测 16–17px × 15 次） | **代码块尾巴必须嵌在 `<pre><code>` 内部**，闭合前后结构逐字节一致 |
| 3 | §2 只规定 localStorage 是 mount-only，未区分系统偏好 | **localStorage：mount-only 缓存；`prefers-reduced-motion` 与 `data-pds-reduce-motion`：每帧求值**（否则系统偏好变化不生效） |
| 4 | §3.2 未点名 keyframes 作用域：SFC `<style scoped>` 的 `@keyframes` 会被 Vue 加 scope hash 重命名，内联 `animationName` 解析不到 ⇒ 动画静默失效 | **keyframes 唯一真值放全局 `src/styles.css`**；内联只保留 name/duration/timing/fill，时长真值仍是 `useStreamingPresentationState.ts` 的 `animationDuration`（已从 350ms 改为 140ms） |
| 5 | §1「存在未闭合尾巴文本时 =1」边界未定义：尾巴最后一行只剩空白时无可见文本 | **尾巴可见文本为空 ⇒ 数量按 0 计**（否则与 V6 身高判据冲突） |
| 6 | 输入前提未记录：`HomeWorkspace` 原本剥掉**尾随换行**，渲染层永远看不到"块已闭合"，尾巴无法归零（V1-6 实测红） | 新增 `cleanStreamingHopText`：**流式正文只剥前导换行、保留尾随**；非流式与 reasoning 路径不变。此为 V1-6 成立的前提 |

**跨波次已知条件**：`V10-3`（实跑既有全量单测）当前被另一并发波次未提交的 `docs/observability.yaml` 触发的 `ob2 [D1]` 门禁卡红。Lead 已独立复现该归因；剔除该外部用例后全量 1191 绿。解除条件：该波次带 `[decl-change-approved]` 提交后重跑 V10-3。**此条不阻塞本波其余验收。**

## 修订记录 v5（双路审查 6 条 BLOCKING 的裁定，第二轮实施依据）

### 尾巴切分（A 路，必须改用仓内既有块切分器）
- **R-A1**：禁止"按最后一个 `\n` 切分"。必须复用 `src/lib/markdown.ts` 的 `splitMarkdownBlocks`（仓内既有、有测试的块切分器），尾巴 = **整个未闭合块**。当前实现导致：表格渲染成空 tbody + 竖线上屏、`text\n---` 变成 `<hr>` 而非 `<h2>`、`**加粗` 出字面星号、嵌套列表塌成并列 `<ul>`、多行段落裂成两个 `<p>`。**以上任一复现即为 BLOCKING。**
- **R-A2**：栅栏闭合帧不得产生"空 `<pre>` 二次块"。`inCodeFence` 为真时尾巴内嵌，为假时尾巴必须为空且不得残留空代码块。
- **R-A3**：`data-tail-len` 改为**源尾巴字符数**（不是渲染后可见文本长度），并由 V1-5 用同一 `splitMarkdownBlocks` 计算的期望值断言。

### 单行约束（契约让步，正确性优先）
- **R-A4（契约变更）**：**取消"尾巴 ≤1 行"的硬约束**。尾巴随未闭合块自然成多行。`[data-stream-tail]` 的遮罩仅在**尾巴实际单行**时启用（多行时 `mask-image: none`），效果优雅降级而不是牺牲 markdown 语义。
- V6 相应改为：**单行尾时高度 ≤ 行高×1.2；多行尾时不做高度断言，改为断言"不得出现块级盒子合并导致的 scrollHeight 回撤"（并入 V4）**。

### 降级（B 路）
- **R-B1**：`AgentRuntimeStatusBar` 的 rise 必须是**有效**的：`data-pds-status` 不能挂在被 `.runtime-status-shimmer` 覆盖的元素上靠特异度竞争。做法：在 shimmer 的 `animation` 声明里**并列**两个动画（`shimmer-sweep …, pds-status-rise …`），而不是让两条规则抢同一个 `animation` 简写。
- **R-B2**：属性轨补 `.runtime-status-orbit *::after`（1800ms 呼吸）；核对媒体轨已有对应规则，保持两轨对齐。
- **R-B3（裁定）**：`useStreamingPresentationState.ts` 的 +4 行注释**降级为 ADVISORY**（零行为影响）。但请把理由从行内注释移到本契约/ADR，源码恢复"仅 1 行改动"，以守住 §7 的写域纪律。

### 生产接线（B/A 两路共同指出）
- **R-B4**：`data-pds-reduce-motion` 在 `src/**` 无任何**写入者**，属性轨是死代码。本波裁定：**不新增 UI 开关**，属性轨保留为"可编程覆盖 + 测试可验证"能力，在 ADR 中明确登记为"已支持、未接线"，避免后人误以为生产可用。

### 证据卫生
- **R-ALL1**：`tests/acceptance/pds-streaming/receipts/e2e/*.json` 仍是红相数据（`tailFrames:0`）。实跑通过后必须刷新为绿相凭据，否则"有验收无凭证"。

**判定口径**：上述 R-A1/R-A2/R-B1/R-B2 任一未达即 FAIL。R-A3/R-A4/R-B3/R-B4/R-ALL1 需有明确落地痕迹。

## 修订记录 v6（V8 测量方法学修正：改为同轮 A/B 差分）

**问题**：Lead 复跑时 V8 红（三次 Δ = 3.2 / **7.3** / 3.1 ms）。查记录：失败那次 **idle 基线是干净的（16.90ms）**，但 stream 阶段 p95=24.20ms ⇒ 负载在**同一轮的两个阶段之间**漂移（当时 load average 4.27–7.66 / 16 核）。即：相对基线法只校正了"机器整体偏慢"，校正不了"测量期间被外部负载污染"。这使 V8 仍带骰子属性。

**裁定：V8 改为「同轮 A/B 差分」**，直接测量**本改动自身的开销**，而不是测量机器状态：

- 每一轮（attempt）在**同一页面**依次采：
  1. `idle` 基线 ≥320 帧（无流）
  2. **A（对照）**：`tail-reveal=off` + 流式 900 帧 —— 等价于 wave-59 行为
  3. **B（实验）**：`tail-reveal=on` + 流式 900 帧
- 判据（三条同时）：`p95_B − p95_A ≤ 4ms`；A、B 两阶段 **Long Task(>50ms) 均为 0**；`p95_A − p95_idle ≤ 6ms`（合理性自检：排除测量框架自身失真）
- **稳定性**：连续 **3 轮**全部满足才算通过；单轮不挑最好。
- **污染剔除**：某轮 `p95_idle` 偏离已记录基线（16.90ms）超过 **1.5ms** ⇒ 该轮标记 `[LOAD-CONTAMINATED]` 并**重测**（最多 6 轮）。被剔除的轮次必须连同标记一起落盘，禁止静默丢弃。

**理由**：A/B 在同轮、同页、同负载窗口内测量，机器变慢会同时抬升 A 与 B，差分把环境项消掉；同时 A 阶段本身就是"旧行为"，因此这条门禁回答的是"这个动效让每帧多花多少"——正是契约关心的量。

**保留**：`V8-b` 继续只采集并留档 idle 基线，不参与判定。

## 修订记录 v7（收缩尾巴作用域：只对"纯段落"发尾巴，彻底消除栅栏类缺陷）

**背景**：第二轮审查 A 路仍 FAIL，新暴露 5 条缺陷全部落在代码栅栏/列表边界（N1 未闭栅栏不在尾巴首行时**整行丢失**且屏上出现字面 ```；N2 栅栏已闭但块内仍有未定稿内容时**不发尾巴**违反 §1；N3 ```md 语言被 `unwrapOuterMarkdownFence` 吃掉 opener；N4 ` ```js ` info string 被当作闭合栅栏导致完成帧内容跳动；N5 loose list 被切成两块）。

**根因**：契约允许在"任意未闭合块"上切分 ⇒ 等价于要求本项目实现一个**增量 markdown 解析器**。这是无底洞，且每次修都在制造新的边界缺陷。

**裁定 v7：尾巴资格门（tail eligibility gate）**

只有当未闭合内容是一个**纯段落**（plain paragraph）时才发尾巴；否则**不发尾巴**，全额走 `stableHtml`（等同 wave-59 行为）。判定为"非纯段落"的充要条件（未闭合内容中任意一行满足即取消尾巴）：

1. 围栏行：`^\s*(```|~~~)`
2. 缩进代码：`^ {4,}\S`
3. 列表项：`^\s*([-*+]|\d+[.)])\s`
4. 引用块：`^\s*>`
5. 表格行：`^\s*\|` 或该行含 `|` 且下一行是 `^\s*\|?[\s:-]*\|`
6. HTML 块：`^\s*<[A-Za-z!/]`
7. setext 下划线：`^\s*(=+|-{3,})\s*$`
8. 数学/其它自定义围栏：`^\s*\$\$`

**同时删除**：`composeCodeTailHtml` 及其"尾巴内嵌 `<pre><code>`"整条路径（N1/N3 就住在这里）。代码块流式期间**不享受**尾巴浮现 —— 这是为正确性付的价，写进 ADR。

**影响评估**：正文流式绝大多数内容是普通段落，故观感收益基本保留；代码块/表格/列表在流式期回退到 wave-59 的"整块追加"形态。

**新增验收 V12（Test Agent 独占）**：对上述 8 类构造各造一条流式输入，断言 ① `[data-stream-tail]` 数量为 **0**；② 全文中**无字面围栏标记**、**无行丢失**（与 `renderMarkdown(全文)` 对齐）；③ done 帧无内容跳动。

**V11 组保留**（它们证明"纯段落尾巴"的语义正确性），但需补一条：**纯段落尾巴**（含行内加粗/行内代码/链接）必须发尾巴且语义正确。

### 其它必修（来自 B 路 PASS 的 ADVISORY，成本低）
- **R-B5**：guards R6 在 `data-pds-status≠null` 分支**没有任何 rise 断言** ⇒ 上一轮的"rise 空转"结构上不会被抓到。补一条真断言（断言 label 上 `animation-name` 同时含 shimmer 与 rise 两项）。
- **R-B6**：属性轨 shimmer 复位缺 `!important`，实测被 scoped 规则反压 ⇒ 降级后仍是渐变+透明填充。补 `!important`。
- **R-B7**：硬编码 `#78716c` 在 dark+reduced-motion 下对比度实测 **3.6:1 < AA 4.5:1**（修前 ~10:1）。改用 `currentColor` 或主题令牌，禁止硬编码灰。

### 文档补齐
- `data-pds-status` 现有第三枚举值 `enter-b`（双相位重播用），需在 primitives 契约注释与 ADR 中写明，避免被当作非法值移除。

## 修订记录 v8（补上纯文本快路径的尾巴 —— 否则效果对多数普通回复失效）

**问题（Test Agent 实测，Lead 已复核）**：`WorkspaceTurnItem.vue:475` 的**纯文本快路径**（`isSimpleTextContent` 命中时）渲染 `{{ event.content }}`，**完全不经过 MarkdownRenderer** ⇒ 这类回复**永远拿不到尾巴**。而"纯段落正文"恰恰是 v7 判定中最该享受浮现的形态。若不补，v7 的"观感收益基本保留"这句话是假的。

**裁定 v8**：
1. **纯文本快路径也必须发尾巴**，与原语共用同一标记 `[data-stream-tail]`、同一开关、同一降级规则。
2. **切分规则**：在已释放文本中取**最后一个空行**（`\n\n`）之后的部分作为尾巴；无空行时整个已释放文本即尾巴。
3. **遮罩规则同 v5 R-A4**：尾巴实际单行才启用 `mask-image`，多行一律 `none`。
4. **开关关闭 / reduced-motion / 非纯段落判定** ⇒ 不渲染尾巴，行为回到 v7 之前（即当前生产行为）。
5. **判定复用**：纯文本快路径天然是纯段落，但仍须走过同一资格函数，避免"纯文本里混入围栏"时误发。

**新增验收 V13（Test Agent 独占）**：
- V13-1 纯文本流式（无 markdown 标记的长中文段落）⇒ 尾巴数量 = 1，`data-tail-len` = 源尾字符数。
- V13-2 尾巴为单行时 `mask-image` 为渐变；跨行时 `none`。
- V13-3 开关 `off` / reduced-motion ⇒ 尾巴数量 = 0，且可见文本与关闭前**逐字一致**（防吞字）。
- V13-4 **无字丢失**：拼接 stable + tail 的可见文本，必须等于当前已释放文本（这是纯文本路径最容易出错的地方，必须逐字断言）。
- V13-5 done 帧无残留尾巴、无跳动。

**范围**：`WorkspaceTurnItem.vue` 已在 Executor 写域内，无需扩写域；若实现中发现需要改 `useStreamingPresentationState.ts` 之外的其它文件，必须先报 Lead。

## 修订记录 v9（资格门改语义判定：用渲染结果自证，而不是用正则黑名单）

**问题（第三轮审查 A 路实测，Lead 复核）**：v7 的 8 条行正则**不是"纯段落"的充要条件**，已实测 4 类漏网，且漏网后果正是 v7 宣称消除的形态：
- `\tconst z = 1;`（tab 缩进代码）⇒ 尾巴内出现 **`<pre>`**（`data-tail-len=13`）
- `# 标题一行` ⇒ 尾巴内出现 `<h1>`
- `正文\n***` / `___` ⇒ 尾巴内出现 `<hr>`
- `标题一行\n-` ⇒ 尾巴内出现 `<h2>`

**裁定 v9：资格判定改为「渲染结果自证」（render-then-verify），删除正则黑名单。**

判定流程（两条路径共用同一纯函数）：
1. 先对候选尾巴源文本渲染：`html = renderPartialMarkdown(tailSource)`。
2. **资格成立当且仅当**：渲染结果中**没有块级元素**——即顶层不得出现 `h1–h6 / ul / ol / li / pre / table / blockquote / hr / div / section / figure` 等块级标签；允许 `<p>`（且**至多一个顶层 `<p>`**）与任意行内标签（`strong/em/code/a/br/del/img/span`）。
3. 不成立 ⇒ **不发尾巴**，全额走 stable（回退形态）。
4. 判定必须用**同一渲染器**（`renderPartialMarkdown`）的结果，禁止再维护标签黑名单字符串。

**为什么这样能收敛**：判据与渲染器同源，渲染器升级则判据自动跟随；不存在"正则没想到的写法"。这是把"猜测渲染行为"换成"直接看渲染结果"。

**顺带修 v7 的两个副作用**：
- **V9-a**：围栏 opener 帧屏上出现字面 ``` 约 1–2 帧（v7 改用 `renderMarkdown` 后丢了 `autoCloseBoundaries` 的兜底）。要求：**非资格路径**仍走既有的 `autoCloseBoundaries` + 增量提交语义（即 HEAD 行为），只有**资格路径**才用新切分。这样既不回退 N3 修复，也不暴露字面围栏。
- **V9-b**：缩进代码与 loose list 的提交帧语义漂移（`<pre>`→`<p>`→`<pre>`、两个 `<ul>` 合并成一个）。因 v9 后这类构造**不再发尾巴**，漂移应回到 HEAD 水平；要求验证并留证。

**新增/加强验收**：
- **V14（资格门完备性）**：对**全部**已知块级构造各一条（含 tab 缩进、`***`/`___`、单 `-` setext、`#`/`##` 标题、四反引号、CRLF、`$$`、HTML 注释），断言尾巴数 = 0 **且** 尾巴 span 内不存在任何块级标签（这是关键断言，不能只断言"tails=0"）。
- **V12 补夹具**：现有 rule 5b（段落中 `|`+分隔行）、6（HTML 块）、7（setext）、8（`$$`）**没有任何夹具，删掉规则仍全绿**——必须补齐，否则等于没有门禁。
- **V12-5 / V12-6 修正**：V12-5 的源文本走纯文本快路径（不进 MarkdownRenderer）属"路径未到"的假绿；V12-6 的尾巴只有 `- 项目 B`，N5（loose list）原型并未被覆盖。两者须改为真正覆盖目标形态。

## 修订记录 v10（`$$` 分歧裁定：选 A —— 判据自洽优先于清单字面）

**分歧**：V14 清单把 `$$` 列为「tails 必须 0」，但 v9 的语义判据下 `renderPartialMarkdown("$$\nx = y\n$$")` = `<p>$$<br>x = y<br>$$</p>`，**无块级标签 ⇒ 判为合格 ⇒ 会发尾巴**。两者不可同时成立。

**裁定：选 A**。V14 对该项改为「tails ≤ 1 **且** 尾巴 span 内无块级标签」。理由：
1. v9 的核心决定是"判据与渲染器同源、删除行黑名单"。为 `$$` 破例重新引入 `^\s*\$\$` 黑名单，等于把刚删掉的机制装回去，且明天会出现第二个 `$$`。
2. 本项目不渲染数学，`$$` 在渲染层就是一个普通段落；尾巴里出现它不会造成块级嵌套、不会丢字符、不会改变语义（与 stable 区显示完全一致）。
3. **自校正**：若将来接入数学扩展，`$$` 会渲染成块级元素，语义判据**自动**判定不合格并发回 stable —— 无需改判据。这正是语义判据优于黑名单的地方。

同一条原则适用于其它"项目未渲染但语法上像块级"的构造：**以渲染结果为准，不以源文本形态为准。**

## 修订记录 v11（三处收尾裁定）

1. **`$$`（确认 v10 的选项 A）**：`$$` 在渲染层就是普通段落，**允许发尾巴**；判据只看"尾巴 span 内有无块级标签"，不看源文本形态。V14/V12 对该项**只断言"尾巴内无块级标签 + 文本无丢失"**，不硬断言 tails=0。契约 V14 清单里 `$$` 一条作废。
2. **loose list（接受 Test Agent 的结构性结论）**：`splitMarkdownBlocks` 遇空行必切块 ⇒ loose list **不可能整体落在一个未闭合块里**，故"流式期单一 `<ul>`"在本切分器下**结构不可实现**，属新增需求而非本波缺陷。要求仅为：**不丢行、不新增 HEAD 之外的结构抖动**；`2 ul → 1 ul` 的收块合并与 HEAD 同构，接受并记入 ADR。
3. **V8 瞬时尖峰（新增容许机制）**：A/B 差分消不掉"恰好压在 B 窗口的瞬时尖峰"（实测出现过单轮 B−A=+7.50，而 idle 与 A 均正常）。裁定：
   - 若某轮 **仅** `B−A > 4ms` 失败，且 `A − idle ≤ 6ms` 且**两段 Long Task 均为 0** ⇒ 该轮标记 `[SPIKE]`，**不计入 3 轮干净轮**，也**不直接判失败**，自动补测；
   - 总尝试上限 **8 轮**；若 8 轮内拿不到 3 轮干净 ⇒ FAIL；
   - 所有 `[SPIKE]` 轮次必须落盘（与 `[LOAD-CONTAMINATED]` 同等留证），**禁止静默丢弃**。
   - 注：这是"容许重测"而不是"放宽阈值"——阈值仍是 4ms，3 轮干净仍是硬要求。
