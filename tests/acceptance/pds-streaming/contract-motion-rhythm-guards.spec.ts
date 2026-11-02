/**
 * wave-pds-3 · 契约 §3 / §4 / §5 的**附加护栏**（不属于 §6 的 V1–V10 表项）
 *
 * 契约源（只读）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`
 *   §3.1 尾巴遮罩 = 静态 mask-image 渐变（不是 animation）；
 *   §3.2 reasoning 尾巴：350ms opacity ⇒ 140ms + translateY(2px)→0，ease-out；
 *   §3.3 工具行/面板入场：`item-smooth-appear` 加 `filter: blur(2px)`、时长 220ms；工具行需补同类；
 *   §3.4 状态行：原位替换加 rise（240ms）/ fall（180ms），label 不重建；
 *   §4 节奏（**v2**）：`STREAMING_PRESENTATION_TICK_MS` 60 → 33；`charsPerTick` **保持 4 不动**；
 *   §5 NFR：新增定时器/监听器泄漏 = 0（卸载后归零）。
 *
 * 为什么单列一个文件：V1–V10 是验收表，§3/§4 的「时长/位移/节拍」是**实现规格**。
 * 把规格写成用例可以避免"看起来动了但其实没改数"的交付，同时让实现偏差在红相就暴露。
 *
 * v1 → v2 的裁定（Lead）：撤销 `charsPerTick 4 → 2`，**只改 tick**。R2 因此从「断言 2」改为
 *   「**同时锁两件事**」：tick === 33 且 `charsPerTick === 4` —— 后者是**锁住"不改它"这个决定**，
 *   防止后人以"更平滑"为名误改后打红冻结基线（`tests/useStreamingPresentationState.spec.ts` 14 例
 *   与 `tests/pds-integration-parity.spec.ts` 黄金表都以「每 tick 4 字」为真值）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { computed, ref } from "vue";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import MarkdownRenderer from "@/components/MarkdownRenderer.vue";
import AgentRuntimeStatusBar from "@/components/chat/AgentRuntimeStatusBar.vue";
import { useStreamingPresentationState } from "@/lib/useStreamingPresentationState";
import { useRuntimeStore } from "@/stores/runtime";
import { DEFAULT_REVEAL_BUDGET } from "../../../.agents/skills/streaming-motion/logic/budget";
import type { ChatMessage } from "@/types/runtime";
import {
  collectCssPool,
  extractRulesMatching,
  extractTailRules,
  installDomStubs,
  mountWorkspace,
  TOOL_ROW_SELECTOR,
  readRepoFile,
  readStyleText,
  stripCssComments,
  stripJsComments
} from "./pds-streaming-harness";

function createAssistantMessage(content: string, reasoningContent: string): ChatMessage {
  return {
    id: "assistant-r",
    turnId: "turn-r",
    role: "assistant",
    content,
    status: "pending",
    tokenCount: null,
    reasoningContent,
    modelName: "OpenAI/GPT-5",
    toolName: null,
    detail: null,
    durationSeconds: null,
    errorDetail: null
  };
}

/** 从 CSS 池里找出「声明了 animation」且选择器含某个 class 的规则。 */
function cssRulesForClass(className: string) {
  return collectCssPool().flatMap((entry) =>
    extractRulesMatching(entry.css, `.${className}`)
      .filter((rule) => /(?:^|;)\s*animation\s*:/.test(rule.body) && !/animation\s*:\s*none/.test(rule.body))
      .map((rule) => ({ path: entry.path, selector: rule.selector, body: rule.body }))
  );
}

/** 元素上是否挂了「有入场动画的类」。 */
function animatedClassesOf(element: Element | null): string[] {
  if (!element) return [];
  const classes = Array.from(element.classList);
  return classes.filter((className) => cssRulesForClass(className).length > 0);
}

