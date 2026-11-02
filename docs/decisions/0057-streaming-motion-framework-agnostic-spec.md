# 0057 流式动效抽象为框架无关规范（五层契约 + 纯 reducer + 机器门禁）

Status: implemented

## 背景

[0056](0056-streaming-render-stale-while-render-and-hop-status.md) 已把流式正文渲染收敛为 stale-while-render，并在同波删除了逐字符淡入（D-59-2）。由此产生两个待解问题：

1. **动效知识只剩否定形态**。「不要逐字符 span」「不要每 tick 用 blur」这类结论记录了**不能做什么**，但没有沉淀**应该怎么做**的正面规范。下一个新会话、或另一个前端项目，无法据此实现流式动效。
2. **可复用部分与不可复用部分混在一起**。对照 `open-claude-ui` 的还原方案可见：`@keyframes`、`cubic-bezier`、Grid `0fr↔1fr` 是浏览器原语（完全可移植）；而 Framer Motion 的 `Transition` 对象、`useXxxScrollPinning` 这类 hook、JSX 组件与其调度语义（`deps[]` 重锚定、`nextTick` 时序）绑定框架，跨框架不可平移。

本决策只确立**规范与资产**（`.agents/skills/streaming-motion/`），**不接入运行时代码**：本仓 `src/` 的流式行为本次零改动，采纳到调用点是后续独立波次的议题。

## 候选方案

- **方案 A：抽成 npm 包发布**（如 `@pony/streaming-motion`）。落选：动效本质是视觉方言，发包即背兼容矩阵；且跨框架包要么内嵌运行时框架判断、要么分裂成 Vue/React/Svelte 多个包，维护成本远高于收益。真正可复用的只有"规范 + 令牌 + reducer + 门禁"四件，复制成本极低。
- **方案 B：只写一篇设计文档**。落选：文档不可执行、不可回归。本仓 `F3-1` / `F3-4` / `F3-5` 三条冻结门禁恰好证明——流式"手感"是可以被机器断言的物理事实；纯文档等于主动放弃这个杠杆，且必然随时间腐化。
- **方案 C：CSS-only 自定义元素**（`<pds-stream>`）。落选：自定义元素必须接管内容渲染与滚动容器，等于从 `marked` + Vue 手里抢走渲染权；与本仓 D-59-1（渲染输入在 `executeRender` 入口快照、`.markdown-body` 唯一承载节点）直接冲突。
- **方案 D（采纳）：四层契约 + 两个纯 reducer + 机器门禁，胶水留给宿主框架**。CSS 令牌与原语全解耦；节奏预算与视口主权抽成**零依赖纯函数**；框架层只保留挂 tick / 挂监听 / 写 `data-pds-*` 属性的胶水（Vue 与 React 各约 40 行）。对外接口是 **DOM 属性 + CSS 变量**，不是 JS API——这是唯一所有框架（含 SSR、web components、原生）都能表达的形式。
- **方案 E：直接照搬 `open-claude-ui` ／ Claude Desktop 的原始参数**。落选：其参数带强前提且已被本仓实测证伪——① 图钉阈值 40px / 冷却 150ms 在本仓不足（wave-14 需 260px 阈值 + 同帧补偿 ≤96px + 320ms lerp）；② 逐 token `filter: blur()` 要求 token 是独立且不重建的节点，与本仓逐 tick 重建尾巴的结构冲突（违背 F3-1/F3-4）。采纳时必须在参数旁标注**来源与适用前提**，禁止当作普适常量。

## 决策

1. **五层边界**：L1 语义（阶段状态机）／L2 令牌（CSS 变量）／L3 原语（`@keyframes` + `data-pds-*` 驱动，零 JS）／L4 逻辑（`budget.ts` 与 `intent.ts` 两个纯 reducer）／L5 门禁（七条可机器断言指标）。框架胶水是第 6 层，故意不提供实现，只提供范式。
2. **唯一允许的动画属性集**：`opacity` / `transform` / `filter` / `grid-template-rows`。前三者走合成器；`grid-template-rows` 是唯一被允许的布局动画，且只用于已知内部高度的展开容器。禁止动画 `width` / `height` / `top` / `left`。
3. **分档揭示**：token 级（`--pds-dur-reveal: 120ms`）只准 `opacity + translateY`，**不准 blur**——因为该节点每个 tick 重建，`filter` 会强制逐 tick 创建/销毁合成层。blur 只用于**一次性**的块级入场（`pds-block-enter`，前 30% 保持全透明再显影，避免"形变可见、显影再发生"的双闪）。
4. **节奏预算与视口主权必须是纯函数**：`stepReveal(state, input, cfg)` 与 `reduceScrollIntent(state, event, cfg)` 不 import 任何框架、不读 DOM、不取时间戳（时间由事件携带）。因此它们可在任意框架与任意测试环境下逐分支验证。
5. **降级双轨且以 `animation: none` 收尾**：`prefers-reduced-motion: reduce` 与 `[data-pds-reduce-motion="true"]` 都必须把动画降为 `none`（元素直接呈现终态样式），而不是把时长压到极短——后者仍会闪一帧。同时 `budget` reducer 的 `reduceMotion` 分支一步释放全部字符。

## 影响

- **本仓运行时零变化**：`.agents/skills/streaming-motion/` 不被 `src/` 引用，`docs/observability.yaml` 四维归属不变（无新增接口 / 网络 IO / 后台任务 / LLM 调用 / UI 交互变更）。
- **新增可复用资产**：[`SKILL.md`](../../.agents/skills/streaming-motion/SKILL.md)、`tokens.css`、`primitives.css`、`logic/{budget,intent}.ts`、`conformance.md`、`anti-patterns.md`、`adapters/{vue,react}.md`。
- **新增回归保护**：`tests/streaming-motion-budget.spec.ts` 与 `tests/streaming-motion-intent.spec.ts` 纳入 `npm run test:unit`，锁定 reducer 的单调性、越界裁剪、停滞收敛、冷却窗口与降级行为。
- **后续动作（已由 [0058](0058-pds-budget-reducer-adoption.md) 部分落地）**：`budget` reducer 已接入 `useStreamingPresentationState`（等价重构，生产释放路径自此由 reducer 单测覆盖）；`intent` reducer **仅常量已被 `useTimelineAutoScroll` 引用**，其控制流未状态机化——现实现含三套互锁与双 lerp，与 reducer 的简化模型不同构，替换属独立波次（wave-pds-2），须另行提案。`intent.ts` 的参数默认值取本仓实测值（260px / 150ms / 3000ms），而非上游常量。
- **约束后续贡献者**：任何新增流式动效必须落在 L2/L3（纯 CSS）或 L4（纯函数）任一层；在组件内写内联 `animation` 或框架动画库的一次性调用，视为绕过本规范。
