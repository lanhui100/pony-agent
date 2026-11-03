/**
 * Wave-61 冻结验收（L2-T）— §1 布局契约 L-1..L-5（jsdom / class + DOM 结构层）
 *
 * 契约源（只读）：`.dev-team/contracts/wave-61-status-panel.md` §1（含 Lead 二轮裁决 R1/R4/R5 修订版）。
 * 用户诉求（契约 §0）：上下文圆环迁入右侧状态面板第一行、与计数同行；图标↔值亲密性可读。
 *
 * 分层说明（关键）：
 *   jsdom 不执行 Tailwind，本文件只做 **DOM 结构 + class 契约层** 断言；
 *   列中心物理对齐（≤0.5px）、位点左边缘漂移（wave-17 C2-2c）由
 *   `tests/e2e/wave61-status-panel.spec.ts`（G-4c）与 `tests/e2e/wave17-browser-gate.spec.ts` 用真实 Chromium 测量。
 *
 * 零假测试律：每个用例先做「锚点存在性/位点数量」自检，再断言契约主体；
 *   选择器落空会立刻因存在性/数量断言失败而报警，不会静默退化成空断言。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import {
  COUNT_SLOT_TESTIDS,
  COUNTS_ROW,
  METRIC_CARD_CLASS,
  RING_SLOT,
  SLOT_MIN_WIDTH,
  TOKENS_ROW,
  TOKEN_SLOT_TESTIDS,
  SLOT_MIN_WIDTH_CEILING_REM,
  allElements,
  collectTooltipTexts,
  mountStatusPanel
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

/** 计数行内「位点级」元素的 testid 序列（计数三位 + 圆环位，按 DOM 文档序）。 */
const COUNT_ROW_SLOT_ORDER = [...COUNT_SLOT_TESTIDS, RING_SLOT];

/** 位点容器必须自持的 class（§1 L-4）。 */
const SLOT_CONTAINER_CLASSES = ["inline-flex", "items-center", "justify-center", "gap-1.5"];

/** 取元素子树内「带宽度预留 / 右对齐」的违规后代（§1 L-4：预留只允许落在位点容器自身）。 */
function widthReservationViolations(element: Element): string[] {
  return Array.from(element.querySelectorAll("*"))
    .map((node) => {
      const classes = Array.from(node.classList);
      const offending = classes.filter((token) => token.startsWith("min-w-") || token === "text-right");
      return offending.length > 0
        ? `<${node.tagName.toLowerCase()} class="${node.className}"> 携带 ${offending.join(" / ")}`
        : "";
    })
    .filter((entry) => entry.length > 0);
}

/** 取指标卡元素（class token 含 `[`/`#`/`]`，不能用 CSS 选择器安全表达）。 */
function findMetricCard(root: Element): Element | null {
  return allElements(root).find((element) => element.classList.contains(METRIC_CARD_CLASS)) ?? null;
}

