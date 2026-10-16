/**
 * Wave-17 冻结契约红相验收（L2-T）— C2：动态数字抖动消除（class 契约层）
 *
 * 契约源（只读）：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 C2-1 / C2-3 / C2-4。
 *
 * 缺陷类：Token / 耗时 / 速度 / 倒计时等数字在同一文本节点**原地变化**，且宿主为
 * `justify-around` / `flex-nowrap` 行，位数变化会横向推移相邻元素。
 *
 * 分层说明（关键）：
 *   jsdom **不执行 Tailwind CSS**（测试环境不注入构建后的样式表），因此本文件只能做
 *   **class 契约层**断言：`classList.contains("tabular-nums")`。
 *   真实的物理断言（`getComputedStyle(el).fontVariantNumeric` + `getBoundingClientRect().width`
 *   位数变化稳定性）在 `tests/e2e/wave17-browser-gate.spec.ts`（C2-2）里用真实 Chromium 测量。
 *
 * 位点定位策略（src 无 testid 时不许改 src）：
 *   用**经实测验证**的结构选择器定位"承载数字的 span"，并**先断言位点数量**——
 *   若模板结构漂移导致选择器落空，用例会因数量断言失败而立刻报警，而不是静默变空断言。
 *   每个位点再断言：其**自身或 ≤2 层祖先**承载 `tabular-nums`。
 *   （2 层上界 = 允许"承载 span 自身"或"该位的直接容器/行容器"两种等价实现，
 *    但不允许把 tabular-nums 撒在整个面板根节点上充数。）
 *
 * 作者：test-agent（L2-T）。红相锚定后本文件冻结只读，任何修改需 Lead 批准。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import type { Component } from "vue";
import { TooltipProvider } from "reka-ui";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";
import HomeTracePanel from "@/components/HomeTracePanel.vue";
import AskPanel from "@/components/AskPanel.vue";
import { useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";
import type { TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

/**
 * `tabular-nums` 允许落在承载 span 自身，或落在其 ≤`maxAncestorHops` 层祖先（位容器/行容器）。
 * 契约 C2-1 的原文是"承载元素的计算样式 font-variant-numeric 必须为 tabular-nums"；
 * 在 jsdom 里该计算样式的唯一可观测代理就是 class，故此处做 class 契约层判定。
 */
function hasTabularNumsAtOrAbove(element: Element, maxAncestorHops = 2): boolean {
  let node: Element | null = element;
  for (let hop = 0; hop <= maxAncestorHops && node != null; hop += 1) {
    if (node.classList.contains("tabular-nums")) {
      return true;
    }
    node = node.parentElement;
  }
  return false;
}

function tabularNumsFailureDetail(element: Element, maxAncestorHops = 2): string {
  const chain: string[] = [];
  let node: Element | null = element;
  for (let hop = 0; hop <= maxAncestorHops && node != null; hop += 1) {
    chain.push(`<${node.tagName.toLowerCase()} class="${node.className}">`);
    node = node.parentElement;
  }
  return chain.join(" ↑ ");
}

/** 把被测组件包进 reka-ui TooltipProvider（TooltipRoot 强依赖该 context，否则 setup 抛错）。 */
function mountWithTooltipProvider(component: Component, props: Record<string, unknown>) {
  const Host = defineComponent({
    name: "Wave17TooltipHost",
    setup() {
      return () => h(TooltipProvider, {}, { default: () => h(component, props) });
    }
  });
  return mount(Host, { global: { stubs: { teleport: true } } });
}

// ── C2-1：HomeStatusPanel 4 个 token 指标位 + 3 个计数位 ────────────────────

