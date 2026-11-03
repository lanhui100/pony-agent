/**
 * Wave-61 冻结验收（L2-T）— §2 空值契约 E-1..E-5（jsdom / DOM 存在性层）
 *
 * 契约源（只读）：`.dev-team/contracts/wave-61-status-panel.md` §2（含 Lead 二轮裁决 R1 / R5 修订版）。
 * 用户诉求（契约 §0-3）：空值 / 零值不再显示 `—` / `0`，而是**不渲染**。
 *
 * 判定口径（契约原文）：
 *   - 一律用「位点 DOM 不存在」表达空值（`find(...).exists() === false` / `toHaveCount(0)`），
 *     **禁止**断言 `—`/`0` 文本（E-5 明确禁止占位文本存在）；
 *   - 整行折叠（E-4）与逐位点缺席（E-1/E-2/E-3）分别断言，避免「整行塌陷」掩盖「单位点该缺席却没缺席」。
 *
 * 零假测试律：每个用例都配一个**反向哨兵**（同夹具下其他位点仍在），
 *   以免「整卡/整行塌陷」把「该缺席的位点缺席」变成恒真断言。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import {
  COUNT_SLOT_TESTIDS,
  COUNTS_ROW,
  METRIC_CARD_CLASS,
  RING_SLOT,
  TOKENS_ROW,
  TOKEN_SLOT_TESTIDS,
  allElements,
  collectTooltipTexts,
  mountStatusPanel,
  semanticChildren
} from "./wave61-harness";

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

const countsRowSelector = `[data-testid="${COUNTS_ROW}"]`;
const tokensRowSelector = `[data-testid="${TOKENS_ROW}"]`;
const ringSelector = `[data-testid="${RING_SLOT}"]`;
const slotSelector = (testid: string) => `[data-testid="${testid}"]`;

/** 全空夹具：所有位点的数据源都为空——用于 E-4 的整卡折叠 / E-5 的无占位文本。 */
const ALL_EMPTY_OVERRIDES = {
  turnCount: 0,
  modelCallCount: 0,
  toolCallCount: 0,
  inputTokensTotal: null,
  outputTokensTotal: null,
  cacheHitTokensTotal: null,
  cacheHitRatio: "",
  tokenGenerationSpeed: null,
  contextDisplayTokens: null,
  contextWindowTokens: null
} as const;

function findMetricCard(root: Element): Element | null {
  return allElements(root).find((element) => element.classList.contains(METRIC_CARD_CLASS)) ?? null;
}

/** 面板内所有「位点级」元素的 testid（两行位点全集，用于 E-5 的逐位点文本检查）。 */
const ALL_SLOT_TESTIDS = [...COUNT_SLOT_TESTIDS, ...TOKEN_SLOT_TESTIDS, RING_SLOT];

