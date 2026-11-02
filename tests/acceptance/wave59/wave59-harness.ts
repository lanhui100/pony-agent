/**
 * Wave-59 冻结验收测试共享夹具（**非 spec**，vitest include 只收 `*.spec.ts`）。
 *
 * 契约源（只读）：`.dev-team/contracts/wave-59-desktop-uiux.md`。
 *
 * 设计原则（与实现解耦）：
 * 1. 本文件只提供「可重复的状态注入 + 挂载」能力，不携带任何断言语义；
 * 2. 不使用任何 executor 即将修改/新增的私有符号，只用契约里点名的公开锚点
 *    （data-testid / 契约类名 / store 公开 getter）；
 * 3. 所有等待都走显式条件或可确定的 tick，不写盲 sleep 做竞态判定。
 */
import { defineComponent, h, nextTick } from "vue";
import type { Component } from "vue";
import { vi } from "vitest";
import { mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";

/** ScrollArea 桩：jsdom 无布局，真实 ScrollArea 的可视区尺寸恒为 0，会干扰流式释放节奏。 */
export const ScrollAreaStub = defineComponent({
  setup(_props, { slots }) {
    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

export const ButtonStub = defineComponent({
  props: {
    disabled: { type: Boolean, default: false },
    title: { type: String, default: "" }
  },
  emits: ["click"],
  template:
    '<button class="button-stub" type="button" :disabled="disabled" :title="title" @click="$emit(\'click\')"><slot /></button>'
});

/** motion-v 指令在单测环境未注册；no-op 桩只为消除解析噪声，不参与断言。 */
export const noopMotionDirective = {
  mounted() {
    /* no-op */
  },
  updated() {
    /* no-op */
  },
  beforeUpdate() {
    /* no-op */
  }
};

/**
 * 把被测组件包进 reka-ui TooltipProvider（TooltipRoot 强依赖该 context，否则 setup 抛错）。
 * 与 `tests/acceptance/wave17/tabular-nums-contract.spec.ts` 的 mountWithTooltipProvider 同构。
 */
export function mountWithTooltipProvider(component: Component, props: Record<string, unknown>) {
  const Host = defineComponent({
    name: "Wave59TooltipHost",
    setup() {
      return () => h(TooltipProvider, {}, { default: () => h(component, props) });
    }
  });
  return mount(Host, { global: { stubs: { teleport: true } } });
}

/**
 * 挂载真实 HomeWorkspace（**不桩 MarkdownRenderer**，用于 F3 的 DOM 身份断言）。
 * `pinia` 由调用方创建并注入，保证 store 注入与 mount 使用同一实例。
 */
export function mountWorkspaceWithRealMarkdown(pinia: unknown) {
  return mount(
    {
      render() {
        return h(TooltipProvider, null, { default: () => h(HomeWorkspace) });
      }
    },
    {
      global: {
        plugins: [pinia as never],
        directives: { motion: noopMotionDirective },
        stubs: {
          ScrollArea: ScrollAreaStub,
          Button: ButtonStub,
          Transition: false,
          TransitionGroup: false
        }
      }
    }
  );
}

/** jsdom 缺失的浏览器 API：按 HomeWorkspaceMarkdown.spec.ts 的既有做法确定性打桩。 */
export function installWorkspaceDomStubs() {
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: function scrollIntoViewStub() {
      /* jsdom 无布局，滚动无副作用 */
    }
  });

  // 同步执行 rAF：单测内不需要真实帧节奏，但要保证依赖 rAF 的释放逻辑能推进。
  vi.stubGlobal("requestAnimationFrame", ((callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  }) as typeof requestAnimationFrame);
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
}

/** 确定性推进：N 个宏任务 + nextTick（不依赖固定时长做竞态判定，只用于让异步渲染落地）。 */
export async function settleTicks(rounds = 3, delayMs = 20) {
  for (let index = 0; index < rounds; index += 1) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, delayMs));
    await nextTick();
  }
}

/** markdown 渲染链路（16ms 定时器 + await 解析）落地所需的确定性推进。 */
export async function settleMarkdownRender() {
  await settleTicks(6, 24);
}

/** 一次性 DOM 采样：把「某类节点是否存在」做成可断言的纯数据，避免散落的选择器。 */
export function collectNodes(root: Element, selector: string): Element[] {
  return Array.from(root.querySelectorAll(selector));
}