describe("wave-17 C2-1：HomeStatusPanel 指标位 tabular-nums class 契约", () => {
  const COUNT_LABELS = ["轮次计数位", "模型调用计数位", "工具调用计数位"] as const;
  const TOKEN_LABELS = ["输入 token 位", "输出 token 位", "缓存 token 位", "生成速度位"] as const;

  function mountStatusPanel() {
    return mountWithTooltipProvider(HomeStatusPanel, {
      sessionId: "session-test",
      turnCount: 9,
      modelCallCount: 12,
      toolCallCount: 345,
      copied: false,
      inputTokensTotal: 9,
      outputTokensTotal: 12,
      cacheHitTokensTotal: 345,
      cacheHitRatio: "12%",
      tokenGenerationSpeed: 42.7,
      showContextUsage: true,
      contextDisplayTokens: 9,
      contextWindowTokens: 200_000
    });
  }

  it("C2-1: 轮次/模型调用/工具调用 三个计数位承载元素必须含 tabular-nums", () => {
    // Arrange
    const wrapper = mountStatusPanel();
    const header = wrapper.get('[data-testid="status-panel-toggle"]');

    // Assert（结构锚点先自检，防止选择器落空造成空断言）
    const countCarriers = header.findAll("span.inline-flex");
    expect(
      countCarriers,
      "status-panel-toggle 行内必须恰好存在 3 个计数承载 span（轮次/模型调用/工具调用）"
    ).toHaveLength(3);

    // Assert（逐位点 class 契约）
    countCarriers.forEach((carrier, index) => {
      const label = COUNT_LABELS[index] ?? `计数位#${index}`;
      expect(
        hasTabularNumsAtOrAbove(carrier.element),
        `${label} 必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：${tabularNumsFailureDetail(carrier.element)}`
      ).toBe(true);
    });

    // Assert（数值语义自检：这三个位点确实渲染了数字）
    expect(countCarriers[0]!.text()).toContain("9");
    expect(countCarriers[1]!.text()).toContain("12");
    expect(countCarriers[2]!.text()).toContain("345");

    wrapper.unmount();
  });

  it("C2-1: 输入/输出/缓存/速度 四个 token 指标位承载元素必须含 tabular-nums", () => {
    // Arrange
    const wrapper = mountStatusPanel();
    const tokenRow = wrapper.get("div.flex.items-center.justify-around");

    // 只取该行的**直接子元素**承载 span（排除更深层的嵌套 span）。
    const tokenCarriers = tokenRow
      .findAll("span.inline-flex")
      .filter((candidate) => candidate.element.parentElement === tokenRow.element);

    // Assert（结构锚点先自检）
    expect(
      tokenCarriers,
      "token 指标行必须恰好存在 4 个直接承载 span（输入/输出/缓存/生成速度）"
    ).toHaveLength(4);

    // Assert（逐位点 class 契约）
    tokenCarriers.forEach((carrier, index) => {
      const label = TOKEN_LABELS[index] ?? `token 位#${index}`;
      expect(
        hasTabularNumsAtOrAbove(carrier.element),
        `${label} 必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：${tabularNumsFailureDetail(carrier.element)}`
      ).toBe(true);
    });

    // Assert（数值语义自检）
    expect(tokenCarriers[0]!.text()).toContain("9");
    expect(tokenCarriers[1]!.text()).toContain("12");
    expect(tokenCarriers[2]!.text()).toContain("345");
    expect(tokenCarriers[3]!.text()).toContain("43 t/s");

    wrapper.unmount();
  });
});

// ── C2-3：HomeTracePanel turn 级与 entry 级指标行 ───────────────────────────

