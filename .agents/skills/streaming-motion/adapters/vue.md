# Vue 3 胶水范式（Adapter: Vue）

> 本文件是**范式**，不是可发布的库。整套胶水约 40 行，职责只有三件：
> 起 tick、把 `data-pds-*` 写到 DOM、把事件翻译成 L4 reducer 的输入。
> 所有动效参数与语义都在 `tokens.css` / `primitives.css` / `logic/*.ts`，这里不重复。

## 1. 引入资源

```css
/* 全局唯一入口，例如 src/styles.css 顶部 */
@import "../../.agents/skills/streaming-motion/tokens.css";
@import "../../.agents/skills/streaming-motion/primitives.css";
```

## 2. 释放节奏（budget）

```ts
// src/lib/usePdsStreamingReveal.ts
import { computed, onBeforeUnmount, ref, shallowRef, watch, type Ref } from "vue";
import {
  INITIAL_REVEAL_STATE,
  stepReveal,
  visiblePrefix,
  type RevealBudgetConfig,
  type RevealState
} from "../../.agents/skills/streaming-motion/logic/budget";

const TICK_MS = 60;

export function usePdsStreamingReveal(
  content: Ref<string>,
  opts: { streaming: Ref<boolean>; budget?: RevealBudgetConfig; reduceMotion?: () => boolean } = {
    streaming: ref(true)
  }
) {
  const state = shallowRef<RevealState>(INITIAL_REVEAL_STATE);
  const timer = ref<ReturnType<typeof setInterval> | null>(null);

  function tick() {
    state.value = stepReveal(
      state.value,
      { incomingLength: content.value.length, reduceMotion: opts.reduceMotion?.() ?? false },
      opts.budget
    );
  }

  watch(
    () => [content.value.length, opts.streaming.value] as const,
    ([, streaming]) => {
      if (!streaming) {
        // 终态：直接释放全部，避免残留游标截断正文
        state.value = stepReveal(state.value, { incomingLength: content.value.length, reduceMotion: true });
        return;
      }
      tick();
    },
    { flush: "post" } // post：等 DOM 落地后再推进游标，配合 C4（≤1 渲染周期）
  );

  // 停滞兜底：内容不再增长时仍需继续 tick，让 reducer 的 stallTicks 生效
  timer.value = setInterval(tick, TICK_MS);
  onBeforeUnmount(() => {
    if (timer.value) clearInterval(timer.value);
  });

  return { visibleText: computed(() => visiblePrefix(content.value, state.value)), state };
}
```

## 3. 模板侧（唯一需要写的属性）

```vue
<template>
  <!-- 正文承载节点：身份必须稳定，不要 v-if 切换 -->
  <div class="markdown-body">
    <span>{{ stablePrefix }}</span>
    <!-- 尾巴：每 tick 重建，所以只准 data-pds-reveal（opacity + transform） -->
    <span v-if="tail" data-pds-reveal :key="tailKey">{{ tail }}</span>
  </div>
</template>
```

## 4. 视口意图（intent）

```ts
// 只列关键接缝，完整实现参照 src/lib/useTimelineAutoScroll.ts
const handlers = {
  wheel: () => dispatch({ type: "user-gesture", kind: "wheel", atMs: performance.now() }),
  touchstart: () => dispatch({ type: "user-gesture", kind: "touchstart", atMs: performance.now() }),
  pointerdown: () => dispatch({ type: "user-gesture", kind: "pointerdown", atMs: performance.now() })
};

function onContentAppended() {
  const decision = reduceScrollIntent(state, {
    type: "content-appended",
    distanceToBottom: viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight,
    atMs: performance.now()
  });
  state = decision.state;
  if (decision.action === "follow") viewport.scrollTop = viewport.scrollHeight; // 同帧对齐，不要 smooth
}
```

**必做**：忽略自己发起的滚动事件（否则 `viewport-scrolled` 会把自己的滚动当成用户行为，
在容量边界上产生自锁）。用 `programmatic` 标志位或 `isTrusted` 区分。

## 5. 降级

```ts
const prefersReduced = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
```

把 `prefersReduced` 同时喂给两处：`budget` 的 `reduceMotion`（一步释放全部字符）、
以及根节点 `data-pds-reduce-motion="true"`（关闭 CSS 动画）。**两处都要，缺一不可。**
