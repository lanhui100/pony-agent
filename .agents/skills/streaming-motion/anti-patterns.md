# 反模式清单（Anti-Patterns）

> 这些不是假想出来的坏味道，是本项目交过学费换来的。每条都附可复核的证据位置。
> 正例规范见 [SKILL.md](SKILL.md)，验收方法见 [conformance.md](conformance.md)。

## §1 逐 token 建 DOM 节点

**症状**：为了做"逐字淡入"，给每个字符包一个 `<span>` 并各自挂动画延迟。

**为什么错**：节点数从 `O(1)` 变成 `O(N)`；每 tick 追加内容时新节点从 `opacity:0` 重播，
已可见字符被重新排布 —— 用户看到的是"文字反复闪一下再稳定"。长回复下逐帧成本线性上升。

**本项目证据**：该实现曾经存在（`.mr-streaming-char` / `.assistant-streaming-char`），
在 wave-59 被明确移除并写入冻结验收：

- 契约决策：`D-59-2` —— [wave-59-desktop-uiux.md](../../../.dev-team/contracts/wave-59-desktop-uiux.md)
- 冻结断言：`F3-1` / `F3-4` —— [wave59-f3-streaming-dom-stability.spec.ts](../../../tests/acceptance/wave59/wave59-f3-streaming-dom-stability.spec.ts)

**替代**：字符释放走 L4 reducer（只推进一个整数游标），动效挂在**单个**尾巴节点上（`data-pds-reveal`）。

---

## §2 对"每 tick 重建"的节点使用 `filter: blur()` 或 `will-change`

**症状**：照搬"模糊消解"效果，给尾巴 span 加 `filter: blur(6px) → blur(0)`；或为"优化"给它加 `will-change: opacity`。

**为什么错**：`filter` 与 `will-change` 都会让该元素成为独立合成层。尾巴节点每 tick 重建 ⇒ 每 60ms
创建 + 销毁一个合成层，在 100 tok/s 下直接掉帧。**看起来更"高级"的属性反而更慢。**

**判据（可用一句话决定用哪个）**：
- 节点**每 tick 重建** → 只准 `opacity` / `transform`，且**不要**加 `will-change`。
- 节点**一次性插入且身份稳定** → 才可以用 `filter` 与 `will-change`。

**本项目证据**：`.assistant-streaming-fade` 全程只用 `opacity`，
见 [WorkspaceTurnItem.vue](../../../src/components/chat/WorkspaceTurnItem.vue)。

---

## §3 流式期用 `behavior: "smooth"` 做程序化滚动

**症状**：每次内容增长 `scrollTo({ top: max, behavior: "smooth" })`。

**为什么错**：`smooth` 是 rAF 驱动的动画，下一个 tick 到达时会被**打断并重启**，
表现为视口持续"追赶但永远追不上"，且方向反复反转。上游方案改用 `behavior: "auto"`
（瞬时对齐）规避这点，但瞬时对齐在长回复下会带来逐行抖动。

**本项目证据**：wave-11 基线实测 **44 次方向反转 / ±23.8px 位移**，
最终解法是"同帧补偿"——在同一微任务内完成小位移对齐（≤96px），超出才回落缓动：

- [useTimelineAutoScroll.ts](../../../src/lib/useTimelineAutoScroll.ts) 的 `SAME_FRAME_COMPENSATION_MAX_PX` 与 `queueScrollToLatestTurn`

**替代**：先保证"内容增长与视口对齐发生在同一帧"，再谈缓动。缓动只用于**用户主动导航**。

---

## §4 同一事实被播报两次

**症状**：既有"三点等待动画"在消息流里转，又有底部状态条显示"思考中"。

**为什么错**：两个动效同时表达"正在等待"，用户会误以为是两条并行任务；
而且两者生命周期不同步时，会出现"点还在转但状态已变"的错位。

**本项目证据**：这是 ADR 0056 记录的四项缺陷之首（"等待首信号时三点跳跃动画与底部状态条
『思考中』重复"），最终**删除三点动画**，把指示权唯一收归状态条，
并用文字扫光（而非点阵）承担持续动效。

**规则**：一个事实只能有一个播报者。启用 `data-pds-dots` 前先确认宿主是否已有状态文案在播报同一件事。

---

## §5 对会重播的节点使用 `animation-fill-mode: both`

**症状**：入场动画写 `animation: xxx both`，节点被重建后动画重播。

**为什么错**：`both` 让 `0%` 关键帧在动画开始前就生效。节点一旦重建，即使内容没变，
也会先被拉回 `opacity: 0` 再播一遍 —— 这就是"闪一下"。

**本项目证据**：注释明确记录过这个坑：`animation-fill-mode: both` 在流式→完成过渡时
"将已渲染的可见元素重置为 opacity:0 导致闪烁"，见
[WorkspaceTurnItem.vue](../../../src/components/chat/WorkspaceTurnItem.vue) 的 `.assistant-response-panel.motion-entrance` 注释。

**规则**：`both` 只用于**身份稳定**的节点；会重建的节点用 `forwards`（或不用 fill）。

---

## §6 用 `scrollHeight` 测量来驱动展开动画

**症状**：展开/折叠时 `el.scrollHeight` → 设 `height` → 过渡 → 再设 `height: auto`。

**为什么错**：每帧读 `scrollHeight` 会触发**同步布局**（forced reflow）；
在流式追加期间叠加，等于把布局计算塞进动画帧。且 `height: 0 → auto` 无法过渡。

**替代**：`grid-template-rows: 0fr ↔ 1fr`（`data-pds-expand`）。一次样式变更、零测量。
代价：子元素需要 `overflow: hidden`，会裁剪焦点环与外投影 —— 容器要预留内边距。

---

## §7 把上游参数当普适常量

**症状**：文档写"底部容差 40px、冷却 150ms、逐字 blur 6px"，直接照抄到新项目。

**为什么错**：每个参数都带**适用前提**，脱离前提即失效：

| 参数 | 上游值 | 前提 | 本项目实测结果 |
|---|---|---|---|
| 底部容差 | 40px | 上游消息流骨架与滚动容器结构 | **不足**，wave-14 需 260px |
| 逐字 blur | 6px / token | token 是独立且不重建的节点 | **不成立**，本项目逐 tick 重建尾巴 ⇒ 改 `opacity + translateY` |
| 程序化滚动 | `behavior: "auto"` | 依赖上游的帧调度时序 | 需改为同帧补偿 + 上限 96px |

**规则**：引用参数时必须连同"来源 + 适用前提 + 若前提不成立改用什么"一起引用。
本 skill 的 `tokens.css` 与 `intent.ts` 已把这条写进注释，改参数时请不要删掉注释。

---

## §8 在组件内写内联 `animation` 或一次性框架动画调用

**症状**：`style={{ animation: "fade-up .4s ..." }}`，或 `motion.div` 临时加一段过渡。

**为什么错**：动效散落到组件后，无法统一降级（`prefers-reduced-motion` 只能逐个补）、
无法静态审计（C6 门禁扫不到）、也无法被下一个项目复用。

**规则**：任何流式动效必须落在 L2 令牌 / L3 原语 / L4 reducer 之一。
确需新原语时，加进 `primitives.css` 并补 `data-pds-*` 契约，而不是就地内联。
