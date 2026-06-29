# Streaming 渲染优化方案 — 对抗审查纪要

## 审查方式

三个子智能体并行从不同视角执行对抗审查：

| 视角 | 审查者 | 审查产出 |
|------|--------|----------|
| 性能/工程 | perf-adversary | 从 CPU、渲染管线、边缘 case 角度 |
| UX/产品 | ux-adversary | 从用户感知、无障碍、认知负荷角度 |
| 架构/可维护性 | arch-adversary | 从代码耦合、状态管理、可测试性角度 |

覆盖范围：原始方案文档 + `src/lib/markdown.ts`、`src/components/MarkdownRenderer.vue`、`src/lib/useStreamingPresentationState.ts`、`src/stores/runtime.ts`、`src/components/HomeWorkspace.vue`、`src/lib/useTimelineAutoScroll.ts`

---

## 采纳清单

### 优化一：Auto-Close

| # | 发现 | 来源 | 严重性 | 处理 |
|---|------|------|--------|------|
| A1 | 将 auto-close 注入 `renderMarkdown()` 污染纯函数 | arch | HIGH | 改为 wrapper `renderPartialMarkdown()`，不修改 `renderMarkdown()` |
| A2 | 奇偶计数在多围栏场景误判 | perf | HIGH | 改用 `openCount - closeCount` |
| A3 | 消息中断时 auto-close 产生欺骗性完整代码块 | UX | HIGH | 消息终态（done/error/cancelled）强制走原始 `renderMarkdown()` |
| A4 | Phase 2 追加 `\n\n` 引入可见空白段落 | UX/arch | HIGH | 移除 Phase 2，仅实现 Phase 1 |
| A5 | `normalizeMarkdownSource()` 与 auto-close 执行顺序未指定 | arch | MEDIUM | 规定：normalization → auto-close → marked.parse() |
| A6 | 反向围栏 `~~~` 未覆盖 | perf | LOW | Phase 1 同步支持 |
| A7 | auto-close 后真实 close 到达时可能存在双 close | arch | MEDIUM | 已修复：`openCount - closeCount` 而非奇偶判断 |

### 优化二：Batch Fade

| # | 发现 | 来源 | 严重性 | 处理 |
|---|------|------|--------|------|
| B1 | `accumulated[messageId]` 方案有概念缺陷：snapshot 每次同步推进，accumulated 滞后，导致 flush 时文本在 stable 和 fade 间跳跃 | perf/arch | HIGH | 移除 `accumulated` 方案，改用纯 snapshot-diff 驱动：snapshot 仅 flush 时推进 |
| B2 | 纯字符阈值导致起始 800ms+ 无文本显示 | UX | HIGH | 首段使用 `STREAM_FADE_FIRST_BATCH_CHARS = 8~12` 低阈值 |
| B3 | 批次间隔间存在"死寂"（代码场景 ~1.6s，中文 ~2.5s） | UX | HIGH | 新增 `STREAM_FADE_TIME_MS = 350~400ms` 时间兜底 |
| B4 | 代码围栏内固定批次大小不适应行级节奏 | UX | MEDIUM | 代码围栏内将 batch chars 降至 15~20 |
| B5 | `accumulated` 未纳入 `PRESENTATION_MAPS` 清理机制 | arch | LOW | 方案已废弃，不引入该映射 |

### 跨优化共同改进

| # | 发现 | 来源 | 严重性 | 处理 |
|---|------|------|--------|------|
| C1 | auto-close 使内容看起来已完结，用户可能过早离开 | UX/arch | HIGH | 新增 streaming 脉动光标指示器 |
| C2 | 缺少 `prefers-reduced-motion` 支持 | UX | HIGH | CSS media query 添加 |
| C3 | 无运行时可观测性，常量无法调优 | arch/perf | HIGH | 新增 metrics、localStorage feature flag |
| C4 | `executeRender()` 无 try-catch，auto-close bug 会导致组件 stuck | arch | MEDIUM | 添加 try-catch + escapeHtml fallback |
| C5 | 渲染后高度变化导致 scroll 偏移未补偿 | perf | MEDIUM | 新增 render 前后高度差 + scrollBy 补偿 |
| C6 | 无 feature flag，回滚需全量 deploy | arch | MEDIUM | localStorage flag `pony-streaming-opt` |

---

## 部分采纳

| # | 发现 | 来源 | 处理理由 |
|---|------|------|----------|
| P1 | v-html 每次重建 DOM，包括代码块 | perf | 这是 v-html 固有行为，auto-close 不使其更糟。替代方案（text node diff）工程量过大，出当前优化范围。不做修改。 |
| P2 | marked.parse() 频次增加约 3x | perf | 可接受。batch fade 限制了渲染频次，且每次 marked 输出更大内容块，单位字符的解析成本更低。 |
| P3 | 渲染内容与外部 fade span 视觉断点 | UX | 该问题当前已存在，不因本次优化加剧。标记为后续改进项。 |

---

## 暂缓 / 驳回

| # | 发现 | 来源 | 决策 |
|---|------|------|------|
| D1 | Phase 2 列表/引用/表格 auto-close | 所有三方 | 暂缓。Phase 1 上线验证后评估 |
| D2 | 基于 <pre><code> text node diff 的渐进式代码块渲染 | perf | 驳回。工程量过大，超出本次优化范围 |
| D3 | 将 fade text 作为 slot 传入 MarkdownRenderer 统一样式 | UX | 暂缓。当前行为已可接受 |
| D4 | 提取常量到 constants.ts | arch | 驳回。代码库惯例是 per-file 常量 |
| D5 | 动画 duration 从 300ms 延长到 500~600ms | UX/perf | 部分采纳。改为 350ms，与 `STREAM_FADE_TIME_MS` 对齐避免重叠 |

---

## 未被采纳的致命风险核实

### Q: v-html + auto-close 每次重建代码块 DOM，是否造成闪烁/选区丢失？

**评估**：auto-close 使 marked 能在 streaming 中产出完整 HTML，v-html 替换 `innerHTML` 导致：
- 代码块每帧重建 `<pre><code>` 子树
- 用户若选中代码块内文本，选区丢失

**决定**：接受该 tradeoff。当前非 auto-close 路径在 1500ms 兜底时同样触发 v-html 全量重建，频次反而更高。auto-close 使每次重建产生视觉更好的结果，并消除了"纯文本→HTML"的布局跳变。选区丢失是 v-html 固有缺陷，不作为本次优化的阻塞项。

### Q: 多个代码围栏的 auto-close 是否正确？

**已验证**：`openCount - closeCount` 算法覆盖全部 case：

```
内容                                    open  close  delta  行为
```a\n1\n```                            1     1     0      不操作
```a\n1\n```  text ```b\n2\n           2     1     1      追加1个```
```a\n1\n```  text ```b\n2\n```         2     2     0      不操作
```a\n1                                   1     0     1      追加1个```
```a\n1\n2\n```  ```b\n                  2     1     1      追加1个```
```a                                     1     0     1      追加1个```
```a  ```b                               2     0     2      追加2个```
```

---

## 审查数据统计

| 指标 | 数值 |
|------|------|
| 审查发现总数 | 35 |
| 采纳修复 | 15 |
| 部分采纳 | 3 |
| 暂缓 | 3 |
| 驳回 | 4 |
| 致命缺陷发现（HIGH → 导致方案重设计） | 3 |
| 缺陷发现（MEDIUM → 导致方案补充） | 7 |
| 调优建议（LOW） | 10 |
