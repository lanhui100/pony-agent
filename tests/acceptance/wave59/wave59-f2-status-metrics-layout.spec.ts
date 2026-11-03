/**
 * Wave-59 F2 冻结验收：状态卡计数位下移 + 排列（契约 §2，只读源
 * `.dev-team/contracts/wave-59-desktop-uiux.md`）。
 *
 * 分层说明：
 * - 本文件做 **DOM 结构 + class 契约层** 断言（jsdom 不执行 Tailwind）；
 * - 位数变化不推移相邻位点的**物理**断言沿用 wave-17 C2 口径，
 *   在 `tests/e2e/wave17-browser-gate.spec.ts`（C2-2，锚点已迁移至 `status-metrics-counts`）里用真实 Chromium 测量。
 *
 * 零假测试律：每个用例先做「结构锚点自检」（位点数量/文本），再断言契约；
 * 选择器落空会立刻因数量断言失败而报警，不会静默变空断言。
 *
 * ── wave-61 契约 §5 授权迁移（Lead 一票，原意图保真）────────────────────────────
 * 迁移点：token 行定位锚点（本文件常量 `TOKEN_ROW`）。
 *   原意图：token 指标行**恰好 4 个直接承载位**（输入/输出/缓存/生成速度），且该行仍直接位于
 *           `bg-[#f6f0e8]` 指标卡内、位于计数行之下。
 *   迁移理由：wave-61 §1 L-3 以新增 `data-testid="status-metrics-tokens"` 取代 class 选择器
 *           `div.flex.items-center.justify-around`（新契约两行同栅格 `grid grid-cols-4`，
 *           `justify-around` 已不存在，class 锚点必然落空）。
 *   等价断言：F2-2/F2-4 对 token 行的全部断言语义（4 个直接承载位、位于卡内、位于计数行之后）
 *           一字不改，仅把定位从 class 换成契约 testid 锚点。
 *   未迁移：计数行「3 个 `status-metric-` 位点」断言保持原样（wave-61 §5 明确该集合仍为 3）。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mountWithTooltipProvider } from "./wave59-harness";
import HomeStatusPanel from "@/components/HomeStatusPanel.vue";

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

const COUNT_TESTIDS = [
  "status-metric-turn-count",
  "status-metric-model-call-count",
  "status-metric-tool-call-count"
] as const;

/** F2-1：头部行（状态标题 + 复制 Session ID）。 */
const HEADER = '[data-testid="status-panel-toggle"]';
/** F2-2：计数行（本轮新增的锚点）。 */
const COUNTS_ROW = '[data-testid="status-metrics-counts"]';
/**
 * 既有 token 指标行（wave-17 C2-1 / wave-54 C4 口径，结构不得回归）。
 * wave-61 §5 授权迁移：class 选择器 `div.flex.items-center.justify-around` ⇒ testid 锚点
 * `[data-testid="status-metrics-tokens"]`（原意图「token 行恰好 4 个承载位」不变）。
 */
const TOKEN_ROW = '[data-testid="status-metrics-tokens"]';
/** token 指标卡锚点（`#f6f0e8` 卡的 class，用 classList 精确匹配而非 CSS 选择器）。 */
const TOKEN_CARD_CLASS = "bg-[#f6f0e8]";

/**
 * `tabular-nums` 允许落在承载位自身，或落在其 ≤2 层祖先（位容器/行容器）。
 * 与 `tests/acceptance/wave17/tabular-nums-contract.spec.ts` 的判定同口径。
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

/** 计数位语义标签（顺序契约：轮次 → 模型调用 → 工具调用）。 */
const COUNT_LABELS = ["轮次计数位", "模型调用计数位", "工具调用计数位"] as const;

function mountStatusPanel(overrides: Record<string, unknown> = {}) {
  return mountWithTooltipProvider(HomeStatusPanel, {
    sessionId: "session-test",
    turnCount: 9,
    modelCallCount: 12,
    toolCallCount: 345,
    copied: false,
    inputTokensTotal: 900,
    outputTokensTotal: 1200,
    cacheHitTokensTotal: 3450,
    cacheHitRatio: "12%",
    tokenGenerationSpeed: 42.7,
    showContextUsage: true,
    contextDisplayTokens: 9,
    contextWindowTokens: 200_000,
    ...overrides
  });
}