describe("wave-17 C2-3：HomeTracePanel 指标行 tabular-nums class 契约", () => {
  const ENTRY: TraceTimelineEntry = {
    id: "e-1",
    kind: "call_model",
    label: "调用模型",
    state: "completed",
    sequence: 1,
    inputTokens: 1234,
    outputTokens: 567,
    cacheHitInputTokens: 89,
    firstTokenLatencyMs: 321,
    turnDurationMs: 4200,
    durationMs: 4200
  };

  const TURN: TurnTraceRecord = {
    turnId: "turn-1",
    title: "第一轮",
    phase: "completed",
    traceSteps: [],
    traceTimeline: [ENTRY],
    toolActivities: [],
    inputTokens: 1234,
    outputTokens: 567,
    turnDurationMs: 4200,
    firstTokenLatencyMs: 321
  };

  function mountTracePanel() {
    return mountWithTooltipProvider(HomeTracePanel, {
      turns: [TURN],
      sessionId: "session-1",
      copiedKey: "",
      open: true,
      canonicalKind: (kind: TraceTimelineEntry["kind"]) => kind,
      turnTimeline: () => [ENTRY],
      providerReturnedCacheHitInputTokens: () => 89
    });
  }

  beforeEach(() => {
    // jsdom 未实现 Element.prototype.scrollTo；HomeTracePanel 的"滚动到底部"副作用会调用它。
    if (typeof Element.prototype.scrollTo !== "function") {
      Element.prototype.scrollTo = function scrollToStub() {
        /* jsdom 无布局，滚动无副作用 */
      };
    }
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("C2-3: turn 级与 entry 级指标行内全部数字承载 span 必须含 tabular-nums", () => {
    // Arrange
    const wrapper = mountTracePanel();

    // 契约 C2-3 的位点 = 两个指标行（turn 级 + entry 级）。
    const metricRows = wrapper.findAll("div.flex-nowrap.items-center.gap-x-3");
    expect(
      metricRows,
      "HomeTracePanel 必须恰好渲染 2 个指标行（turn 级 + entry 级）"
    ).toHaveLength(2);

    // Assert：每行内 5 个数字承载 span（token / t·s⁻¹ / ms）。
    metricRows.forEach((row, rowIndex) => {
      const carriers = row.findAll("span.inline-flex.whitespace-nowrap");
      expect(
        carriers,
        `指标行#${rowIndex} 必须恰好存在 5 个数字承载 span；实际 ${carriers.length}`
      ).toHaveLength(5);

      carriers.forEach((carrier, index) => {
        const label = `指标行#${rowIndex} 承载位#${index}（文本「${carrier.text()}」）`;
        expect(
          hasTabularNumsAtOrAbove(carrier.element),
          `${label} 必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：${tabularNumsFailureDetail(carrier.element)}`
        ).toBe(true);
      });
    });

    // Assert（数值语义自检：确认这两个行真的渲染了 token / t/s / ms 量纲）
    const allMetricText = metricRows.map((row) => row.text()).join(" | ");
    expect(allMetricText).toMatch(/\bt\/s\b/);
    expect(allMetricText).toMatch(/\bms\b/);

    wrapper.unmount();
  });

  it("C2-3: turn 级与 entry 级的耗时位（ms/s 量纲，契约明列 ms）同样必须含 tabular-nums", () => {
    // Arrange
    const wrapper = mountTracePanel();

    // 耗时位与指标行同属"会原地变化并推移右侧图标/箭头"的数字位点。
    const durationCarriers = wrapper.findAll("span.shrink-0.whitespace-nowrap");
    expect(
      durationCarriers,
      "HomeTracePanel 必须恰好渲染 2 个耗时承载 span（turn 级 + entry 级）"
    ).toHaveLength(2);

    // Assert
    durationCarriers.forEach((carrier, index) => {
      const label = `耗时位#${index}（文本「${carrier.text()}」）`;
      expect(
        hasTabularNumsAtOrAbove(carrier.element),
        `${label} 必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：${tabularNumsFailureDetail(carrier.element)}`
      ).toBe(true);
    });

    // Assert（量纲自检）
    for (const carrier of durationCarriers) {
      expect(carrier.text()).toMatch(/^[\d.]+\s*(s|ms)$/);
    }

    wrapper.unmount();
  });
});

// ── C2-4：AskPanel 过期倒计时位 ─────────────────────────────────────────────

describe("wave-17 C2-4：AskPanel 过期倒计时位 tabular-nums class 契约", () => {
  function createPendingAsk(partial: Partial<PendingAsk> = {}): PendingAsk {
    return {
      requestId: partial.requestId ?? "ask-1",
      requestKind: partial.requestKind ?? "interaction",
      sessionId: partial.sessionId ?? "session-1",
      runId: partial.runId ?? "run-1",
      turnId: partial.turnId ?? "turn-1",
      callId: partial.callId ?? "call-1",
      descriptorSnapshotId: partial.descriptorSnapshotId ?? "snapshot-1",
      descriptorId: partial.descriptorId ?? "builtin:ask_user",
      finalArgsDigest: partial.finalArgsDigest ?? "args-digest",
      policyDigest: partial.policyDigest ?? "policy-digest",
      nonce: partial.nonce ?? "nonce-1",
      version: partial.version ?? 1,
      expiresAtMs: partial.expiresAtMs ?? Date.now() + 30_000,
      state: partial.state ?? "pending",
      prompt: partial.prompt ?? "继续执行？",
      options: partial.options ?? ["是", "否"]
    };
  }

  async function mountAskPanel(ask: PendingAsk) {
    tauriMocks.mockSafeInvoke.mockImplementation((command: string) => {
      if (command === "ask_list_pending") {
        return Promise.resolve([ask]);
      }
      return Promise.resolve(null);
    });
    const wrapper = mount(AskPanel, { global: { stubs: { teleport: true } } });
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
    await vi.waitFor(() => {
      expect(useAskStore().pendingAsks).toHaveLength(1);
    });
    await nextTick();
    return wrapper;
  }

  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("C2-4: formatExpiry 倒计时 span 必须含 tabular-nums（30s 档，逐秒原地变化）", async () => {
    // Arrange：30s 内过期 → 每秒重渲染一次，是抖动最剧烈的档位。
    const ask = createPendingAsk({ requestId: "ask-c2-4", expiresAtMs: Date.now() + 30_000 });

    // Act
    const wrapper = await mountAskPanel(ask);

    // Assert（结构锚点先自检）
    const carriers = wrapper
      .get('[data-testid="ask-card"]')
      .findAll("span.inline-flex");
    expect(
      carriers,
      "ask-card 内必须恰好存在 1 个 inline-flex 承载 span（即 formatExpiry 倒计时位）"
    ).toHaveLength(1);

    // Assert（数值语义自检：确认该位点就是倒计时，而不是别的 span）
    expect(carriers[0]!.text()).toMatch(/\d+\s*(s|m)\s*后过期/);

    // Assert（class 契约）
    expect(
      hasTabularNumsAtOrAbove(carriers[0]!.element),
      `formatExpiry 倒计时位必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：${tabularNumsFailureDetail(carriers[0]!.element)}`
    ).toBe(true);

    wrapper.unmount();
  });

  it("C2-4: 过期态（已过期文案）同一倒计时 span 仍必须含 tabular-nums", async () => {
    // Arrange：已过期档 → 文案切换为「已过期」，但同一 span 仍需等宽数字契约。
    const ask = createPendingAsk({ requestId: "ask-c2-4b", expiresAtMs: Date.now() - 1_000 });

    // Act
    const wrapper = await mountAskPanel(ask);

    // Assert（结构锚点 + 语义）
    const carriers = wrapper.get('[data-testid="ask-card"]').findAll("span.inline-flex");
    expect(carriers).toHaveLength(1);
    expect(carriers[0]!.text()).toContain("已过期");
    expect(
      wrapper.find('[data-testid="ask-expired-hint"]').exists(),
      "过期态必须渲染 ask-expired-hint 行"
    ).toBe(true);

    // Assert（class 契约）
    expect(
      hasTabularNumsAtOrAbove(carriers[0]!.element),
      `过期倒计时位必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：${tabularNumsFailureDetail(carriers[0]!.element)}`
    ).toBe(true);

    wrapper.unmount();
  });
});