describe("wave-pds-3 附加护栏 · §4 释放节奏", () => {
  it("R1: HomeWorkspace.STREAMING_PRESENTATION_TICK_MS 必须是 33（现状 60）", () => {
    // Arrange
    const code = stripJsComments(readRepoFile("src/components/HomeWorkspace.vue"));

    // Act
    const match = code.match(/STREAMING_PRESENTATION_TICK_MS\s*=\s*(\d+)/);

    // Assert
    expect(match, "R1：HomeWorkspace.vue 必须存在 STREAMING_PRESENTATION_TICK_MS 常量").not.toBeNull();
    expect(
      Number(match?.[1]),
      "R1：契约 §4 要求释放节拍 60ms → 33ms（4 字/60ms = 16.7Hz 的 4 字阶梯）"
    ).toBe(33);
  });

  it("R2: 节奏双锁 —— tick === 33 且 charsPerTick === 4（后者锁「不改它」的决定）", () => {
    // Arrange
    const code = stripJsComments(readRepoFile("src/components/HomeWorkspace.vue"));
    const tickMatch = code.match(/STREAMING_PRESENTATION_TICK_MS\s*=\s*(\d+)/);

    // Assert ①：tick 60 → 33（本轮唯一要改的节奏参数）
    expect(
      Number(tickMatch?.[1]),
      "R2 ①：契约 v2 §4 要求 STREAMING_PRESENTATION_TICK_MS = 33（平滑来自更高 tick 频率 + 新字符落在渐隐区）"
    ).toBe(33);

    // Assert ②：charsPerTick 必须保持 4 —— 本波**明确不改**它（v2 撤销了 4→2）
    //   这是"锁住不变量"的护栏：14 例冻结基线 + parity 黄金表都以 4 字/tick 为真值，
    //   任何"顺手改成 2 更平滑"的改法都会打红 V10。
    expect(
      DEFAULT_REVEAL_BUDGET.charsPerTick,
      "R2 ②：契约 v2 §4 明确 charsPerTick 保持 4（DEFAULT_REVEAL_BUDGET 不动）；"
        + "改动它会打红 V10 的 14 例冻结基线与 pds-integration-parity 黄金表"
    ).toBe(4);

    // Assert ③：确定性护栏 —— 值必须是可被冻结断言消费的整数（防止被改成 2.5 / NaN 之类的"更平滑"写法）
    expect(
      Number.isInteger(DEFAULT_REVEAL_BUDGET.charsPerTick) && DEFAULT_REVEAL_BUDGET.charsPerTick > 0,
      `R2 ③：charsPerTick 必须是正整数，实际 ${DEFAULT_REVEAL_BUDGET.charsPerTick}`
    ).toBe(true);
  });
});

describe("wave-pds-3 附加护栏 · §3.2 reasoning 尾巴动效", () => {
  it("R3: reasoning 尾巴淡入必须是 140ms + translateY(2px) + ease-out（现状 350ms 纯透明度）", () => {
    // Arrange
    const messages = ref<ChatMessage[]>([createAssistantMessage("正文", "思考片段一")]);
    const state = useStreamingPresentationState(computed(() => messages.value));
    state.syncStreamingPresentationState();
    state.syncStreamingPresentationState();

    // Act
    const style = state.assistantDisplayedReasoningFadeStyle(messages.value[0]!) ?? {};
    const styleText = JSON.stringify(style);

    // Assert ①：时长 140ms
    expect(
      styleText,
      `R3：reasoning 尾巴动画时长必须为 140ms（契约 §3.2）；实际 style=${styleText}`
    ).toContain("140ms");

    // Assert ②：必须带 2px 位移语义（要么在 style 里，要么在 CSS keyframes 里）
    const pool = collectCssPool();
    const animationName = String((style as Record<string, unknown>).animationName ?? "");
    const keyframeRules = pool.flatMap((entry) => {
      const match = entry.css.match(/@keyframes\s+([\w-]+)\s*\{([\s\S]*?)\n\}/);
      return match ? [{ name: match[1] ?? "", body: match[2] ?? "", path: entry.path }] : [];
    });
    const hasTranslateSemantics = styleText.includes("translateY(2px)")
      || styleText.includes("translateY(2px)→0")
      || keyframeRules.some((rule) => rule.name === animationName && /translateY\(\s*2px\s*\)/.test(rule.body));

    expect(
      hasTranslateSemantics,
      `R3：reasoning 尾巴必须带 translateY(2px)→0（契约 §3.2）；animationName=${JSON.stringify(animationName)}，`
        + `实际 style=${styleText}`
    ).toBe(true);

    // Assert ③：ease-out
    expect(styleText, "R3：缓动必须是 ease-out（契约 §3.2）").toContain("ease-out");
  });
});

