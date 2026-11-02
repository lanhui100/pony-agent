# React 胶水范式（Adapter: React）

> 本文件是**范式**，不是可发布的库。与 Vue 版的差异只有三处调度接缝，逻辑一行不改。

## 1. 释放节奏（budget）

```tsx
// src/usePdsStreamingReveal.ts
import { useEffect, useMemo, useRef, useState } from "react";
import {
  INITIAL_REVEAL_STATE,
  stepReveal,
  visiblePrefix,
  type RevealBudgetConfig,
  type RevealState
} from "../../.agents/skills/streaming-motion/logic/budget";

const TICK_MS = 60;

export function usePdsStreamingReveal(
  content: string,
  streaming: boolean,
  opts: { budget?: RevealBudgetConfig; reduceMotion?: boolean } = {}
) {
  const [state, setState] = useState<RevealState>(INITIAL_REVEAL_STATE);
  const stateRef = useRef(state);
  stateRef.current = state;

  const advance = (reduceMotion: boolean) => {
    setState((prev) => stepReveal(prev, { incomingLength: content.length, reduceMotion }, opts.budget));
  };

  // 内容增长：post-commit 推进（对应 Vue 的 flush: "post"）
  useEffect(() => {
    if (!streaming) {
      setState((prev) => stepReveal(prev, { incomingLength: content.length, reduceMotion: true }));
      return;
    }
    advance(opts.reduceMotion ?? false);
  }, [content.length, streaming]);

  // 停滞兜底：内容不增长时也要继续 tick，stallTicks 才会生效
  useEffect(() => {
    if (!streaming) return;
    const id = setInterval(() => advance(opts.reduceMotion ?? false), TICK_MS);
    return () => clearInterval(id);
  }, [streaming, content.length]);

  return useMemo(
    () => ({ visibleText: visiblePrefix(content, state), state }),
    [content, state]
  );
}
```

**React 特有的坑**：不要在 render 阶段调用 `stepReveal`。StrictMode 下 render 会双跑，
游标会被推进两次（表现为"偶尔吐字快一倍"）。一切推进都在 effect 或 timer 中。

## 2. 模板侧

```tsx
<div className="markdown-body">
  <span>{stablePrefix}</span>
  {tail ? <span data-pds-reveal key={tailKey}>{tail}</span> : null}
</div>
```

`key` 变化才会重建尾巴节点。若 `key` 恒定，`data-pds-reveal` 的动画**不会重播**——
这正好符合"已渲染内容不再闪"的要求；判断依据是尾巴是否"每次都是新的一段文本"。

## 3. 视口意图（intent）

```tsx
const stateRef = useRef(INITIAL_SCROLL_INTENT_STATE);

useEffect(() => {
  const gesture = () => {
    const decision = reduceScrollIntent(stateRef.current, {
      type: "user-gesture", kind: "wheel", atMs: performance.now()
    });
    stateRef.current = decision.state;
  };
  viewport.addEventListener("wheel", gesture, { passive: true });
  return () => viewport.removeEventListener("wheel", gesture);
}, []);

// 内容追加时
function onAppended() {
  const decision = reduceScrollIntent(stateRef.current, {
    type: "content-appended",
    distanceToBottom: viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight,
    atMs: performance.now()
  });
  stateRef.current = decision.state;
  if (decision.action === "follow") viewport.scrollTop = viewport.scrollHeight; // 同帧对齐
}
```

**必做**：忽略自己发起的滚动。React 里常见做法是一个 `programmaticRef`：
写入 `scrollTop` 前置 `true`，在 scroll handler 里读到 `true` 时跳过并复位。

## 4. 与 `motion` 类库的分工

不需要引入动画库即可完成 PDS-SM 的全部原语 —— 这是刻意的：

- **CSS 能表达的部分**（入场/展开/降级）交给 `primitives.css`，它们走合成器且自动响应系统偏好；
- **动画库只适合**手势拖拽回弹、共享元素过渡这类需要速度/位置连续性的场景；
- 若已引入动画库，**不要**用它做流式尾巴的逐 tick 动画：每 tick 一次 JS 驱动的 tween 会与
  content 更新争主线程，这正是上游 1.x 被重构掉的原因。

## 5. 降级

```tsx
const reduceMotion = useReducedMotion(); // 或 window.matchMedia 自实现
<div data-pds-reduce-motion={reduceMotion ? "true" : undefined}>…</div>
```

同样两处都要喂：`budget` 的 `reduceMotion` + 根节点的 `data-pds-reduce-motion`。
