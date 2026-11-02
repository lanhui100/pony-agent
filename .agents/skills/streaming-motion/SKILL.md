---
name: streaming-motion
description: 在实现或评审大模型流式输出的前端动效时使用——包括给流式正文/思考过程/工具调用加入场与揭示动画、消除流式期的闪烁与视口抖动、决定滚动跟随与用户手势的优先级、编写跨框架（Vue/React/Svelte/原生）可复用的动效规范、或为 streaming 渲染层编写可机器验收的门禁。触发词：streaming 动效、流式闪烁、逐字 reveal、流式滚动跟随、打字机效果、消息入场动画、motion token、stale-while-render 动效。
---

# Streaming Motion（PDS-SM）

流式输出的动效设计规范与可复用资产。**框架无关**：动效本体是纯 CSS，节奏与视口逻辑是纯函数，
框架层只留约 40 行胶水。

## 何时用

- 要给"模型正在吐字"的正文、思考流、工具调用加入场/揭示动效；
- 流式期出现**闪烁、行位移、视口被拽回**；
- 需要一次定好令牌与降级策略，避免动效散落进各组件；
- 需要为 streaming 渲染层写**可机器断言**的验收（而不是"看起来挺顺"）。

## 何时不该用

- 只想做一个静态页面的入场动画 —— 用普通 CSS transition 即可，不必引入五层结构；
- 想在单个组件里临时内联一段 `animation` —— 见 [anti-patterns §8](anti-patterns.md)，请改为新增原语；
- 需要转场级编排（共享元素、页面切换）—— 不在本规范范围。

## 五层结构与文件地图

> L1–L5 是规范本体；**L6（胶水）故意不提供实现**，只给范式——它必须按宿主框架重写，
> 提供成库就会把框架差异固化进来。这就是本规范只到五层的原因。

| 层 | 职责 | 文件 | 可移植性 |
|---|---|---|---|
| L1 语义 | 阶段划分与"每阶段允许什么动效" | 本文件 §阶段模型 | 100% |
| L2 令牌 | 缓动、时长、位移、视口常量 | [`tokens.css`](tokens.css) | 100% |
| L3 原语 | `@keyframes` + `data-pds-*` 属性驱动，零 JS | [`primitives.css`](primitives.css) | 100% |
| L4 逻辑 | 两个零依赖 reducer | [`logic/budget.ts`](logic/budget.ts) · [`logic/intent.ts`](logic/intent.ts) | 100% |
| L5 门禁 | 七条可机器断言指标 | [`conformance.md`](conformance.md) | 规范可移植；命令需改路径 |
| L6 胶水 | 挂 tick / 挂监听 / 写属性（**不在本规范内**） | [`adapters/vue.md`](adapters/vue.md) · [`adapters/react.md`](adapters/react.md) | 每框架重写 |

配套：[`anti-patterns.md`](anti-patterns.md) —— 八条踩过的坑，动手前先读。
决策依据：[`docs/decisions/0057`](../../../docs/decisions/0057-streaming-motion-framework-agnostic-spec.md)。

## 阶段模型（L1）

| 阶段 | 允许的动效 | 禁止 |
|---|---|---|
| `idle` | 无 | — |
| `awaiting`（已提交、未收到首信号） | 状态条文字扫光；**单一**等待指示 | 与状态文案重复播报同一事实（§4） |
| `reasoning` | 状态行升降（`data-pds-status`）、尾巴揭示 | 折叠/展开高度动画（会抖动） |
| `content` | 尾巴揭示（`data-pds-reveal`）、块级入场（`data-pds-block-enter`） | 逐字符节点（§1）、尾巴 blur（§2） |
| `settled` | 整条消息入场（`data-pds-message-enter`，只播一次） | 对已渲染内容重播动画（§5） |

**关键约束**：同一时刻只允许一个"进行中"指示在播报。多一个就等于多一个假任务。

## 接入流程（四步）

1. **引入资源**：`@import tokens.css` + `primitives.css`（全局一次）。
2. **接释放节奏**：用 `budget.ts` 的 `stepReveal` 驱动可见文本游标。上游 content 长度进，游标出。
   - tick 周期 60ms 为默认；**`charsPerTick` 必须保证一帧配额消化后仍有"落后感"但不积压**。
3. **接视口意图**：用 `intent.ts` 的 `reduceScrollIntent`。三类事件必须投递：
   用户物理手势、内容追加、视口滚动回读。**自己发起的滚动必须不进事件流**。
4. **挂属性**：宿主渲染时按阶段写 `data-pds-*`。不要内联 `animation`。

## 不可协商的约束（红线）

1. **字符级动效禁止建 DOM 节点**。游标是整数，动效挂在单个尾巴节点上。
2. **每 tick 重建的节点只准 `opacity` / `transform`**，且**不加 `will-change`**。
   `filter` / `will-change` 只给一次性、身份稳定的块级入场。
3. **reducer 不许读时钟、不碰 DOM、不 import 框架**。时间由事件携带。
4. **降级必须 `animation: none`**，不许把时长压到极低（那仍会闪一帧）。
5. **参数必须带适用前提**。引用上游常量时同时写明"前提不成立改用什么"。
6. **禁止 `behavior: "smooth"` 做流式跟随**；先保证内容增长与视口对齐同帧，缓动只用于用户主动导航。
7. **非法输入一律 fail-closed**：
   - 长度非法（NaN / ±Infinity / 负数）⇒ 整帧不推进，**绝不回退已显示游标**（否则正文整段消失再重打）；
   - 时序不可知（`lastUserInputAtMs` 仍为初值，例如用户只拖滚动条）⇒ **永不自动恢复跟随**；
   - 时间戳非法 ⇒ 判为"冷却中"，不移动视口。

## 验证

- L4 有单测模板：见本项目 `tests/streaming-motion-budget.spec.ts` / `tests/streaming-motion-intent.spec.ts`
  （44 例，含畸形输入对抗与 2 条回归用例）。
- L2/L3 无逻辑，只需静态约束 C6 + 降级 C7。
- 渲染层接入后跑 C1/C2/C5（jsdom 即可，一分钟接入）。

## 参数校准记录

| 参数 | 上游值 | 本规范采用值 | 理由 |
|---|---|---|---|
| 底部容差 | 40px | **260px** | 40px 在长回复下频繁误判"已离开底部"，产生 ±1 行方向反转抖动 |
| 冷却窗口 | 150ms | 150ms | 实测足够，未调整 |
| 空闲恢复 | 无此机制 | **3000ms** | 用户的"暂停阅读"远长于 150ms，需独立阈值 |
| 尾巴 reveal | `blur(6px)` | **`opacity` + `translateY(4px)`** | 尾巴每 tick 重建，`filter` 会逐 tick 创建合成层 |
| reveal 时长 | 120ms | 120ms | 必须短于 token 到达间隔，否则动画积压成拖影 |
