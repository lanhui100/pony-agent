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
| Executor | `src/components/MarkdownRenderer.vue`、`src/components/chat/WorkspaceTurnItem.vue`、`src/components/HomeWorkspace.vue`、`src/components/chat/AgentRuntimeStatusBar.vue`、`src/styles.css` |
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
