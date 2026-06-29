# Agent 消息 Streaming 渲染优化方案（v2 — 对抗审查修订版）

## 概述

当前 agent 消息的 streaming 渲染管线存在两个体验问题：

1. streaming 过程中未闭合 markdown 边界（尤其是代码块）只能以纯文本呈现，转为 HTML 时发生布局跳变
2. 增量文本的淡入动画阈值过低（`STREAM_FADE_MIN_CHARS = 3`），视觉效果不明显；渲染粒度与后端 chunk 频率耦合

---

## 优化一：Auto-Close 未闭合 Markdown 边界

### 问题描述

`endsWithNaturalBoundary()`（`src/lib/markdown.ts:372`）只在已闭合边界（`\n\n`、已闭合 ``` 等）才触发 markdown 渲染。streaming 中代码块正在生成时，内容以 `unrenderedSuffix` 纯文本 `<span>` 暂存，直到 1500ms/800chars 兜底触发。导致：
- 代码块长时间无语法高亮
- 纯文本→HTML 瞬间布局跳变 + scroll 抖动  
- streaming 中与最终渲染排版不一致

### 方案

**不在 `renderMarkdown()` 内部注入 auto-close 逻辑**（保持纯函数 purity），而是创建独立的 wrapper 函数，由调用方选择是否启用：

```typescript
// src/lib/markdown.ts — 新增，不修改 renderMarkdown()
export function renderPartialMarkdown(partialContent: string): Promise<string> {
  return renderMarkdown(autoCloseBoundaries(partialContent));
}
```

#### `autoCloseBoundaries()` 核心逻辑

```
autoCloseBoundaries() 在 normalizing 之后、marked.parse() 之前执行

输入: 已 normalization 的内容
1. 扫描行首 ` ``` ` / ` ~~~ `，统计 openCount vs closeCount
2. 若 openCount > closeCount，追加 (openCount - closeCount) 个 `\n\`\`\``
   （非奇偶判断，避免多围栏场景误判）
3. 返回补全后的内容
```

#### 三种围栏状态处理

| 围栏状态 | 举例 | auto-close 行为 |
|----------|------|-----------------|
| 未闭合 | ` ```python\ncode\n` → open=1, close=0 | 追加 `\n\`\`\`` |
| 已闭合 | ` ```python\ncode\n``` ` → open=1, close=1 | 不做任何操作 |
| 多围栏 | ` ```a\n1\n``` \n```b\n2\n` → open=2, close=1 | 追加 1 个 `\`\`\`` |
| 过闭合 | ` ```a\n1\n``` \n``` ` → open=2, close=2 | 不做任何操作 |

#### Phase 2（列表/引用/表格）**暂不实现**

对抗审查指出：
- 追加 `\n\n` 会引入可见的空白段落，造成视觉假象
- 列表的中断不像代码 fence 那样视觉上明显，用户可能误以为列表已完结
- 暂留作后续阶段，仅 Phase 1 代码围栏进入本次优化

#### 消息中断/取消时的处理

当 streaming 结束（message status → `"done"` / `"error"` / `"cancelled"`）时，**强制执行一次不含 auto-close 的最终渲染**：

```
turn:completed / turn:failed / turn:cancelled
  → MarkdownRenderer 收到 streaming=false
    → handleContentChange() 执行 scheduleNonStreamingRender()
      → 调用原始 renderMarkdown()，不经过 autoCloseBoundaries()
```

这样，若消息被中断在未闭合围栏处，历史记录中以 raw text 显示（诚实反映不完整状态），而非编造的闭合块。Auto-close 仅 streaming 过程中生效。

### 对抗审查采纳的修复项

| 审查发现 | 严重性 | 处理 |
|----------|--------|------|
| 直接注入 `renderMarkdown()` 破坏纯函数 purity | HIGH | 改为 wrapper `renderPartialMarkdown()` |
| 奇偶计数在多围栏场景误判 | HIGH | 改用 `openCount - closeCount` |
| 消息中断时 auto-close 产生欺骗性内容 | HIGH | 消息终态强制使用原始 `renderMarkdown()` |
| Phase 2 追加 `\n\n` 引入可见假象 | HIGH | 移除 Phase 2，仅 Phase 1 |
| `normalizeMarkdownSource()` 与 auto-close 执行顺序未指定 | MEDIUM | 规定在 normalization 之后、marked.parse() 之前 |
| 反向围栏（`~~~`）未覆盖 | LOW | Phase 1 同步支持 ` ~~~ ` |

### 修改范围

| 文件 | 改动 |
|------|------|
| `src/lib/markdown.ts` | 新增 `autoCloseBoundaries(): string`、`renderPartialMarkdown(): Promise<string>`；**不修改** `renderMarkdown()` |
| `src/components/MarkdownRenderer.vue` | streaming 路径调用 `renderPartialMarkdown()`，非 streaming 路径仍用 `renderMarkdown()` |
| `__tests__/` | 新增边界 case（多围栏、过闭合、inline code 非围栏、中断后内容） |

---

## 优化二：基于 Diff 驱动 + 时间兜底的批次淡入

### 对抗审查揭示的核心问题

原方案提议在 `useStreamingPresentationState` 中新增 `accumulated[messageId]` 映射，对抗审查指出该设计存在 **概念缺陷**：

```
原方案：
  appendedText = content - snapshot
  accumulated[messageId] += appendedText
  snapshot[messageId] = content       ← 每次同步都推进 snapshot
  
  当 accumulated >= threshold：
    fade = accumulated（包含已显示过的内容！→ 文本跳跃）