describe("wave-61 §1 布局契约（L-1..L-5）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("L-1: 上下文圆环的唯一渲染位置 = status-metrics-counts 行内，且面板内恰好 1 处", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（锚点自检：计数行必须存在）
    const countsRowProbe = wrapper.find(countsRowSelector);
    expect(
      countsRowProbe.exists(),
      `L-1：计数行 [data-testid="${COUNTS_ROW}"] 必须存在（圆环唯一合法归属地）`
    ).toBe(true);
    const countsRow = wrapper.get(countsRowSelector);

    // Assert（契约主体 1：圆环位在计数行子树内）
    const ring = countsRow.find(ringSelector);
    expect(
      ring.exists(),
      `L-1：圆环位 [data-testid="${RING_SLOT}"] 必须位于计数行内（契约 §1 L-1/L-2）`
    ).toBe(true);

    // Assert（契约主体 2：全面板唯一 —— 不得在头部行/token 行重复渲染）
    expect(
      wrapper.findAll(ringSelector),
      "L-1：圆环位在整个状态面板内必须恰好 1 处（不得重复渲染）"
    ).toHaveLength(1);
    expect(
      wrapper.get('[data-testid="status-panel-toggle"]').find(ringSelector).exists(),
      "L-1：圆环不得出现在 status-panel-toggle 头部行"
    ).toBe(false);

    // Assert（物理形态：圆环仍是 SVG 双 circle（轨道 + 进度），进度环携带 stroke-dashoffset）
    const circles = ring.findAll("circle");
    expect(
      circles,
      "L-1：圆环必须是 SVG 双 circle（轨道 + 进度），不得改用无法测 dashoffset 的实现"
    ).toHaveLength(2);
    expect(
      circles[1]!.attributes("stroke-dashoffset"),
      "L-1：进度 circle 必须携带 stroke-dashoffset（browser-smoke 的 dashoffset 口径随之迁移）"
    ).toBeDefined();

    wrapper.unmount();
  });

  it("L-2: 计数行 4 位点顺序固定为 轮次→模型调用→工具调用→圆环；圆环 testid 不得带 status-metric- 前缀", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（锚点自检）
    expect(wrapper.find(countsRowSelector).exists(), `L-2：计数行必须存在`).toBe(true);
    const countsRow = wrapper.get(countsRowSelector);

    // Assert（契约主体 1：4 位点顺序 = 契约原文顺序）
    const slots = countsRow.findAll(`[data-testid^="status-metric-"], ${ringSelector}`);
    expect(
      slots.map((slot) => slot.attributes("data-testid")),
      "L-2：计数行位点顺序必须为 轮次 → 模型调用 → 工具调用 → 上下文圆环"
    ).toEqual(COUNT_ROW_SLOT_ORDER);

    // Assert（契约主体 2：圆环 testid 命名独立性 —— 既有 3 位计数断言语义不得被污染）
    expect(
      RING_SLOT.startsWith("status-metric-"),
      "L-2：圆环 testid 不得使用 status-metric- 前缀（契约原文；用于隔离既有计数断言语义）"
    ).toBe(false);
    const countSlotTestids = countsRow
      .findAll('[data-testid^="status-metric-"]')
      .map((slot) => slot.attributes("data-testid"));
    expect(
      countSlotTestids,
      "L-2：带 status-metric- 前缀的位点集合必须仍恰好是既有 3 个计数位（圆环不得偷用该前缀）"
    ).toEqual([...COUNT_SLOT_TESTIDS]);

    // Assert（数值语义自检：确认 3 个计数位确实渲染了夹具真值，而非空壳位点）
    expect(countsRow.get(`[data-testid="${COUNT_SLOT_TESTIDS[0]}"]`).text()).toContain("9");
    expect(countsRow.get(`[data-testid="${COUNT_SLOT_TESTIDS[1]}"]`).text()).toContain("12");
    expect(countsRow.get(`[data-testid="${COUNT_SLOT_TESTIDS[2]}"]`).text()).toContain("345");

    wrapper.unmount();
  });

  it("L-2(圆环语义): 百分比 clamp(round(tokens/window*100),1,100) + data-context-debug 字面量 + aria-label + tooltip 文案", () => {
    // Arrange：debug 属性由 `pony-agent.debug.runtime-logs=true` 开启（与 production 门控同源）。
    const DEBUG_KEY = "pony-agent.debug.runtime-logs";
    const EVIDENCE_KEY_BACKUP = window.localStorage.getItem(DEBUG_KEY);

    const readRing = (wrapper: ReturnType<typeof mountStatusPanel>) => {
      const ring = wrapper.get(ringSelector);
      const circles = ring.findAll("circle");
      const dashOffset = Number(circles[1]!.attributes("stroke-dashoffset"));
      return { ring, dashOffset };
    };
    const expectedOffset = (pct: number) => 2 * Math.PI * 7.5 * (1 - pct / 100);

    try {
      window.localStorage.setItem(DEBUG_KEY, "true");

      // ── 帧 1：比例 < 0.5% ⇒ round=0 ⇒ clamp 下限 1%（9 / 200000） ────────────
      const lowerWrapper = mountStatusPanel({ contextDisplayTokens: 9, contextWindowTokens: 200_000 });
      expect(lowerWrapper.find(ringSelector).exists(), "L-2：圆环位必须存在（有值夹具）").toBe(true);

      // Assert（aria-label：可及名与 composer 原实现逐字保真）
      const lowerRing = lowerWrapper.get(ringSelector);
      const ariaCarrier =
        lowerRing.attributes("aria-label") === "上下文用量"
          ? lowerRing
          : lowerRing.findAll('[aria-label="上下文用量"]')[0];
      expect(
        ariaCarrier != null && ariaCarrier.exists(),
        'L-2：圆环必须携带 aria-label="上下文用量"（契约 §1 L-2 语义保真）'
      ).toBe(true);

      // Assert（百分比下限 clamp：dashoffset = 2πr(1 − pct/100)，r=7.5，pct=1）
      const lowerOffset = readRing(lowerWrapper).dashOffset;
      expect(Number.isFinite(lowerOffset), "L-2：进度 circle 的 stroke-dashoffset 必须是数值").toBe(true);
      expect(
        Math.abs(lowerOffset - expectedOffset(1)),
        `L-2：比例 9/200000 下百分比必须 clamp 到下限 1%（预期 offset≈${expectedOffset(1).toFixed(2)}，实际 ${lowerOffset}）`
      ).toBeLessThanOrEqual(0.6);

      // Assert（data-context-debug 字面量：in=…;win=…;pct=…;mode=…;speed=…）
      const debugLiteral = lowerRing.attributes("data-context-debug") ?? "";
      expect(
        debugLiteral,
        "L-2：debug 开启时圆环必须输出 data-context-debug 字面量（契约 §1 L-2）"
      ).toContain("in=9");
      expect(debugLiteral, "L-2：data-context-debug 必须含 win= 段").toContain("win=200000");
      expect(debugLiteral, "L-2：data-context-debug 必须含 pct= 段且为 clamp 后的 1").toContain("pct=1");
      expect(debugLiteral, "L-2：data-context-debug 必须含 mode= 段").toMatch(/(^|;)mode=/);
      expect(debugLiteral, "L-2：data-context-debug 必须含 speed= 段").toMatch(/(^|;)speed=/);

      // Assert（tooltip 文案：`上下文用量：{pct}% · {used} / {window} …`）
      const tooltipTexts = collectTooltipTexts(lowerWrapper);
      const ringTooltip = tooltipTexts.find((text) => text.startsWith("上下文用量："));
      expect(
        ringTooltip,
        `L-2：圆环必须把用量信息承载在 tooltip 文案里（以「上下文用量：」开头）；实际 tooltip 文案：${JSON.stringify(tooltipTexts)}`
      ).toBeDefined();
      expect(ringTooltip!, "L-2：圆环 tooltip 必须含百分比 1%").toContain("1%");
      expect(ringTooltip!, "L-2：圆环 tooltip 必须含已用量 9").toContain("9");
      expect(ringTooltip!, "L-2：圆环 tooltip 必须含窗口量（200K / 200000 任一形态）").toMatch(/200(\.0)?K|200000/);
      expect(ringTooltip!, "L-2：圆环 tooltip 必须有 「已用 / 窗口」的斜杠分隔").toContain("/");

      lowerWrapper.unmount();

      // ── 帧 2：比例 > 100% ⇒ clamp 上限 100%（210000 / 200000） ──────────────
      const upperWrapper = mountStatusPanel({ contextDisplayTokens: 210_000, contextWindowTokens: 200_000 });
      const upperOffset = readRing(upperWrapper).dashOffset;
      expect(
        Math.abs(upperOffset - expectedOffset(100)),
        `L-2：比例 >100% 时百分比必须 clamp 到上限 100%（预期 offset≈${expectedOffset(100).toFixed(2)}，实际 ${upperOffset}）`
      ).toBeLessThanOrEqual(0.6);
      expect(
        upperWrapper.get(ringSelector).attributes("data-context-debug") ?? "",
        "L-2：pct= 必须等于 clamp 后的 100"
      ).toContain("pct=100");
      upperWrapper.unmount();
    } finally {
      if (EVIDENCE_KEY_BACKUP == null) {
        window.localStorage.removeItem(DEBUG_KEY);
      } else {
        window.localStorage.setItem(DEBUG_KEY, EVIDENCE_KEY_BACKUP);
      }
    }
  });

  it("L-3: 计数行与 token 行必须同为 4 等列栅格（grid grid-cols-4 items-center），token 行锚点 status-metrics-tokens", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（锚点自检：两行都必须存在，且 token 行不再依赖 class 选择器）
    const countsRowProbe = wrapper.find(countsRowSelector);
    const tokensRowProbe = wrapper.find(tokensRowSelector);
    expect(countsRowProbe.exists(), `L-3：计数行必须存在`).toBe(true);
    expect(
      tokensRowProbe.exists(),
      `L-3：token 行必须新增 [data-testid="${TOKENS_ROW}"] 锚点（取代 wave-17/59 的 justify-around class 锚点）`
    ).toBe(true);

    const countsRow = countsRowProbe;
    const tokensRow = tokensRowProbe;

    // Assert（契约主体 1：两行同为 4 等列栅格）
    for (const [label, row] of [["计数行", countsRow], ["token 行", tokensRow]] as const) {
      const classes = row.classes();
      for (const required of ["grid", "grid-cols-4", "items-center"]) {
        expect(
          classes,
          `${label} 必须携带栅格 class "${required}"（§1 L-3：两行同栅格才能列中心对齐）；实际 class="${row.element.className}"`
        ).toContain(required);
      }
    }

    // Assert（契约主体 2：旧实现痕迹必须消失 —— 3 列栅格 / flex 均分）
    expect(countsRow.classes(), "L-3：计数行不得再是 grid-cols-3").not.toContain("grid-cols-3");
    expect(tokensRow.classes(), "L-3：token 行不得再使用 justify-around").not.toContain("justify-around");

    // Assert（契约主体 3：每行恰好 4 个承载位，栅格列数与位点数一致）
    expect(
      countsRow.findAll(`[data-testid^="status-metric-"], ${ringSelector}`),
      "L-3：计数行必须恰好 4 个承载位（与 grid-cols-4 列数一致）"
    ).toHaveLength(4);
    expect(
      tokensRow.findAll('[data-testid^="status-metric-"]'),
      "L-3：token 行必须恰好 4 个承载位（输入/输出/缓存/生成速度）"
    ).toHaveLength(4);

    // Assert（顺序自检：token 行位点顺序未被迁移打乱）
    expect(
      tokensRow.findAll('[data-testid^="status-metric-"]').map((slot) => slot.attributes("data-testid")),
      "L-3：token 行位点顺序必须为 输入 → 输出 → 缓存 → 生成速度"
    ).toEqual([...TOKEN_SLOT_TESTIDS]);

    wrapper.unmount();
  });

  it("L-4: 位点 = inline-flex/justify-center/gap-1.5 且 min-w 预留上移到容器；图标↔值之间无宽度预留盒", () => {
    // Arrange：缓存比例非空 ⇒ 缓存位含「值 + 尾段」两段文本，用于验证预留盒在**任何**文本节点上都不存在。
    const wrapper = mountStatusPanel({ cacheHitRatio: "12%" });

    // Assert（锚点自检：两行都必须存在，否则下面的逐位点断言会退化成泛化取值异常）
    const countsRowProbe = wrapper.find(countsRowSelector);
    const tokensRowProbe = wrapper.find(tokensRowSelector);
    expect(countsRowProbe.exists(), "L-4：计数行必须存在").toBe(true);
    expect(
      tokensRowProbe.exists(),
      `L-4：token 行必须存在（§1 L-3 新增 [data-testid="${TOKENS_ROW}"] 锚点）`
    ).toBe(true);
    const countsRow = countsRowProbe;
    const tokensRow = tokensRowProbe;

    const expectations = [
      ...COUNT_SLOT_TESTIDS.map((testid) => ({
        label: `计数位 ${testid}`,
        testid,
        row: countsRow,
        minWidth: SLOT_MIN_WIDTH.count
      })),
      ...TOKEN_SLOT_TESTIDS.map((testid) => ({
        label: `token 位 ${testid}`,
        testid,
        row: tokensRow,
        minWidth: testid === "status-metric-token-speed" ? SLOT_MIN_WIDTH.speed : SLOT_MIN_WIDTH.token
      })),
      { label: `圆环位 ${RING_SLOT}`, testid: RING_SLOT, row: countsRow, minWidth: SLOT_MIN_WIDTH.ring }
    ];

    for (const expectation of expectations) {
      // Assert（锚点自检）
      const slotProbe = expectation.row.find(`[data-testid="${expectation.testid}"]`);
      expect(
        slotProbe.exists(),
        `L-4：${expectation.label} 必须存在（有值夹具下所有位点都应渲染）`
      ).toBe(true);
      const slot = slotProbe;

      // Assert（契约主体 1：容器 class —— 图标+值作为一组居中，gap 1.5 提供亲密性）
      for (const required of SLOT_CONTAINER_CLASSES) {
        expect(
          slot.classes(),
          `${expectation.label} 容器必须携带 "${required}"（§1 L-4）；实际 class="${slot.element.className}"`
        ).toContain(required);
      }

      // Assert（契约主体 2：宽度预留落在位点容器自身）
      expect(
        slot.classes(),
        `${expectation.label} 容器必须自持预留下限 "${expectation.minWidth}"（§1 L-4）；实际 class="${slot.element.className}"`
      ).toContain(expectation.minWidth);

      // Assert（契约主体 3：图标↔值之间不得有宽度预留盒 / 右对齐值）
      expect(
        widthReservationViolations(slot.element),
        `${expectation.label} 子树内不得出现 min-w-* / text-right（§1 L-4：禁止图标与值之间的宽度预留盒）`
      ).toEqual([]);
    }

    // Assert（契约主体 4：无「值 + 尾段」的分裂结构的位点，其元素子节点必须恰为「图标 + 值」两个）
    for (const testid of [...COUNT_SLOT_TESTIDS, "status-metric-input-tokens", "status-metric-output-tokens", "status-metric-token-speed"]) {
      const slot = tokensRow.find(`[data-testid="${testid}"]`).exists()
        ? tokensRow.get(`[data-testid="${testid}"]`)
        : countsRow.get(`[data-testid="${testid}"]`);
      expect(
        slot.element.children.length,
        `L-4：${testid} 必须只含「图标 + 值文本」两个子节点（多余子节点即宽度预留盒）；实际 ${slot.element.children.length} 个：${Array.from(slot.element.children).map((node) => node.className || node.tagName).join(" | ")}`
      ).toBe(2);
    }

    // Assert（裁决 R1：缓存比率尾段 `· 12%` 已**移出行内**（并入 tooltip），
    //   故缓存位在行内只保留「图标 + 值」两个子节点，且不得再出现 `·` / 百分比尾段；
    //   尾段去向（tooltip 文案）由 `wave61-empty-values.spec.ts` 的 E-2 用例把关。
    const cacheSlot = tokensRow.get('[data-testid="status-metric-cache-tokens"]');
    expect(
      cacheSlot.element.children.length,
      `L-4：缓存位行内必须只剩「图标 + 值」两个子节点（裁决 R1：尾段移出行内）；实际 ${cacheSlot.element.children.length} 个`
    ).toBe(2);
    expect(
      cacheSlot.text(),
      "L-4：缓存位行内不得再渲染 `· 12%` 尾段（裁决 R1）"
    ).not.toContain("·");
    expect(
      widthReservationViolations(cacheSlot.element),
      "L-4：缓存位子树内不得携带 min-w-* / text-right（预留只允许落在位点容器）"
    ).toEqual([]);

    // Assert（契约表格末句：任何位点 min-w 不得超过 3.5rem —— 4.25rem 会撑破 63.5px track）
    const minWidthTokens = Array.from(
      new Set(
        allElements(wrapper.element)
          .flatMap((element) => Array.from(element.classList))
          .filter((token) => token.startsWith("min-w-"))
      )
    );
    for (const token of minWidthTokens) {
      const remValue = /^min-w-\[([\d.]+)rem\]$/.exec(token)?.[1];
      if (remValue == null) {
        continue;
      }
      expect(
        Number(remValue),
        `L-4：min-w 不得超过 ${SLOT_MIN_WIDTH_CEILING_REM}rem（契约表格末句，Lead 实测 63.5px track 约束）；实际出现 "${token}"`
      ).toBeLessThanOrEqual(SLOT_MIN_WIDTH_CEILING_REM);
    }

    wrapper.unmount();
  });

  it("L-5: 栅格 gap-x-2；计数行/token 行之间保留 border-t 分隔与 mt-2/pt-2；卡内 px-3 py-2 不变", () => {
    // Arrange
    const wrapper = mountStatusPanel();

    // Assert（锚点自检：两行 + 指标卡都必须存在）
    expect(wrapper.find(countsRowSelector).exists(), "L-5：计数行必须存在").toBe(true);
    expect(wrapper.find(tokensRowSelector).exists(), "L-5：token 行必须存在").toBe(true);
    const countsRow = wrapper.get(countsRowSelector);
    const tokensRow = wrapper.get(tokensRowSelector);
    const card = findMetricCard(wrapper.element);
    expect(card, `L-5：指标卡（class "${METRIC_CARD_CLASS}"）必须存在`).not.toBeNull();

    // Assert（契约主体 1：横向 gap + 纵向 gap-y-0）
    for (const [label, row] of [["计数行", countsRow], ["token 行", tokensRow]] as const) {
      for (const required of ["gap-x-2", "gap-y-0"]) {
        expect(
          row.classes(),
          `${label} 必须携带 "${required}"（§1 L-5）；实际 class="${row.element.className}"`
        ).toContain(required);
      }
    }

    // Assert（契约主体 2：行间分隔与上下留白）
    const tokenRowClasses = tokensRow.classes();
    for (const required of ["border-t", "mt-2", "pt-2"]) {
      expect(
        tokenRowClasses,
        `token 行必须携带 "${required}"（§1 L-5：计数行与 token 行之间的既有分隔不回归）；实际 class="${tokensRow.element.className}"`
      ).toContain(required);
    }

    // Assert（契约主体 3：卡内边距不变）
    const cardClasses = Array.from(card!.classList);
    for (const required of ["px-3", "py-2"]) {
      expect(
        cardClasses,
        `指标卡必须保留 "${required}"（§1 L-5：卡内边距不变）；实际 class="${card!.className}"`
      ).toContain(required);
    }

    // Assert（结构自检：两行同属一张卡，且计数行在 token 行之上）
    expect(card!.contains(countsRow.element), "L-5：计数行必须在指标卡内").toBe(true);
    expect(card!.contains(tokensRow.element), "L-5：token 行必须在指标卡内").toBe(true);
    expect(
      Boolean(countsRow.element.compareDocumentPosition(tokensRow.element) & Node.DOCUMENT_POSITION_FOLLOWING),
      "L-5：计数行必须仍在 token 行之上（DOM 序在前）"
    ).toBe(true);

    wrapper.unmount();
  });
});