describe("wave-pds-3 附加护栏 · §3.3 工具行 / 工具面板入场", () => {
  it("R4: item-smooth-appear 必须带 filter: blur(2px) 且时长 220ms（现状无 blur、200ms）", () => {
    // Arrange
    const css = stripCssComments(readStyleText("src/components/chat/WorkspaceTurnItem.vue"));

    // Act
    const keyframes = css.match(/@keyframes\s+item-smooth-appear\s*\{([\s\S]*?)\n\}/);
    const enterRule = extractRulesMatching(css, ".transition-item-enter")
      .find((rule) => /animation\s*:\s*item-smooth-appear/.test(rule.body));

    // Assert ①：关键帧存在且含 blur(2px)
    expect(keyframes, "R4：WorkspaceTurnItem.vue 必须存在 item-smooth-appear 关键帧").not.toBeNull();
    expect(
      keyframes?.[1] ?? "",
      "R4：契约 §3.3 要求 item-smooth-appear 增加 filter: blur(2px)→0"
    ).toMatch(/blur\(\s*2px\s*\)/);

    // Assert ②：时长 220ms
    expect(enterRule, "R4：必须存在 `.transition-item-enter { animation: item-smooth-appear ... }` 规则").toBeDefined();
    expect(
      enterRule?.body ?? "",
      `R4：入场时长必须为 220ms（契约 §3.3）；实际规则：${JSON.stringify(enterRule)}`
    ).toContain("220ms");
  });

  it("R5: 工具行必须带 data-pds-tool-row 锚点并挂载入场动画类（契约 v2 §1 + §3.3）", async () => {
    // Arrange：直接挂载 WorkspaceTurnItem 的成本高，这里用生产路径的最小工具态（role=tool 消息）
    const pinia = createPinia();
    setActivePinia(pinia);
    installDomStubs();
    const store = useRuntimeStore();
    await store.$patch({
      sessionOperation: null,
      phase: "ready",
      error: null,
      messages: [
        {
          id: "tool-row",
          turnId: "turn-row",
          role: "tool",
          content: "",
          status: "done",
          tokenCount: 3,
          toolName: "Search",
          canonicalToolName: "Search",
          displayNameZh: "搜索",
          detail: "搜索：pony",
          durationSeconds: 0.2
        },
        createAssistantMessage("答案", "")
      ]
    });
    const wrapper = mountWorkspace(pinia);
    await nextTick();
    await nextTick();

    // Act
    const panel = wrapper.element.querySelector(".conversation-tool-panel");
    expect(panel, "R5 自检：必须渲染出 .conversation-tool-panel（工具面板）").not.toBeNull();

    // Assert ①：工具面板本身有入场动画（§3.3）
    expect(
      animatedClassesOf(panel).length,
      "R5：工具面板必须挂载入场动画类（契约 §3.3）"
    ).toBeGreaterThan(0);

    // Assert ②（契约 v2 §1）：工具行必须带 data-pds-tool-row 锚点
    const rows = Array.from(wrapper.element.querySelectorAll(TOOL_ROW_SELECTOR));
    expect(
      rows.length,
      `R5：工具行必须带 ${TOOL_ROW_SELECTOR} 锚点（契约 v2 §1 新增必需属性）；`
        + `实际工具面板内的行：${JSON.stringify(
          Array.from((panel as Element).children).map((row) => Array.from(row.classList))
        )}`
    ).toBeGreaterThan(0);

    // Assert ③（§3.3）：每一条工具行都必须有入场动画
    const rowViolations = rows
      .map((row, index) => ({
        index,
        classes: Array.from(row.classList),
        animated: animatedClassesOf(row)
      }))
      .filter((entry) => entry.animated.length === 0);
    expect(
      rowViolations,
      `R5：以下工具行没有任何入场动画类（契约 §3.3 要求补同类入场）：${JSON.stringify(rowViolations)}`
    ).toHaveLength(0);

    wrapper.unmount();
  });
});