describe("wave-61 §2 空值契约（E-1..E-5）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("E-1: 计数位为 0 / 非有限值时，该位点整个 DOM 节点不渲染（逐位点隔离验证）", () => {
    // Arrange：只把目标位点置空，其余计数位保持有值 —— 防止「整行塌陷」掩盖单位点缺席。
    const cases = [
      { testid: COUNT_SLOT_TESTIDS[0]!, overrides: { turnCount: 0 }, label: "turnCount=0" },
      { testid: COUNT_SLOT_TESTIDS[1]!, overrides: { modelCallCount: 0 }, label: "modelCallCount=0" },
      { testid: COUNT_SLOT_TESTIDS[2]!, overrides: { toolCallCount: 0 }, label: "toolCallCount=0" },
      { testid: COUNT_SLOT_TESTIDS[0]!, overrides: { turnCount: Number.NaN }, label: "turnCount=NaN" },
      {
        testid: COUNT_SLOT_TESTIDS[1]!,
        overrides: { modelCallCount: Number.POSITIVE_INFINITY },
        label: "modelCallCount=Infinity"
      }
    ] as const;

    for (const testCase of cases) {
      const wrapper = mountStatusPanel(testCase.overrides);

      // Assert（反向哨兵：计数行仍存在，说明不是整行塌陷造成的「缺席」）
      expect(
        wrapper.find(countsRowSelector).exists(),
        `E-1（${testCase.label}）：其余计数位仍有值 ⇒ 计数行必须仍渲染（反向哨兵）`
      ).toBe(true);

      // Assert（契约主体：目标位点整个节点不存在）
      expect(
        wrapper.find(slotSelector(testCase.testid)).exists(),
        `E-1（${testCase.label}）：${testCase.testid} 整个 DOM 节点必须不渲染（禁止渲染 0 占位）`
      ).toBe(false);

      // Assert（其余计数位不受牵连）
      const remaining = COUNT_SLOT_TESTIDS.filter((testid) => testid !== testCase.testid);
      for (const testid of remaining) {
        expect(
          wrapper.find(slotSelector(testid)).exists(),
          `E-1（${testCase.label}）：${testid} 仍有真值，必须继续渲染`
        ).toBe(true);
      }

      wrapper.unmount();
    }
  });

  it("E-2: token 位为 null / 非有限 / ≤0 时不渲染（逐位点隔离验证）", () => {
    // Arrange
    const cases = [
      { testid: TOKEN_SLOT_TESTIDS[0]!, overrides: { inputTokensTotal: null }, label: "input=null" },
      { testid: TOKEN_SLOT_TESTIDS[0]!, overrides: { inputTokensTotal: 0 }, label: "input=0" },
      { testid: TOKEN_SLOT_TESTIDS[0]!, overrides: { inputTokensTotal: -1 }, label: "input=-1" },
      { testid: TOKEN_SLOT_TESTIDS[1]!, overrides: { outputTokensTotal: null }, label: "output=null" },
      { testid: TOKEN_SLOT_TESTIDS[2]!, overrides: { cacheHitTokensTotal: 0 }, label: "cache=0" },
      {
        testid: TOKEN_SLOT_TESTIDS[3]!,
        overrides: { tokenGenerationSpeed: 0 },
        label: "speed=0"
      },
      {
        testid: TOKEN_SLOT_TESTIDS[3]!,
        overrides: { tokenGenerationSpeed: Number.NaN },
        label: "speed=NaN"
      },
      {
        testid: TOKEN_SLOT_TESTIDS[3]!,
        overrides: { tokenGenerationSpeed: -3.5 },
        label: "speed=-3.5"
      }
    ] as const;

    for (const testCase of cases) {
      const wrapper = mountStatusPanel(testCase.overrides);

      // Assert（反向哨兵：token 行仍存在，说明不是整行塌陷造成的「缺席」）
      expect(
        wrapper.find(tokensRowSelector).exists(),
        `E-2（${testCase.label}）：其余 token 位仍有值 ⇒ token 行必须仍渲染（反向哨兵）`
      ).toBe(true);

      // Assert（契约主体）
      expect(
        wrapper.find(slotSelector(testCase.testid)).exists(),
        `E-2（${testCase.label}）：${testCase.testid} 整个 DOM 节点必须不渲染（禁止渲染 0 / — 占位）`
      ).toBe(false);

      // Assert（其余 token 位不受牵连）
      for (const testid of TOKEN_SLOT_TESTIDS.filter((slot) => slot !== testCase.testid)) {
        expect(
          wrapper.find(slotSelector(testid)).exists(),
          `E-2（${testCase.label}）：${testid} 仍有真值，必须继续渲染`
        ).toBe(true);
      }

      wrapper.unmount();
    }
  });

  it("E-2(裁决 R1): 缓存比率尾段不再行内渲染（并入缓存位 tooltip）—— 行内只留图标+值，比率信息不丢失", () => {
    // Arrange 1：缓存 token 有值 + 比率 "12%"。
    const wrapper = mountStatusPanel({ cacheHitRatio: "12%" });

    // Assert（锚点自检：缓存位存在）
    const cacheSlotProbe = wrapper.find(slotSelector(TOKEN_SLOT_TESTIDS[2]!));
    expect(
      cacheSlotProbe.exists(),
      "E-2：cacheHitTokensTotal 有值 ⇒ 缓存位必须渲染"
    ).toBe(true);
    const cacheSlot = cacheSlotProbe;

    // Assert（契约主体 1：尾段移出行内 —— 只剩「图标 + 值」两个**语义**子节点）
    //
    // 用例缺陷修正（2026-11-03，Lead 复核授权「用例自身缺陷重写豁免」）：
    //   缺陷根因：原断言用 `children.length === 2`，但挂载把 `Teleport` 桩成 `teleport: true`，
    //   缓存位内两个 `Tooltip`（图标 + 值）各渲染一个 `TELEPORT-STUB` ⇒ DOM 实得 4，
    //   该断言在任何实现下不可满足（原实现同样如此），属测试环境渲染噪声而非 SUT 缺陷。
    //   修正：只统计非 `teleport-stub` 的语义子节点。原意图（尾段 `· 12%` 不在行内）不弱化：
    //   仍由「语义子节点恰为 2」+「行内无 `·` / `%`」+「比率出现在 tooltip 文案」三重覆盖。
    const cacheChildren = semanticChildren(cacheSlot.element);
    expect(
      cacheChildren.length,
      `E-2（R1）：缓存位行内必须只剩「图标 + 值」两个语义子节点；实际 ${cacheChildren.length} 个：${cacheChildren.map((node) => node.tagName.toLowerCase()).join(" | ")}（已排除 teleport-stub）`
    ).toBe(2);
    expect(cacheSlot.text(), "E-2（R1）：行内不得再出现 `· 12%` 尾段").not.toContain("·");
    expect(cacheSlot.text(), "E-2（R1）：行内不得再出现百分比尾段（实测 86px > 63.5px track）").not.toContain("%");

    // Assert（契约主体 2：比率信息未丢失 —— 并入缓存位 tooltip 文案）
    const tooltipTexts = collectTooltipTexts(wrapper);
    expect(
      tooltipTexts.some((text) => text.includes("12%")),
      `E-2（R1）：缓存比率必须并入 tooltip 文案（不得随尾段一起丢失）；实际 tooltip 文案：${JSON.stringify(tooltipTexts)}`
    ).toBe(true);

    wrapper.unmount();

    // Arrange 2：比率空串 ⇒ 位点仍渲染（不得连带整位消失），且 tooltip 不含比率。
    const emptyRatioWrapper = mountStatusPanel({ cacheHitRatio: "" });

    // Assert（锚点自检）
    const emptyRatioCacheSlotProbe = emptyRatioWrapper.find(slotSelector(TOKEN_SLOT_TESTIDS[2]!));
    expect(
      emptyRatioCacheSlotProbe.exists(),
      "E-2：cacheHitRatio 为空串不得连带缓存位消失（缓存 token 仍有值）"
    ).toBe(true);

    // Assert（契约主体 3：无比率时不渲染比率文案）
    //
    // 用例缺陷修正（2026-11-03，Lead 复核授权「用例自身缺陷重写豁免」）：
    //   缺陷根因：原断言对**全部** tooltip 文案扫 `\d+%`，而圆环 tooltip 天然含
    //   `上下文用量：<1% · …`（契约 §1 L-2 语义）⇒ 该断言与 L-2 直接冲突，任何正确实现都会红。
    //   修正：把百分比检查限定在「缓存位相关 tooltip」（文案含 `缓存读取` 或 `命中率`），
    //   原意图（ratio 为空时不得出现比率文案）保持不变。
    const emptyRatioTooltips = collectTooltipTexts(emptyRatioWrapper);
    const cacheRelatedTooltips = emptyRatioTooltips.filter(
      (text) => text.includes("缓存读取") || text.includes("命中率")
    );
    expect(
      cacheRelatedTooltips.length,
      `E-2：缓存位必须有 tooltip 文案（锚点自检，防止空集上的恒真断言）；实际 tooltip 文案：${JSON.stringify(emptyRatioTooltips)}`
    ).toBeGreaterThan(0);
    expect(
      cacheRelatedTooltips.some((text) => /\d+%/.test(text)),
      `E-2：cacheHitRatio 为空串时缓存位相关 tooltip 不得出现比率百分比；实际 ${JSON.stringify(cacheRelatedTooltips)}`
    ).toBe(false);
    expect(
      emptyRatioTooltips.some((text) => /命中率\s*[\d.]+%/.test(text)),
      `E-2：cacheHitRatio 为空串时不得渲染「命中率 x%」；实际 ${JSON.stringify(emptyRatioTooltips)}`
    ).toBe(false);

    emptyRatioWrapper.unmount();
  });

  it("E-3: contextDisplayTokens 为 null 或 contextWindowTokens 缺失/≤0 时，圆环位不渲染", () => {
    // Arrange
    const cases = [
      { overrides: { contextDisplayTokens: null }, label: "contextDisplayTokens=null" },
      { overrides: { contextWindowTokens: null }, label: "contextWindowTokens=null" },
      { overrides: { contextWindowTokens: 0 }, label: "contextWindowTokens=0" },
      { overrides: { contextWindowTokens: -200_000 }, label: "contextWindowTokens=-200000" }
    ] as const;

    for (const testCase of cases) {
      const wrapper = mountStatusPanel(testCase.overrides);

      // Assert（反向哨兵：计数行仍存在，排除整卡塌陷）
      expect(
        wrapper.find(countsRowSelector).exists(),
        `E-3（${testCase.label}）：计数位仍有值 ⇒ 计数行必须仍渲染（反向哨兵）`
      ).toBe(true);

      // Assert（契约主体）
      expect(
        wrapper.find(ringSelector).exists(),
        `E-3（${testCase.label}）：上下文圆环位必须整个不渲染（§2 E-3）`
      ).toBe(false);

      wrapper.unmount();
    }

    // Assert（正向对照：窗口值合法时圆环必须出现，证明上面四条不是因为「圆环永不渲染」而恒真）
    const positiveWrapper = mountStatusPanel();
    expect(
      positiveWrapper.find(ringSelector).exists(),
      "E-3 正向对照：contextDisplayTokens=9 / contextWindowTokens=200000 ⇒ 圆环位必须渲染"
    ).toBe(true);
    positiveWrapper.unmount();
  });

  it("E-4: 计数行/token 行各自为空时整行折叠；两行皆空且无拦截横幅时外层 section 整段折叠（裁决 R5）", () => {
    // Arrange 1：三个计数位全空 **且圆环无值**，token 位有值（⇒ 计数行可见位点数为 0）。
    //
    // 用例缺陷修正（2026-11-03，Lead 复核授权「用例自身缺陷重写豁免」）：
    //   缺陷根因：本分支原先只置空三个计数位，却仍沿用默认夹具的
    //   `contextDisplayTokens: 9 / contextWindowTokens: 200000` —— 按契约 §1 L-2 圆环是
    //   计数行的**第 4 个位点**，圆环有值时计数行可见位点数 = 1（不是 0），
    //   因此「计数行必须不渲染」与契约语义冲突（SUT 行为正确，是用例夹具自相矛盾）。
    //   修正：本分支同时把圆环数据置空（`contextDisplayTokens: null`），使「可见位点数 = 0」成立。
    //   原意图保持不变：整行折叠只由「可见位点数为 0」触发。
    const countsEmptyWrapper = mountStatusPanel({
      turnCount: 0,
      modelCallCount: 0,
      toolCallCount: 0,
      contextDisplayTokens: null
    });

    // Assert（契约主体 1：可见位点数为 0 ⇒ 整行折叠）
    expect(
      countsEmptyWrapper.find(countsRowSelector).exists(),
      "E-4：计数行可见位点数为 0（三计数位 + 圆环皆无值）⇒ 计数行必须整个不渲染"
    ).toBe(false);
    expect(
      countsEmptyWrapper.find(tokensRowSelector).exists(),
      "E-4：计数行为空不得连带折叠 token 行（token 位仍有值）"
    ).toBe(true);
    countsEmptyWrapper.unmount();

    // Arrange 1b（反向哨兵 · 把语义显式冻结，防止将来被误判为「折叠」）：
    //   三个计数位全 0，但圆环有值 ⇒ 计数行**仍必须渲染**，且行内只剩圆环 1 个位点。
    const ringOnlyWrapper = mountStatusPanel({
      turnCount: 0,
      modelCallCount: 0,
      toolCallCount: 0
    });

    // Assert（反向哨兵 1：计数行必须仍在，可见位点数为 1 而非 0）
    const ringOnlyCountsRow = ringOnlyWrapper.find(countsRowSelector);
    expect(
      ringOnlyCountsRow.exists(),
      "E-4 反向哨兵：计数位全 0 但圆环有值 ⇒ 计数行**必须仍渲染**（可见位点数 1；契约 §1 L-2 圆环是计数行第 4 个位点）"
    ).toBe(true);

    // Assert（反向哨兵 2：行内仅剩圆环位 —— 三个计数位按 E-1 各自不渲染）
    expect(
      ringOnlyCountsRow.findAll('[data-testid^="status-metric-"]'),
      "E-4 反向哨兵：三个计数位为 0 ⇒ 必须各自不渲染（行内不得残留 0 占位）"
    ).toHaveLength(0);
    expect(
      ringOnlyCountsRow.findAll(ringSelector),
      "E-4 反向哨兵：圆环位必须仍在计数行内（唯一位点）"
    ).toHaveLength(1);
    ringOnlyWrapper.unmount();

    // Arrange 2：四个 token 位全空，计数位有值。
    const tokensEmptyWrapper = mountStatusPanel({
      inputTokensTotal: null,
      outputTokensTotal: null,
      cacheHitTokensTotal: null,
      cacheHitRatio: "",
      tokenGenerationSpeed: null
    });
    const tokensEmptyCard = findMetricCard(tokensEmptyWrapper.element);

    // Assert（契约主体 2：token 行不渲染）
    expect(
      tokensEmptyWrapper.find(tokensRowSelector).exists(),
      "E-4：token 行可见位点数为 0 ⇒ token 行必须整个不渲染"
    ).toBe(false);
    expect(
      tokensEmptyWrapper.find(countsRowSelector).exists(),
      "E-4：token 行为空不得连带折叠计数行（计数位仍有值）"
    ).toBe(true);

    // Assert（契约主体 3：token 行自持的 border-t 一并消失）
    expect(tokensEmptyCard, "E-4：计数行仍有值 ⇒ 指标卡必须仍在渲染").not.toBeNull();
    const borderTopCarriers = allElements(tokensEmptyCard!).filter((element) =>
      element.classList.contains("border-t")
    );
    expect(
      borderTopCarriers.map((element) => `${element.tagName.toLowerCase()}.${element.className}`),
      "E-4：token 行折叠时其 `border-t` 分隔线必须一并消失（不得留下悬空分隔线）"
    ).toEqual([]);
    tokensEmptyWrapper.unmount();

    // Arrange 3：两行皆空、且无拦截横幅。
    const allEmptyWrapper = mountStatusPanel(ALL_EMPTY_OVERRIDES);

    // Assert（契约主体 4：卡内容区不渲染，面板头部保留）
    expect(
      findMetricCard(allEmptyWrapper.element),
      `E-4：两行皆空 ⇒ 卡内容区（class "${METRIC_CARD_CLASS}"）必须整个不渲染`
    ).toBeNull();
    expect(
      allEmptyWrapper.find('[data-testid="status-panel-toggle"]').exists(),
      "E-4：两行皆空时面板头部（status-panel-toggle）必须保留"
    ).toBe(true);
    expect(
      allEmptyWrapper.find('[data-testid="status-copy-session-id"]').exists(),
      "E-4：两行皆空时头部复制按钮必须保留（头部不受内容区折叠影响）"
    ).toBe(true);

    // Assert（裁决 R5：外层 `<section class="border-t pt-3 mt-3">` 必须整段折叠 —— 不是保留空卡，
    //   否则冷启动会在头部下方留 1px 分隔线 + 24px 空带）
    const outerSectionCarriers = allElements(allEmptyWrapper.element)
      .filter((element) => element.classList.contains("pt-3") && element.classList.contains("mt-3"))
      .map((element) => `<${element.tagName.toLowerCase()} class="${element.className}">`);
    expect(
      outerSectionCarriers,
      "E-4（R5）：两行皆空时外层 section（pt-3 mt-3 的卡容器段）必须整段不渲染"
    ).toEqual([]);
    const outerBorderTopCarriers = allElements(allEmptyWrapper.element)
      .filter((element) => element.classList.contains("border-t"))
      .map((element) => `<${element.tagName.toLowerCase()} class="${element.className}">`);
    expect(
      outerBorderTopCarriers,
      "E-4（R5）：整段折叠后不得残留 1px 分隔线（border-t）"
    ).toEqual([]);

    allEmptyWrapper.unmount();

    // Arrange 4（裁决 E-4 防连坐）：用量治理拦截横幅在场时，卡容器保留、只折叠两行指标行。
    //
    // 夹具说明（2026-11-03 实测）：`governanceDecision` 不在 runtime store 的 `state()` 声明里
    // （由事件处理器动态赋值）；pinia 的 `$patch(object)` 对**未声明键**静默丢弃 ⇒ 必须直接赋值。
    const blockedStore = useRuntimeStore() as unknown as { governanceDecision: unknown };
    blockedStore.governanceDecision = {
      kind: "Block",
      current: 1.234,
      limit: 5,
      reason: "wave-61 契约用例"
    };
    expect(
      (useRuntimeStore() as unknown as { governanceBlockDetail: unknown }).governanceBlockDetail,
      "E-4：夹具自检 —— 拦截决策必须已生效（否则下方横幅断言会因夹具失效而变成空断言）"
    ).not.toBeNull();
    const blockedWrapper = mountStatusPanel(ALL_EMPTY_OVERRIDES);

    // Assert（契约主体 5：安全/拦截语义优先 —— 横幅必须照常渲染）
    expect(
      blockedWrapper.find('[data-testid="governance-block-banner"]').exists(),
      "E-4：拦截横幅存在时必须照常渲染（安全 UI 不受 E-4 连坐）"
    ).toBe(true);

    // Assert（契约主体 6：此时卡容器保留，但两行指标行仍按 E-4 折叠）
    expect(
      findMetricCard(blockedWrapper.element),
      "E-4：拦截横幅在场 ⇒ 卡容器必须保留（不得因两行皆空而整卡消失）"
    ).not.toBeNull();
    expect(
      blockedWrapper.find(countsRowSelector).exists(),
      "E-4：拦截横幅在场时计数行仍按整行折叠规则不渲染"
    ).toBe(false);
    expect(
      blockedWrapper.find(tokensRowSelector).exists(),
      "E-4：拦截横幅在场时 token 行仍按整行折叠规则不渲染"
    ).toBe(false);

    blockedWrapper.unmount();
  });

  it("E-5: 任何位点不得出现 `—` / `0` / `≈—` 等占位文本（有值态与全空态双向验证）", () => {
    // Arrange 1：有值态（所有位点渲染真值）。
    const valueWrapper = mountStatusPanel();

    // Assert（契约主体：有值态无任何占位符）
    const valueText = valueWrapper.element.textContent ?? "";
    expect(valueText, "E-5：有值态不得出现 `—` 占位文本").not.toContain("—");
    expect(valueText, "E-5：有值态不得出现 `≈—` 估算占位").not.toContain("≈—");

    // Assert（逐位点：不得渲染单独的 0 占位）
    for (const testid of ALL_SLOT_TESTIDS) {
      const slot = valueWrapper.find(slotSelector(testid));
      if (!slot.exists()) {
        continue;
      }
      expect(
        slot.text().trim(),
        `E-5：${testid} 不得以 "0" 作为占位值（有值夹具下该位点应显示真值）`
      ).not.toBe("0");
      expect(slot.text(), `E-5：${testid} 不得渲染 "—" 占位`).not.toContain("—");
    }
    valueWrapper.unmount();

    // Arrange 2：全空态（所有位点都不渲染）。
    const emptyWrapper = mountStatusPanel(ALL_EMPTY_OVERRIDES);

    // Assert（契约主体：全空态不存在任何占位文本 —— 空＝不渲染，而不是空＝`—`）
    const emptyText = emptyWrapper.element.textContent ?? "";
    expect(emptyText, "E-5：全空态不得出现 `—` 占位文本").not.toContain("—");
    expect(
      emptyText,
      `E-5：全空态不得出现任何占位数字（含 0）；实际面板文本：「${emptyText}」`
    ).not.toMatch(/[0-9]/);

    // Assert（结构自检：全空态下位点一个都不剩）
    for (const testid of ALL_SLOT_TESTIDS) {
      expect(
        emptyWrapper.find(slotSelector(testid)).exists(),
        `E-5：全空态下 ${testid} 必须整个不渲染`
      ).toBe(false);
    }

    emptyWrapper.unmount();
  });
});