describe("wave-59 F2：状态卡计数位下移 + 排列", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("F2-1: 头部行不得再承载计数位，且必须保留「状态」标题与复制 Session ID 按钮", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（头部锚点自检：头部存在且承载「状态」标题 + 复制按钮）
    const header = wrapper.get(HEADER);
    expect(header.text()).toContain("状态");
    expect(
      header.find('[data-testid="status-copy-session-id"]').exists(),
      "F2-1：复制 Session ID 按钮必须仍留在头部行"
    ).toBe(true);

    // Assert（契约主体：3 个计数承载 span 必须已迁出头部）
    const headerCountCarriers = header.findAll("span.inline-flex");
    expect(
      headerCountCarriers.map((carrier) => carrier.text()),
      'F2-1：头部行内不得再出现计数承载 span.inline-flex（契约要求计数下移，头部仅保留标题+复制按钮）'
    ).toHaveLength(0);

    // Assert（契约主体：三个计数 testid 不得出现在头部子树）
    for (const testid of COUNT_TESTIDS) {
      expect(
        header.find(`[data-testid="${testid}"]`).exists(),
        `F2-1：${testid} 不得出现在 status-panel-toggle 头部行内`
      ).toBe(false);
    }

    wrapper.unmount();
  });

  it("F2-2: 新增 status-metrics-counts 行，位于 token 卡内、token 行之上，顺序为 轮次→模型调用→工具调用", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（锚点自检：token 卡与 token 行均存在）
    const tokenRow = wrapper.get(TOKEN_ROW);
    const tokenCard = tokenRow.element.parentElement;
    expect(
      tokenCard?.classList.contains(TOKEN_CARD_CLASS),
      "F2-2：token 指标行必须仍直接位于 bg-[#f6f0e8] 指标卡内"
    ).toBe(true);

    // Assert（契约主体 1：新增计数行存在）
    const countsRowProbe = wrapper.find(COUNTS_ROW);
    expect(
      countsRowProbe.exists(),
      "F2-2：必须新增 [data-testid=\"status-metrics-counts\"] 计数行（用于承接下移的三个计数位）"
    ).toBe(true);
    const countsRow = wrapper.get(COUNTS_ROW);

    // Assert（契约主体 2：计数行在 token 卡内、且在 token 行之上）
    expect(
      tokenCard!.contains(countsRow.element),
      "F2-2：计数行必须位于 token 指标卡（#f6f0e8 卡）内部"
    ).toBe(true);
    expect(
      Boolean(countsRow.element.compareDocumentPosition(tokenRow.element) & Node.DOCUMENT_POSITION_FOLLOWING),
      "F2-2：计数行必须位于 token 指标行之上（DOM 序在前）"
    ).toBe(true);

    // Assert（契约主体 3：行内恰好 3 个计数位，testid 保留）
    const countSlots = countsRow.findAll('[data-testid^="status-metric-"]');
    expect(
      countSlots.map((slot) => slot.attributes("data-testid")),
      "F2-2：计数行内必须恰好 3 个计数位，且顺序为 轮次 → 模型调用 → 工具调用"
    ).toEqual([...COUNT_TESTIDS]);

    // Assert（数值语义自检：确认三位点分别渲染了轮次/模型调用/工具调用的真实值）
    expect(countSlots[0]!.text()).toContain("9");
    expect(countSlots[1]!.text()).toContain("12");
    expect(countSlots[2]!.text()).toContain("345");

    wrapper.unmount();
  });

  it("F2-3: 三个计数位自身或 ≤2 层祖先必须含 tabular-nums（wave-17 C2 口径的新锚点）", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（锚点自检：新计数行必须已存在，否则 tabular-nums 契约无从承载）
    expect(
      wrapper.find(COUNTS_ROW).exists(),
      "F2-3：status-metrics-counts 计数行必须存在（wave-17 C2 口径的定位锚点）"
    ).toBe(true);
    const countsRow = wrapper.get(COUNTS_ROW);

    // Assert（逐位点 class 契约）
    COUNT_TESTIDS.forEach((testid, index) => {
      const slot = countsRow.get(`[data-testid="${testid}"]`);
      expect(
        hasTabularNumsAtOrAbove(slot.element),
        `${COUNT_LABELS[index]} 必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：${tabularNumsFailureDetail(slot.element)}`
      ).toBe(true);
    });

    // Assert（位数变化语义自检：4 位数仍在同一位点渲染，结构不漂移）
    const wideWrapper = mountStatusPanel({ turnCount: 9999, modelCallCount: 1234, toolCallCount: 5678 });
    const wideCounts = wideWrapper.get(COUNTS_ROW).findAll('[data-testid^="status-metric-"]');
    expect(
      wideCounts.map((slot) => slot.attributes("data-testid")),
      "F2-3：位数变化（4 位数）后计数位数量与顺序不得漂移"
    ).toEqual([...COUNT_TESTIDS]);
    expect(wideCounts[0]!.text()).toContain("9999");
    expect(wideCounts[1]!.text()).toContain("1234");
    expect(wideCounts[2]!.text()).toContain("5678");

    wrapper.unmount();
    wideWrapper.unmount();
  });

  it("F2-4: token 行结构不回归（4 个直接 span.inline-flex），且计数行不得使用 justify-around", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（回归守卫：token 行仍恰好 4 个直接承载 span）
    const tokenRow = wrapper.get(TOKEN_ROW);
    const tokenCarriers = tokenRow
      .findAll("span.inline-flex")
      .filter((candidate) => candidate.element.parentElement === tokenRow.element);
    expect(
      tokenCarriers,
      "F2-4：token 指标行必须仍恰好存在 4 个直接承载 span（输入/输出/缓存/生成速度）"
    ).toHaveLength(4);

    // Assert（契约主体：计数行存在且不使用 justify-around，避免既有选择器歧义）
    expect(
      wrapper.find(COUNTS_ROW).exists(),
      "F2-4：status-metrics-counts 计数行必须存在"
    ).toBe(true);
    const countsRow = wrapper.get(COUNTS_ROW);
    expect(
      countsRow.classes(),
      "F2-4：计数行不得使用 justify-around 类（会与既有 token 行选择器歧义）"
    ).not.toContain("justify-around");

    // Assert（整体仍在同一张卡内）
    const tokenCard = wrapper.get(TOKEN_ROW).element.parentElement;
    expect(
      tokenCard?.classList.contains(TOKEN_CARD_CLASS),
      "F2-4：token 指标行必须仍直接位于 bg-[#f6f0e8] 指标卡内"
    ).toBe(true);
    expect(tokenCard!.contains(countsRow.element)).toBe(true);

    wrapper.unmount();
  });
});
