/**
 * Wave-61 冻结验收（L2-T）— §3 动效契约 M-1..M-5（jsdom 层：CSS 声明面 + DOM 挂载面）
 *
 * 契约源（只读）：`.dev-team/contracts/wave-61-status-panel.md` §3。
 * 用户诉求（契约 §0-3）：有值时**平滑动效入场**；空值不渲染；降级下"直接出现"。
 *
 * 为什么在 jsdom 层只能这样测（口径必须写清，禁止假装测到了物理动画）：
 *   vitest 的 jsdom 环境既不执行 Tailwind，也**不注入** SFC `<style>`（`test.css` 默认关闭），
 *   因此 `getComputedStyle` 读不到 200ms / cubic-bezier / reduced-motion 屏蔽规则。
 *   契约 §3 允许的 jsdom 代理是「scoped CSS 中存在 enter 类规则 + 降级关闭规则」，
 *   故本文件把**应用真实会打包的两处 CSS 面**（`HomeStatusPanel.vue` 的 `<style>` + `src/styles.css`）
 *   解析成结构，再与真实挂载 DOM 的 class 集合**求交**：
 *     · 只写 CSS 但从不挂该 class ⇒ 交集为空 ⇒ 失败（挡住"写了 CSS 等于实现"的假绿）；
 *     · 只挂 class 但 CSS 无对应规则 ⇒ 找不到 200ms/cubic-bezier 规则 ⇒ 失败。
 *   真实浏览器里的物理动效（`getAnimations()` 命中 200ms / cubic-bezier / opacity+translateY + 圆环 scale）
 *   由 `tests/e2e/wave61-status-panel.spec.ts`（G-4b）用真实 Chromium 把关。
 *
 * 实现机制（契约 §3 **M-1 裁决 R4**，Lead 钉死，本文件按此冻结）：
 *   1. 位点根元素在「有值出现」时携带**静态 class** `status-slot-enter`（圆环位附加
 *      `status-slot-enter--ring`），**不得**使用 `<TransitionGroup>`；
 *      理由（契约原文）：位点外层是 `Tooltip`（Fragment 根），TransitionGroup 对 Fragment 根
 *      只 `console.warn` 而不下发 enter 类；「每次挂载即重播」是本契约的预期语义。
 *   2. 入场效果以 `@keyframes` 承载：`from` 含 `opacity: 0` 与 `translateY(4px)`，`to` 含 `opacity: 1`，
 *      时长 `200ms`、缓动 `cubic-bezier(0.3, 0.7, 0.4, 1)`（三项必须同一条 animation 声明）；
 *   3. 圆环位额外 `scale(0.92)` 出现在其入场起始帧；
 *   4. reduced-motion 双轨：`@media (prefers-reduced-motion: reduce)` 与
 *      `[data-pds-reduce-motion="true"]` 两条轨道都要把入场动画关掉（`animation: none` /
 *      `transition: none`），与 `src/styles.css:173-255` 既有双轨约定一致。
 *
 * 失败归因纪律：每条 M-* 用例只在**自己的契约点**上报错；位点缺失（属 §1 L-1..L-3）
 *   不会在 beforeEach 里吞掉全部用例，而是由各自用例的锚点自检点名。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import type { DOMWrapper, VueWrapper } from "@vue/test-utils";
import {
  COUNT_SLOT_TESTIDS,
  COUNTS_ROW,
  RING_ENTER_CLASS,
  RING_SLOT,
  SLOT_ENTER_CLASS,
  TOKENS_ROW,
  TOKEN_SLOT_TESTIDS,
  allElements,
  collectClassTokens,
  compactCss,
  compactSelector,
  hasTabularNumsAtOrAbove,
  keyframeFrame,
  mountStatusPanel,
  readWave61CssSurfaces,
  referencedKeyframeNames,
  ruleTargetsClasses,
  selectorClassTokens,
  tabularNumsFailureDetail
} from "./wave61-harness";
import type { Wave61CssRule, Wave61CssSurface, Wave61Keyframes } from "./wave61-harness";

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

const DURATION_TOKEN = "200ms";
const EASING_TOKEN = "cubic-bezier(0.3,0.7,0.4,1)";

/** 禁止被动画驱动的高开销属性（§3 M-3：只动合成层）。 */
const LAYOUT_PROPERTY_PATTERN =
  /(^|[;{])\s*(width|height|min-width|max-width|min-height|max-height|margin|margin-[a-z]+|padding|padding-[a-z]+|gap|row-gap|column-gap|inset|top|left|right|bottom|font-size|line-height)\s*:/;

/** 入场关键帧允许声明的属性白名单（§3 M-3：只动合成层）。 */
const ALLOWED_MOTION_PROPERTIES = ["opacity", "transform", "animation-timing-function", "filter", "will-change"];

interface MotionEvidence {
  /** 声明 200ms + easing 且命中真实位点 class 的规则。 */
  rule: Wave61CssRule;
  keyframesBlock: Wave61Keyframes | null;
  fromFrame: Wave61CssRule | null;
  toFrame: Wave61CssRule | null;
  /** 该规则选择器中、真实命中位点 DOM 的 class token。 */
  matchedClasses: string[];
}

function ruleMatchesClassTokens(rule: Wave61CssRule, classTokens: Set<string>): string[] {
  return selectorClassTokens(rule.selector).filter((token) => classTokens.has(token));
}

/**
 * 在 CSS 面中找「入场动效证据」：200ms + 契约缓动 + 命中真实位点 class + 起始态 opacity/translateY。
 * 同时接受 `@keyframes` 与 `transition` 两种承载方式（见文件头实现约定 2）。
 */
function findMotionEvidence(surface: Wave61CssSurface, classTokens: Set<string>): MotionEvidence | null {
  const knownNames = surface.keyframes.map((block) => block.name);
  const candidates = surface.rules
    .filter((rule) => ruleTargetsClasses(rule, classTokens))
    .filter((rule) => {
      const body = compactCss(rule.body);
      return body.includes(DURATION_TOKEN) && body.includes(EASING_TOKEN);
    });

  // 路径 A：animation + @keyframes（契约首选）
  for (const rule of candidates) {
    for (const name of referencedKeyframeNames(rule.body, knownNames)) {
      const keyframesBlock = surface.keyframes.find((block) => block.name === name) ?? null;
      if (!keyframesBlock) {
        continue;
      }
      const fromFrame = keyframeFrame(keyframesBlock, "from");
      const toFrame = keyframeFrame(keyframesBlock, "to");
      if (!fromFrame || !toFrame) {
        continue;
      }
      const fromBody = compactCss(fromFrame.body);
      const toBody = compactCss(toFrame.body);
      if (fromBody.includes("opacity:0") && fromBody.includes("translatey(4px)") && toBody.includes("opacity:1")) {
        return {
          rule,
          keyframesBlock,
          fromFrame,
          toFrame,
          matchedClasses: ruleMatchesClassTokens(rule, classTokens)
        };
      }
    }
  }

  // 路径 B：transition（同 class 规则内给出起始态）
  for (const rule of candidates) {
    const body = compactCss(rule.body);
    if (body.includes("opacity:0") && body.includes("translatey(4px)")) {
      return {
        rule,
        keyframesBlock: null,
        fromFrame: null,
        toFrame: null,
        matchedClasses: ruleMatchesClassTokens(rule, classTokens)
      };
    }
  }

  return null;
}

/** 失败信息：把「读过哪些 CSS 面、里面有什么动效线索」全部打印出来，便于归因 SUT。 */
function describeSurface(surface: Wave61CssSurface): string {
  const timingCandidates = surface.rules
    .filter((rule) => compactCss(rule.body).includes(DURATION_TOKEN))
    .map((rule) => `${rule.source} :: "${rule.selector}" { ${rule.body.replace(/\s+/g, " ").trim()} }`);
  return [
    `读取的 CSS 面：${surface.files.join(" + ")}`,
    `含 ${DURATION_TOKEN} 的规则数：${timingCandidates.length}`,
    ...timingCandidates.slice(0, 6),
    `@keyframes：${surface.keyframes.map((block) => block.name).join(", ") || "(无)"}`
  ].join("\n  ");
}

describe("wave-61 §3 动效契约（M-1..M-5）", () => {
  let wrapper: VueWrapper;
  let surface: Wave61CssSurface;
  let countsRow: DOMWrapper<Element>;
  let tokensRow: DOMWrapper<Element> | null;
  let ringSlot: DOMWrapper<Element> | null;
  /** 当前实现下**已渲染**的位点（缺失的位点由 §1/§2 的用例负责报警，本文件不重复霸占失败点）。 */
  let slotElements: DOMWrapper<Element>[];
  let slotClassTokens: Set<string>;

  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);

    surface = readWave61CssSurfaces();
    wrapper = mountStatusPanel();

    const rootWrapper = wrapper as unknown as DOMWrapper<Element>;
    const countsRowProbe = rootWrapper.find(`[data-testid="${COUNTS_ROW}"]`);
    if (!countsRowProbe.exists()) {
      throw new Error(
        `M-* 前置条件未满足：§1 L-3 计数行 [data-testid="${COUNTS_ROW}"] 必须存在（否则无法把动效规则与位点 class 求交）`
      );
    }
    countsRow = countsRowProbe;

    const tokensRowProbe = rootWrapper.find(`[data-testid="${TOKENS_ROW}"]`);
    tokensRow = tokensRowProbe.exists() ? tokensRowProbe : null;
    const ringProbe = countsRow.find(`[data-testid="${RING_SLOT}"]`);
    ringSlot = ringProbe.exists() ? ringProbe : null;

    const candidates = [
      ...COUNT_SLOT_TESTIDS.map((testid) => countsRow.find(`[data-testid="${testid}"]`)),
      ...(tokensRow ? TOKEN_SLOT_TESTIDS.map((testid) => tokensRow!.find(`[data-testid="${testid}"]`)) : [])
    ];
    slotElements = candidates.filter((slot) => slot.exists());

    // 位点范围内（根 + 子树）的 class 集合：CSS 规则必须命中其中的 class，才算真的作用于位点。
    slotClassTokens = collectClassTokens([
      ...slotElements.flatMap((slot) => allElements(slot.element)),
      ...(ringSlot ? allElements(ringSlot.element) : [])
    ]);
  });

  afterEach(() => {
    wrapper.unmount();
    vi.restoreAllMocks();
  });

  it("M-1: 入场动画存在且参数精确（200ms / cubic-bezier(0.3,0.7,0.4,1) / opacity 0→1 / translateY(4px)→0 + 圆环 scale(0.92)）", () => {
    // Assert（锚点自检：位点 DOM 必须真的渲染出来，否则「CSS 规则命中位点 class」无从求交）
    expect(
      slotElements.length,
      "M-1：有值夹具下数值位点必须渲染（否则无法把 CSS 动效规则与真实 class 求交）"
    ).toBeGreaterThan(0);
    expect(ringSlot, `M-1：圆环位 [data-testid="${RING_SLOT}"] 必须存在（scale(0.92) 的载体）`).not.toBeNull();

    // Assert（契约主体 0 · 裁决 R4：位点根元素必须携带**静态**入场 class）
    for (const slot of slotElements) {
      expect(
        slot.classes(),
        `M-1（R4）：位点 ${slot.attributes("data-testid")} 根元素必须携带静态 class "${SLOT_ENTER_CLASS}"；实际 class="${slot.element.className}"`
      ).toContain(SLOT_ENTER_CLASS);
    }
    expect(
      ringSlot!.classes(),
      `M-1（R4）：圆环位必须同时携带 "${SLOT_ENTER_CLASS}" 与 "${RING_ENTER_CLASS}"；实际 class="${ringSlot!.element.className}"`
    ).toEqual(expect.arrayContaining([SLOT_ENTER_CLASS, RING_ENTER_CLASS]));

    // Act：在 CSS 面中定位入场动效证据（数值位点 + 圆环位）
    const evidence = findMotionEvidence(surface, slotClassTokens);

    // Assert（契约主体 1：动效规则必须存在，且命中真实位点 class）
    expect(
      evidence,
      `M-1：未找到入场动效证据（契约要求：一条命中 ".${SLOT_ENTER_CLASS}" 的规则以 @keyframes 声明 ${DURATION_TOKEN} 与 ${EASING_TOKEN}，`
        + `且起始帧含 opacity:0 + translateY(4px)、终止帧含 opacity:1）。\n  ${describeSurface(surface)}`
    ).not.toBeNull();
    const motion = evidence!;
    expect(
      motion.matchedClasses.length,
      `M-1：动效规则的选择器必须命中真实位点 DOM 的 class（防止「CSS 有规则但从未挂类」）；规则="${motion.rule.selector}"`
    ).toBeGreaterThan(0);

    // Assert（契约主体 2：时长与缓动必须同一条 animation 声明给出 —— 数值取自契约原文）
    const motionBody = compactCss(motion.rule.body);
    expect(motionBody, `M-1：入场动效必须声明时长 ${DURATION_TOKEN}`).toContain(DURATION_TOKEN);
    expect(
      motionBody,
      `M-1：入场动效必须声明缓动 ${EASING_TOKEN}（契约 §3 M-1 原文）`
    ).toContain(EASING_TOKEN);

    // Assert（契约主体 3：裁决 R4 钉死 @keyframes 承载 —— 不接受 Transition/TransitionGroup 方案）
    expect(
      motion.keyframesBlock,
      `M-1（R4）：入场动效必须由 @keyframes 承载（不接受 TransitionGroup / transition 方案）；规则="${motion.rule.selector}"`
    ).not.toBeNull();
    const fromBody = compactCss(motion.fromFrame!.body);
    const toBody = compactCss(motion.toFrame!.body);
    expect(fromBody, "M-1：入场起始帧必须 opacity: 0").toContain("opacity:0");
    expect(fromBody, "M-1：入场起始帧必须 translateY(4px)").toContain("translatey(4px)");
    expect(toBody, "M-1：入场终止帧必须 opacity: 1").toContain("opacity:1");
    // 终止帧可不写 transform（CSS 会回落到元素常态），但只要写了就必须归零。
    const toTransform = /transform:([^;]*)/.exec(toBody)?.[1] ?? null;
    if (toTransform != null) {
      expect(
        toTransform,
        "M-1：入场终止帧的位移必须归零（translateY(0) / translateY(0px) / none 任一）"
      ).toMatch(/translatey\(0(px)?\)|none/);
    }

    // Assert（契约主体 4：圆环位额外 scale(0.92)，且必须被 ".${RING_ENTER_CLASS}" 规则真实引用）
    const ringClassTokens = collectClassTokens(allElements(ringSlot!.element));
    expect(
      ringClassTokens.has(RING_ENTER_CLASS),
      `M-1（R4）：圆环位根元素必须携带 "${RING_ENTER_CLASS}"（其入场规则的选择器锚点）`
    ).toBe(true);
    const scaleKeyframes = surface.keyframes.filter((block) => {
      const from = keyframeFrame(block, "from");
      return from != null && compactCss(from.body).includes("scale(0.92)");
    });
    expect(
      scaleKeyframes.map((block) => block.name),
      "M-1：必须存在以 scale(0.92) 作为起始帧的 @keyframes（契约 §3 M-1：圆环位额外 scale(0.92)→1）"
    ).not.toEqual([]);
    const ringEvidence = scaleKeyframes.some((block) =>
      surface.rules.some(
        (rule) =>
          ruleTargetsClasses(rule, ringClassTokens) &&
          referencedKeyframeNames(rule.body, [block.name]).includes(block.name)
      )
    );
    expect(
      ringEvidence,
      `M-1：scale(0.92) 的 @keyframes 必须被命中圆环位 class 的规则引用（防止只定义不引用）；圆环 class="${ringSlot!.element.className}"`
    ).toBe(true);
  });

  it("M-2: 不做退场动画（位点相关 CSS 面不得出现 leave/exit 规则或关键帧）", () => {
    // Assert（锚点自检：位点 class 集合非空，否则「位点相关规则」为空集、断言恒真）
    expect(
      slotClassTokens.size,
      "M-2：位点 DOM 必须渲染出 class（否则 leave 规则检测退化为空集断言）"
    ).toBeGreaterThan(0);

    // Assert（契约主体 1：位点相关规则中不得出现 leave/exit）
    const leaveRules = surface.rules
      .filter((rule) => ruleTargetsClasses(rule, slotClassTokens))
      .filter((rule) => /leave|exit/i.test(`${rule.selector} ${rule.body}`))
      .map((rule) => `${rule.source} :: ${rule.selector} { ${rule.body.replace(/\s+/g, " ").trim()} }`);
    expect(
      leaveRules,
      "M-2：位点不得声明退场（leave/exit）动画 —— 契约要求即时移除，避免绝对定位幽灵节点与行高抖动"
    ).toEqual([]);

    // Assert（契约主体 2：不得存在 leave/exit 关键帧）
    const leaveKeyframes = surface.keyframes
      .filter((block) => /leave|exit/i.test(block.name))
      .map((block) => `${block.source} :: @keyframes ${block.name}`);
    expect(
      leaveKeyframes,
      "M-2：不得存在 leave/exit 关键帧（契约 §3 M-2：不做退场动画）"
    ).toEqual([]);

    // Assert（契约主体 3：DOM 上不得残留 leave 类 —— Vue Transition 的 leave 态类名同属退场动画）
    const leaveClasses = allElements(wrapper.element)
      .flatMap((element) => Array.from(element.classList))
      .filter((token) => /leave/.test(token));
    expect(
      Array.from(new Set(leaveClasses)),
      "M-2：挂载后的位点 DOM 上不得出现 leave 相关 class"
    ).toEqual([]);

    // Assert（裁决 R4 的负向半边：机制必须不是 TransitionGroup）
    const componentPath = path.join(process.cwd(), "src", "components", "HomeStatusPanel.vue");
    expect(fs.existsSync(componentPath), `M-2：被测组件源码必须存在 → ${componentPath}`).toBe(true);
    const componentSource = fs.readFileSync(componentPath, "utf-8");
    expect(
      componentSource,
      "M-2（R4）：不得使用 <TransitionGroup> 承载位点入场（对 Fragment 根不下发 enter 类，只会 warn）"
    ).not.toContain("TransitionGroup");
  });

  it("M-3: 只动合成层（入场声明与关键帧不得驱动 width/height/margin/gap 等布局属性）", () => {
    // Arrange
    const evidence = findMotionEvidence(surface, slotClassTokens);
    expect(
      evidence,
      `M-3：未找到入场动效证据，无法判定合成层约束（§3 M-3 以 M-1 的入场动效为前提）。\n  ${describeSurface(surface)}`
    ).not.toBeNull();
    const motion = evidence!;

    // Assert（契约主体 1：动效规则本身不得声明布局属性）
    expect(
      motion.rule.body.match(LAYOUT_PROPERTY_PATTERN)?.[0] ?? null,
      `M-3：入场规则不得驱动布局属性（契约 §3 M-3）；规则="${motion.rule.selector}" body="${motion.rule.body.replace(/\s+/g, " ").trim()}"`
    ).toBeNull();

    // Assert（契约主体 2：关键帧帧体不得驱动布局属性）
    if (motion.keyframesBlock) {
      for (const frame of motion.keyframesBlock.frames) {
        expect(
          frame.body.match(LAYOUT_PROPERTY_PATTERN)?.[0] ?? null,
          `M-3：@keyframes ${motion.keyframesBlock.name} 的 "${frame.selector}" 帧不得驱动布局属性；body="${frame.body.replace(/\s+/g, " ").trim()}"`
        ).toBeNull();
      }

      // Assert（契约主体 3：正向要求 —— 关键帧只允许合成层属性）
      const declaredProperties = motion.keyframesBlock.frames
        .flatMap((frame) => compactCss(frame.body).split(";"))
        .map((declaration) => declaration.split(":")[0] ?? "")
        .filter((property) => property.length > 0);
      for (const property of new Set(declaredProperties)) {
        expect(
          ALLOWED_MOTION_PROPERTIES.includes(property),
          `M-3：入场关键帧只允许合成层属性（${ALLOWED_MOTION_PROPERTIES.join(" / ")}），实际声明了 "${property}"`
        ).toBe(true);
      }
    }
  });

  it("M-4: reduced-motion 双轨降级（@media prefers-reduced-motion 与 [data-pds-reduce-motion=\"true\"] 均关闭入场）", () => {
    // Arrange
    const evidence = findMotionEvidence(surface, slotClassTokens);
    expect(evidence, `M-4：未找到入场动效证据，无法判定降级分支（§3 M-4 以 M-1 的入场动效为前提）。\n  ${describeSurface(surface)}`).not.toBeNull();
    const enterClasses = new Set(
      selectorClassTokens(evidence!.rule.selector).filter((token) => slotClassTokens.has(token))
    );

    const disablesMotion = (body: string): boolean => {
      const compact = compactCss(body);
      return compact.includes("animation:none") || compact.includes("transition:none");
    };
    const targetsEnterClass = (rule: Wave61CssRule): boolean =>
      selectorClassTokens(rule.selector).some((token) => enterClasses.has(token));

    // Assert（契约主体 1：系统媒体查询轨道）
    const mediaTrackRules = surface.rules.filter(
      (rule) =>
        rule.atRules.some((atRule) => compactSelector(atRule).includes("prefers-reduced-motion:reduce")) &&
        targetsEnterClass(rule) &&
        disablesMotion(rule.body)
    );
    expect(
      mediaTrackRules.map((rule) => `${rule.source} :: ${rule.selector}`),
      `M-4：@media (prefers-reduced-motion: reduce) 下必须有关闭入场动画的规则（契约 §3 M-4 轨道一）；入场 class=${[...enterClasses].join(",")}`
    ).not.toEqual([]);

    // Assert（契约主体 2：应用内根属性轨道）
    const attributeTrackRules = surface.rules.filter(
      (rule) =>
        compactSelector(rule.selector).includes("[data-pds-reduce-motion=true]") &&
        targetsEnterClass(rule) &&
        disablesMotion(rule.body)
    );
    expect(
      attributeTrackRules.map((rule) => `${rule.source} :: ${rule.selector}`),
      "M-4：[data-pds-reduce-motion=\"true\"] 下必须有关闭入场动画的规则（契约 §3 M-4 轨道二，与既有 styles.css 双轨约定一致）"
    ).not.toEqual([]);

    // Assert（契约主体 3：降级不得只关一半 —— 两条轨道都必须显式覆盖入场 class）
    expect(
      mediaTrackRules.length > 0 && attributeTrackRules.length > 0,
      "M-4：双轨必须同时存在（只在一轨降级 ＝ 真遗漏）"
    ).toBe(true);
  });

  it("M-5: 动效不改变位点顺序 / testid / tabular-nums 承载链（wave-17 C2 不回归）", () => {
    // Assert（锚点自检：两行与圆环位都必须存在，顺序断言才具备区分力）
    expect(tokensRow, `M-5：token 行 [data-testid="${TOKENS_ROW}"] 必须存在`).not.toBeNull();
    expect(ringSlot, `M-5：圆环位 [data-testid="${RING_SLOT}"] 必须存在`).not.toBeNull();

    // Assert（契约主体 1：顺序不变）
    expect(
      countsRow.findAll(`[data-testid^="status-metric-"], [data-testid="${RING_SLOT}"]`).map((slot) => slot.attributes("data-testid")),
      "M-5：动效不得改变计数行位点顺序（轮次 → 模型调用 → 工具调用 → 圆环）"
    ).toEqual([...COUNT_SLOT_TESTIDS, RING_SLOT]);
    expect(
      tokensRow!.findAll('[data-testid^="status-metric-"]').map((slot) => slot.attributes("data-testid")),
      "M-5：动效不得改变 token 行位点顺序（输入 → 输出 → 缓存 → 生成速度）"
    ).toEqual([...TOKEN_SLOT_TESTIDS]);

    // Assert（契约主体 2：testid 不变 —— 圆环仍不得带 status-metric- 前缀）
    expect(ringSlot!.attributes("data-testid"), "M-5：圆环位 testid 必须仍为 status-context-usage").toBe(RING_SLOT);
    expect(
      countsRow.findAll('[data-testid^="status-metric-"]'),
      "M-5：带 status-metric- 前缀的位点必须仍恰好 3 个"
    ).toHaveLength(3);

    // Assert（契约主体 3：tabular-nums 承载链不变 —— 与 wave-17 C2 判定同口径）
    expect(
      slotElements.length,
      "M-5：有值夹具下位点必须渲染（tabular-nums 链断言的前提）"
    ).toBeGreaterThan(0);
    for (const slot of slotElements) {
      expect(
        hasTabularNumsAtOrAbove(slot.element),
        `M-5：${slot.attributes("data-testid")} 的 tabular-nums 承载链不得被动效改造破坏；实际链路：${tabularNumsFailureDetail(slot.element)}`
      ).toBe(true);
    }
  });
});