```

由于 snapshot 在每次同步都推进，而 accumulated 滞后于 snapshot，在 flush 时刻 accumulated 包含此前已作为 stable 显示的内容，导致**文本在 stable 和 fade 之间来回跳跃**。

**修复方案**：移除 `accumulated[messageId]`，改为纯 snapshot-diff 驱动的批次逻辑。**不在 `useStreamingPresentationState` 中引入额外可变状态**。

### 方案

将现有的"时间驱动 flush（32ms）+ 独立 fade 阈值（3 chars）"改为 **"char 阈值 + time 兜底的双重触发"**，且只使用现有的 snapshot diff 机制：

```
syncStreamingPresentationState() 每次调用时：

  pendingChars = nextText.length - snapshotText[messageId].length
  
  if pendingChars >= STREAM_FADE_BATCH_CHARS || timeSinceLastFade > STREAM_FADE_TIME_MS:
    fadeText[messageId] = nextText.slice(snapshotText[messageId].length)  // 整个待渲染段
    snapshotText[messageId] = nextText                                    // 一次性推进
  else:
    fadeText[messageId] = ""                                              // 不输出 fade，全部视为 stable
```

**关键差异**：snapshot 只在 flush 时推进，不在每次同步时推进。不引入 `accumulated` 映射。

### 双重触发阈值

| 触发条件 | 值 | 说明 |
|----------|-----|------|
| `STREAM_FADE_BATCH_CHARS` | 40~60 chars | 字符累积阈值。缓冲区满后整个段以 fade 动画渲染 |
| `STREAM_FADE_TIME_MS` | 350~400ms | 时间兜底。即使 chars 未满，长时间无视觉反馈时强制 flush |

代码围栏内（auto-close 检测到 `openCount > closeCount` 时），将 batch chars 降至 15~20 chars，保证代码行级别的可视化节奏。

### 首段延迟消除

对抗审查指出纯字符阈值导致起始 800ms+ 无任何文本显示。解决方案：

**首段特殊处理**：
- streaming 开始后的首个 fade 段使用 `STREAM_FADE_FIRST_BATCH_CHARS = 8~12` 的低阈值
- 后续段恢复正常阈值（40~60 chars）
- 检测方式：该 message 的 `snapshotText` 首次从空变为非空时即为首段

### 效果对比

| 维度 | 当前 | 优化后 |
|------|------|--------|
| fade 触发机制 | 每次 sync（~32ms/50chars） | pending >= 40~60 chars，或 >= 350~400ms |
| 单次 fade 内容量 | 3~15 chars | 40~60 chars（约 2~4 个后端 chunk） |
| 动画感知度 | 难以察觉的闪烁 | 平滑淡入 |
| 渲染状态管理 | snapshot 每次推进 | snapshot 仅 flush 时推进 |
| 引入额外状态 | 无 | 无（原方案已废弃） |

### 保留的降级机制

- `message.status` → `"done"`：flush 所有剩余内容
- `endsWithNaturalBoundary()` 返回 `true`：强制渲染，不等待 batch 满
- 围栏内窄阈值：自动生效，无需外部信号

---

## 跨优化共同改进

以下问题由对抗审查识别，但不专属某个优化：

### C1. Streaming 进行中指示器

auto-close 使代码块在 streaming 过程中呈现语法高亮 + 完整格式的外观，用户可能误认为消息已完结。

**方案**：在 `HomeWorkspace.vue` 的消息容器外层，通过 `message.status === "pending"` 控制显示一个内置的"流式传输中"脉冲点（使用已有的 `Motion` 动画组件即可，不做额外光标 DOM 元素）。该脉冲点与消息体在语义上属于同一区域，不单独占据布局空间。

### C2. `prefers-reduced-motion` 支持

```css
@media (prefers-reduced-motion: reduce) {
  .assistant-streaming-fade {
    animation: none !important;
    opacity: 1 !important;
  }
}
```

### C3. 可观测性 Metrics

利用现有 `streamDebugState` 机制，新增：

| Metric | 采集点 | 方式 |
|--------|--------|------|
| auto-close 触发率 | `autoCloseBoundaries()` | 每调用计数 |
| batch 实际大小分布 | `syncStreamingPresentationState()` | flush 时记录 chars 数 |
| 渲染间隔 | `MarkdownRenderer.executeRender()` | 两次 render 间的时间差 |
| 布局偏移事件 | `render-complete` + height diff | ResizeObserver 检测高度变化 |
| GPU 耗时（移动端） | 首帧协商 | `performance.now()` 在动画开始时 |

### C4. 异常处理增强

`executeRender()` 增加 try-catch：

```typescript
async function executeRender(version: number) {
  try {
    const html = await renderMarkdown(props.content);
    if (version !== renderVersion) return;
    renderedHtml.value = html;
    // ...
  } catch (err) {
    console.error('[MarkdownRenderer] render failed:', err);
    renderedHtml.value = escapeHtml(props.content); // fallback: 纯文本
  }
}
```

### C5. 运行时 Feature Flag

`localStorage` 可覆盖，无需部署即可回滚：

| Flag | 默认 | 效果 |
|------|------|------|
| `pony-streaming-opt` | `"1"` | 全部启用（当前 build 含此代码时） |
| `pony-streaming-opt=0` | — | 跳过 auto-close，回退到原始 `renderMarkdown()` |
| `pony-streaming-fade-batch-chars` | 50 | 覆盖字符阈值 |
| `pony-streaming-fade-time-ms` | 350 | 覆盖时间阈值 |

### C6. `executeRender` 中的布局变化滚动补偿

当前 `useTimelineAutoScroll.ts` 在 `render-complete` 时调用 `queueScrollToLatestTurn`。每次 auto-close 渲染触发 `render-complete` 时，若内容不是用户主动滚动离底部，应补偿高度变化带来的滚动偏移。

**方案**：在 `executeRender` 中，render 前后测量 `renderedHtml` 父容器的高度差，通过 `scrollBy(0, delta)` 补偿，防止页面因内容高度增长而将已读区域推出视口。

---

## 实现优先级

| 优先级 | 工作项 | 预估工时 | 依赖 |
|--------|--------|----------|------|
| **P0** | 优化一：auto-close（代码围栏）+ wrapper 方式 | 1d | — |
| **P0** | C1：streaming 光标指示器 | 0.5d | — |
| **P1** | 优化二：diff-driven batch fade + 首段低阈值 | 1.5d | 需要 C3 确认基线 |
| **P1** | C2：prefers-reduced-motion | 0.25d | — |
| **P1** | C4：executeRender try-catch | 0.25d | — |
| **P2** | C5：feature flag localStorage | 0.5d | — |
| **P2** | C3：metrics | 1d | 需要 C5 辅助采集 |
| **P2** | C6：render 高度补偿 | 0.5d | — |
| **P3** | 优化一 Phase 2（列表/引用） | 暂缓 | 视 Phase 1 效果决定 |

---

## 验收标准

### 优化一

1. 代码块在 streaming 过程中即以语法高亮呈现，不依赖 1500ms/800chars 兜底
2. 代码块内容持续追加时，渲染区域高度平滑增长，无跳动
3. 消息 streaming 结束后，最终渲染结果与过程中渲染结果一致（无"变一下"的跳变）
4. 消息被中断/取消时，auto-close 不生效，内容如实显示不完整状态
5. 多代码围栏内容渲染正确（不产生冗余 ``` 行）
6. 不含代码围栏的普通内容不受影响

### 优化二

1. streaming 过程中文本以 ~40~60 chars 为一组逐段淡入出现，起始段 8~12 chars
2. 纯字符累积超过 400ms 无 batch 满时，时间兜底触发渲染
3. 动画流畅、不闪烁，`prefers-reduced-motion` 时动画禁用
4. 消息流结束后，所有剩余内容最终完整呈现
5. 代码围栏内批次大小自动降低至 15~20 chars

### 可观测性

1. 可通过 localStorage flag 开关所有优化（零部署回滚）
2. 调试日志可见 auto-close 触发次数、batch 大小分布、渲染间隔

---

## 对抗审查纪要

本次审查由三个独立子智能体分别从 **性能/工程**、**UX/产品**、**架构/可维护性** 角度执行对抗审查，共发现 35 项问题。采纳修复 15 项，部分采纳 3 项，暂缓 3 项，驳回 4 项（附理由）。详细审查报告见 `docs/streaming-rendering-optimization-review.md`。
