# L5 一致性门禁（Conformance Gates）

> 这套门禁是 PDS-SM 能被称为"最佳实践"而非"一段好看的 CSS"的原因：
> 它把"手感"翻译成**可由机器断言的物理事实**。换框架后照样能测。

七条指标全部来自本项目已落地并冻结的验收（`tests/acceptance/wave59/`、`tests/e2e/wave59-stream-stability.spec.ts`），
因此不是凭空设计的清单。

## C1 字符级节点数为 0

- **断言**：流式全过程中，内容容器内**元素节点数不随字符数线性增长**；若实现确实引入了字符级节点，
  它们必须打上 `data-pds-char` 标记，而该标记数量恒为 `0`。
- **判定**（二选一，接入时先用客观法）：
  - **客观法（无需任何标记）**：同一段 500 字内容，分别在"已显示 100 字"与"全部 500 字"两个时刻记录
    `container.querySelectorAll("*").length`。两次差值必须 ≤ 20（常量级），不得随字数增长。
  - **标记法（可门禁化）**：`container.querySelectorAll("[data-pds-char]").length === 0`。
- **关于 `data-pds-char`**：它**不是** L3 原语的一部分（规范刻意不提供字符级原语），而是一个**禁用标记**——
  存在的唯一目的是让"是否有人偷偷建了字符节点"这件事可被机器检查。本仓冻结门禁采样的是
  `.mr-streaming-char` / `.assistant-streaming-char` 类名（`wave59-f3-...spec.ts`）。
- **理由**：`O(N)` 节点 + 每 tick 重建 ⇒ 必然闪跳与掉帧。详见 [anti-patterns §1](anti-patterns.md)。
- **例外**：无。

## C2 正文承载节点身份不变

- **断言**：`streaming → done` 切换前后，正文承载节点是**同一个 DOM 引用**。
- **判定**：切换前取 `Element` 引用并与切换后 `querySelector` 结果做 `===` 比较；同时断言最终文本含完整内容（排除"空容器恰好也满足身份不变"的假绿）。
- **理由**：一次性替换整棵子树 = 完成瞬间重绘闪帧。这是本项目 RC5 的根因。

## C3 `scrollHeight` 逐帧不回撤 >1px

- **断言**：流式采样窗口内，内容列与正文容器的 `scrollHeight` 单调不减（容差 1px）。
- **判定**：真实浏览器 `requestAnimationFrame` 逐帧采样 `scrollHeight`，过滤回撤 >1px 的帧；采样帧数须 >20（防止样本不足导致空转通过）。
- **理由**：高度回撤 = 内容被重新排版/吸回上一行，用户看到"文字往回跳"。

## C4 首个非空正文延迟 ≤ 4 帧

- **断言**：内容到达（长度 >0）到正文首次出现非空文本之间，帧数差 ≤ 4。
- **判定**：逐帧同时记录 `contentLength` 与 `bodyTextLength`，取两者首次 >0 的帧号相减，断言 `≤ 4`。
- **为什么是 4 而不是 1**：语义目标确实是"滞后 ≤1 个渲染周期"（stale-while-render），
  但真实测量中一个 render 周期包含 16ms 帧 + markdown 解析，跨帧抖动大；本仓冻结门禁取
  **4 帧（≈66ms）** 作为工程余量（`FIRST_TEXT_MAX_FRAME_LAG = 4`，
  `tests/e2e/wave59-stream-stability.spec.ts`）。**接入方可按自己的解析开销收紧，
  但不要把 1 当普适阈值** —— 那会把正常实现判红，门禁随即被绕过。

## C5 正文首次非空后无空白帧

- **断言**：正文一旦出现过非空文本，之后任何"内容长度 >0 但正文为空"的帧都是违规。
- **判定**：逐帧过滤统计，违规帧必须为空数组。
- **理由**：这是"白屏闪"的直接定义。

## C6 静态约束（两条子规则）

### C6a 动画属性白名单