describe("wave-pds-3 附加护栏 · §3.4 状态行交接", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("R6: 状态行 label 原位替换且带 rise/fall 动画（240ms / 180ms）", async () => {
    // Arrange
    const wrapper = mount(AgentRuntimeStatusBar, {
      props: { status: { mode: "thinking", label: "思考中", startTime: Date.now() } }
    });
    const labelBefore = wrapper.find('[data-testid="agent-runtime-status-label"]');
    expect(labelBefore.exists(), "R6 自检：必须存在状态行 label 节点").toBe(true);
    const labelElement = labelBefore.element;

    // Act：切换状态（原位替换）
    await wrapper.setProps({ status: { mode: "responding", label: "生成中", startTime: Date.now() } });
    await nextTick();

    // Assert ①：label 节点不得被重建（契约 §3.4「保持 label 原位」）
    const labelAfter = wrapper.find('[data-testid="agent-runtime-status-label"]');
    expect(
      labelAfter.element === labelElement,
      "R6：状态切换不得重建 label 节点（原位替换）"
    ).toBe(true);

    // Assert ②：状态行**自身**必须留下 rise 动画证据
    //   判据刻意收窄到「组件自己的 <style> 或节点属性」，不接受"仓库里某处存在 rise 规则"：
    //   primitives.css 里本来就有 `[data-pds-status="enter"]`，但**从未被 import** —— 用它判绿就是假绿。
    const COMPONENT = "src/components/chat/AgentRuntimeStatusBar.vue";
    const componentCss = stripCssComments(readStyleText(COMPONENT));
    const statusAttr = labelElement.getAttribute("data-pds-status")
      ?? labelElement.parentElement?.getAttribute("data-pds-status");
    const riseRules = extractRulesMatching(componentCss, "rise")
      .concat(extractRulesMatching(componentCss, "status-enter"))
      .filter((rule) => /animation\s*:/.test(rule.body));
    const fallRules = extractRulesMatching(componentCss, "fall")
      .filter((rule) => /animation\s*:/.test(rule.body));

    // Assert ②a：节点属性协议（若采用 primitives 的 data-pds-status 方言）
    if (statusAttr === null) {
      // Assert ②b：否则必须由组件自身定义 rise 动画，且 label 挂着该动画
      const animatedOnLabel = animatedClassesOf(labelElement).filter((className) =>
        /rise|status/i.test(className)
      );
      expect(
        { riseRules, animatedOnLabel },
        `R6：状态切换必须留下 rise 证据 —— 要么 label 挂 data-pds-status="enter"，`
          + `要么 ${COMPONENT} 内定义 rise 动画并挂到 label 上；`
          + `实际 statusAttr=null、label classes=${JSON.stringify(Array.from(labelElement.classList))}、`
          + `组件内 rise 规则=${JSON.stringify(riseRules)}`
      ).not.toEqual({ riseRules: [], animatedOnLabel: [] });
    }

    // Assert ③：降级/退场也必须被组件自身覆盖（fall，180ms）
    expect(
      fallRules,
      `R6：${COMPONENT} 内必须存在 fall（退场，180ms）动画规则（契约 §3.4）；实际：${JSON.stringify(fallRules)}`
    ).not.toHaveLength(0);

    wrapper.unmount();
  });
});

describe("wave-pds-3 附加护栏 · §3.1 尾巴遮罩 / §5 定时器泄漏", () => {
  beforeEach(() => {
    window.localStorage.clear();
    installDomStubs();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("R7: 尾巴必须用静态 mask-image 渐隐实现浮现（契约 §3.1：不是 animation）", () => {
    // Arrange
    const pool = collectCssPool();

    // Act
    const maskRules = pool.flatMap((entry) =>
      extractTailRules(entry.css)
        .filter((rule) => /mask-image\s*:\s*linear-gradient/.test(rule.body))
        .map((rule) => ({ path: entry.path, body: rule.body }))
    );
    // 反向：尾巴节点不得靠 animation 做逐字浮现（那是 RC2 的闪跳来源，D-59-2 明确禁止）
    const animatedTailRules = pool.flatMap((entry) =>
      extractTailRules(entry.css)
        .filter((rule) => /animation\s*:/.test(rule.body))
        .map((rule) => ({ path: entry.path, body: rule.body }))
    );

    // Assert
    expect(
      maskRules,
      "R7：必须存在 `[data-stream-tail] { mask-image: linear-gradient(...) }`（契约 §3.1 的静态遮罩）"
    ).not.toHaveLength(0);
    expect(
      animatedTailRules,
      `R7：尾巴不得用 animation 实现浮现（契约 §3.1 明确不是 animation）：${JSON.stringify(animatedTailRules)}`
    ).toHaveLength(0);
  });

  it("R8: MarkdownRenderer 卸载后不得遗留定时器（§5 NFR：新增定时器泄漏 = 0）", async () => {
    // Arrange
    vi.useFakeTimers();
    const wrapper = mount(MarkdownRenderer, {
      props: { content: "## 标题\n\n流式正文", streaming: true, forceMarkdownStreaming: true }
    });
    // Act：让渲染排程落地（16ms 节流 + async 解析）
    await vi.advanceTimersByTimeAsync(40);
    await nextTick();
    wrapper.unmount();

    // Assert
    expect(
      vi.getTimerCount(),
      "R8：MarkdownRenderer 卸载后必须清理全部定时器（§5：新增定时器泄漏 = 0）"
    ).toBe(0);

    vi.useRealTimers();
  });
});