- **断言**：`@keyframes` 内声明的属性、以及 `transition` 的目标属性，只出现 `opacity` / `transform` / `filter` / `grid-template-rows`。
- **判定**：只扫"真正被动画的属性"，不扫静态布局声明（`min-height`、点阵尺寸这类必须放行，否则门禁永远红）：

```bash
# ① @keyframes 体内
awk '/@keyframes/{inblock=1} inblock{print FILENAME":"FNR":"$0} inblock&&/^}/{inblock=0}' \
  .agents/skills/streaming-motion/primitives.css \
  | grep -E '\b(width|height|top|left|right|bottom|margin|padding)\s*:'
# 期望：无输出

# ② transition 目标属性（排除降级块里的 `transition: none`）
grep -nE 'transition\s*:' .agents/skills/streaming-motion/primitives.css \
  | grep -vE 'grid-template-rows|:\s*none'
# 期望：无输出
```

- **理由**：前三者走合成器线程；`grid-template-rows` 是唯一被允许的布局动画，且必须作用在已知内部高度的容器上。
- **已知例外**：`[data-pds-expand] > *` 里的 `min-width/min-height: 0` 与 `[data-pds-dots] > *` 的 `width/height` 是**静态**声明（0fr 折叠与点阵尺寸的必要条件），不参与动画，故不在白名单校验范围。

### C6b L4 纯函数性

- **断言**：`logic/*.ts` 的**代码**不 import 任何 UI 框架，不访问 DOM，不读时钟。
- **判定**：先剥掉注释再扫（否则文档里提到 `performance.now` 就会误报）：

```bash
sed 's://.*$::' .agents/skills/streaming-motion/logic/*.ts \
  | grep -nE "from ['\"](react|vue|svelte|@angular)|\bwindow\.|\bdocument\.|\bDate\.now|\bperformance\.now|\brequestAnimationFrame"
# 期望：无输出
```

- **理由**：时钟与 DOM 一旦进入 reducer，就无法在任意框架、任意测试环境（含纯 Node）下逐分支验证。

## C7 降级后 `animation: none` 且终态正确

- **断言**：两种降级通道（`@media (prefers-reduced-motion: reduce)` 与 `[data-pds-reduce-motion="true"]`）下，所有 PDS 原语的 `animation-name` 均为 `none`，且元素呈现**终态样式**（`opacity:1`、无位移）。
- **判定**：在开启降级的页面读取 `getComputedStyle(el).animationName === "none"`，并断言 `opacity === "1"`。
- **禁止的替代实现**：把 `animation-duration` 压到 `0.01ms`。那仍会闪一帧，且与 `animation-fill-mode` 存在竞态。
- **配套**：`budget.ts` 的 `reduceMotion: true` 分支必须一步释放全部字符（单测已锁）。

---

## 如何套用到其他框架

门禁**规范**与框架无关，但 C6 里的 shell 命令把路径写死成了本仓的相对路径
（`.agents/skills/streaming-motion/...`）。复制到其他项目时必须替换为目标项目内这四类文件的路径。
这与 L1–L4 的"代码与令牌可零改动复用"不是同一层次的承诺：**规范可移植，命令要改路径。**

只有"怎么驱动采样"需要按框架适配：

| 框架 | 逐帧采样入口 | 内容增长信号 |
|---|---|---|
| 任意 | `requestAnimationFrame` 循环读 DOM | 你的 store / signal 的 content 长度 |
| Vue | `watch(..., { flush: "post" })` + `nextTick` | store getter |
| React | `useEffect`（post-commit）+ `useLayoutEffect` | `useSyncExternalStore` 选择器 |
| Svelte | `$effect`（DOM 更新后运行） | `$derived` |

**建议的最小可行门禁集**：C1 + C2 + C5 + C6。四条都不需要真实浏览器（jsdom 足够），
能在一分钟内接入任意项目。C3 / C4 需要真实 Chromium，成本高，建议只在"流式渲染层重构"这类变更上启用。
