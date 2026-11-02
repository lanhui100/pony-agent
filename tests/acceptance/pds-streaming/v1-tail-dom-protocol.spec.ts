/**
 * wave-pds-3 · V1：DOM 协议（尾巴节点）—— jsdom
 *
 * 契约源（只读）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`
 *   §1 DOM 协议（**v2**）：`<div class="markdown-body">`（身份稳定）+ `<span data-stream-tail data-tail-len="N">`；
 *      · 数量**恒 ≤ 1**：存在未闭合尾巴文本时 = 1，无尾巴时 = 0；
 *      · 尾巴 = 源文本的**严格后缀**，不 trim、不重排；
 *      · `data-tail-len` == **源尾巴字符数**（契约 v5 R-A3；v4 的"渲染后可见文本长度"已被覆盖）；
 *        期望值一律由 `splitMarkdownBlocks(source).tail.length` 现算，不写死数字；
 *      · 尾巴非块级（display ∈ {inline, inline-block}）。
 *   §6 V1：三类历史尾巴/字符节点恒为 0。
 *   §0（F3-1 修订）：除 `[data-stream-tail]` 外，内容区不得有第二个尾巴节点。
 *
 * 采样口径（刻意写死在用例里，防「换个采样点就变绿」）：
 *   每个内容增长步都采 **两个帧**：`patch 后的同步帧`（@patch + nextTick）与 `渲染落地帧`（+2 个宏任务）。
 *   尾巴节点的归属组件不作假设 —— 全部在**生产路径** HomeWorkspace 的
 *   `[data-testid="workspace-content-column"]` 内容区内采样。
 *
 * 环境能力边界（如实声明，非托辞）：
 *   vitest 的 jsdom 不注入 SFC/应用 CSS（`css:false`），因此 `getComputedStyle` 对「未在文档中声明的样式」
 *   返回空串（实测：`<span>` 无规则时 `display === ""`）。所以 V1 的「非块级」判据拆成两半：
 *   ① 结构+CASCADE 证据：从仓库 CSS 抽出 `[data-stream-tail]` 规则、注入文档、读取真实 computed display
 *      （用例 V1-4；注入的是仓库源码文本，不是测试自造的期望值）；
 *   ② 规则存在性证据：`[data-stream-tail]` 必须声明 `display: inline` 或 `inline-block`，且不得声明块级值
 *      （用例 V1-3）。二者同时成立才算通过。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import { useRuntimeStore } from "@/stores/runtime";
import { splitMarkdownBlocks } from "@/lib/markdown";
import {
  TAIL_LEN_ATTR,
  TAIL_SELECTOR,
  collectCssPool,
  contentColumn,
  extractTailRules,
  installDomStubs,
  legacyTailTotal,
  MD_VISIBLE_MARKERS,
  mdStreamSteps,
  mountWorkspace,
  patchAssistant,
  probeStreamNodes,
  setTailSwitch,
  settle,
  settleRender,
  stripCssComments
} from "./pds-streaming-harness";
import type { StreamNodesProbe } from "./pds-streaming-harness";

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

interface Sample {
  step: number;
  phase: "sync" | "settled";
  probe: StreamNodesProbe;
}

describe("wave-pds-3 V1：内容区尾巴节点 DOM 协议（jsdom）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    // 释放节奏不属于 V1 判据（契约 §4 单列）；放大速率只为让内容确定性到达展示层，
    // 避免「内容还没被释放出来」被误判成「尾巴缺失」。
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
    installDomStubs();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("V1-1: 数量恒 ≤1；存在未闭合尾巴文本时恒 =1（同步帧 + 落地帧双采样）", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const steps = mdStreamSteps();
    await patchAssistant(store, steps[0]!, "pending");

    const wrapper = mountWorkspace(pinia);
    await settleRender();
    const samples: Sample[] = [];

    const sample = (step: number, phase: Sample["phase"]) => {
      samples.push({ step, phase, probe: probeStreamNodes(contentColumn(wrapper)) });
    };

    sample(0, "settled");

    // Act：逐步增长，每一步都采同步帧与落地帧
    for (let step = 1; step < steps.length; step += 1) {
      await patchAssistant(store, steps[step]!, "pending");
      await nextTick();
      sample(step, "sync");
      await settle(2, 24);
      sample(step, "settled");
    }

    // Assert（自检）：内容确实到达内容区，排除「什么都没渲染 ⇒ 尾巴自然为 0」的假绿
    const last = samples[samples.length - 1]!.probe;
    expect(last.bodyPresent, "V1-1 自检：内容区必须存在 .markdown-body 承载节点").toBe(true);
    expect(last.bodyTextLength, "V1-1 自检：承载节点文本必须非空（内容已到达 DOM）").toBeGreaterThan(20);

    // Assert ①（契约 v2 §1 上界）：任何采样点数量都不得超过 1
    const overCount = samples
      .filter((entry) => entry.probe.tailCount > 1)
      .map((entry) => ({ step: entry.step, phase: entry.phase, tailCount: entry.probe.tailCount }));
    expect(
      overCount,
      `V1-1：内容区 [data-stream-tail] 数量必须恒 ≤1；超量采样：${JSON.stringify(overCount)}`
    ).toHaveLength(0);

    // Assert ②（契约 v2 §1 存在性）：本夹具每一步都以「未定稿尾段」收尾 ⇒ 存在未闭合尾巴文本 ⇒ 必须 =1
    const mismatched = samples
      .filter((entry) => entry.probe.tailCount !== 1)
      .map((entry) => ({ step: entry.step, phase: entry.phase, tailCount: entry.probe.tailCount }));
    expect(
      mismatched,
      `V1-1：存在未闭合尾巴文本时 [data-stream-tail] 必须恰好为 1（夹具每一步都以未定稿尾段收尾）；`
        + `违规采样：${JSON.stringify(mismatched)}`
    ).toHaveLength(0);

    wrapper.unmount();
  });

  it("V1-2: 三类历史尾巴节点恒为 0，且内容区不得出现第二个尾巴属性节点", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const steps = mdStreamSteps();
    await patchAssistant(store, steps[0]!, "pending");
    const wrapper = mountWorkspace(pinia);
    await settleRender();

    const samples: Sample[] = [{ step: 0, phase: "settled", probe: probeStreamNodes(contentColumn(wrapper)) }];

    // Act
    for (let step = 1; step < steps.length; step += 1) {
      await patchAssistant(store, steps[step]!, "pending");
      await nextTick();
      samples.push({ step, phase: "sync", probe: probeStreamNodes(contentColumn(wrapper)) });
      await settle(2, 24);
      samples.push({ step, phase: "settled", probe: probeStreamNodes(contentColumn(wrapper)) });
    }

    // Assert（契约 §0 / V1：三类历史尾巴节点继续为 0）
    const legacyViolations = samples
      .filter((entry) => legacyTailTotal(entry.probe) > 0)
      .map((entry) => ({ step: entry.step, phase: entry.phase, legacy: entry.probe.legacy }));
    expect(
      legacyViolations,
      `V1-2：.mr-streaming-raw / .streaming-unrendered-suffix / .mr-streaming-char 必须恒为 0；违规：${JSON.stringify(legacyViolations)}`
    ).toHaveLength(0);

    // Assert（契约 §0 新增约束：除唯一条尾巴外不得有第二个尾巴节点）
    const duplicateViolations = samples
      .filter((entry) => entry.probe.tailCount > 1 || entry.probe.tailLenAttrCount > 1)
      .map((entry) => ({
        step: entry.step,
        phase: entry.phase,
        tailCount: entry.probe.tailCount,
        tailLenAttrCount: entry.probe.tailLenAttrCount
      }));
    expect(
      duplicateViolations,
      `V1-2：内容区不得出现第二个尾巴节点（[data-stream-tail] / [data-tail-len] 均不得 > 1）；违规：${JSON.stringify(duplicateViolations)}`
    ).toHaveLength(0);

    // Assert（自检，防止「没有尾巴 ⇒ 不可能有第二个尾巴」的假绿）：
    // 「不得有第二个尾巴」只有在第一个尾巴真实存在时才是有效判据。
    const tailObserved = samples.some((entry) => entry.probe.tailCount === 1);
    expect(
      tailObserved,
      "V1-2 自检：采样窗口内必须至少有一帧存在 [data-stream-tail]；否则本用例退化成了「0 个尾巴不含第二个尾巴」的假绿"
    ).toBe(true);

    wrapper.unmount();
  });

  it("V1-3: 尾巴必须声明为行内级（display ∈ {inline, inline-block}），不得声明块级", async () => {
    // Arrange：从仓库 CSS 源码抽出所有作用于 [data-stream-tail] 的规则
    const pool = collectCssPool();
    const tailRules = pool.flatMap((entry) =>
      extractTailRules(entry.css).map((rule) => ({ ...rule, path: entry.path }))
    );

    // Act
    const displayDeclarations = tailRules
      .flatMap((rule) =>
        Array.from(rule.body.matchAll(/(?:^|;)\s*display\s*:\s*([^;]+)/g)).map((match) => ({
          path: rule.path,
          selector: rule.selector,
          value: (match[1] ?? "").trim()
        }))
      );

    // Assert ①：必须存在 [data-stream-tail] 的 display 声明（契约 §1 的 display:inline 是强制项）
    expect(
      displayDeclarations,
      `V1-3：仓库 CSS 中必须存在对 [data-stream-tail] 的 display 声明（契约 §1 / §3.1）；`
        + `实际扫描来源：${pool.map((entry) => entry.path).join(", ")}；命中的尾巴规则：${JSON.stringify(tailRules)}`
    ).not.toHaveLength(0);

    // Assert ②：至少一条声明是行内级
    const inlineDeclarations = displayDeclarations.filter((entry) => /^(inline|inline-block)\b/.test(entry.value));
    expect(
      inlineDeclarations,
      `V1-3：尾巴必须 display:inline（或 inline-block）；实际声明：${JSON.stringify(displayDeclarations)}`
    ).not.toHaveLength(0);

    // Assert ③：不得有任何块级声明覆盖它
    const blockDeclarations = displayDeclarations.filter((entry) =>
      /^(block|flex|grid|list-item|flow-root|table)\b/.test(entry.value)
    );
    expect(
      blockDeclarations,
      `V1-3：尾巴不得声明为块级（会产生新行，违反契约 §1）；违规声明：${JSON.stringify(blockDeclarations)}`
    ).toHaveLength(0);
  });

  it("V1-4: 注入仓库真实尾巴样式后，真实尾巴节点的 computed display 落在 {inline, inline-block}", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const steps = mdStreamSteps();
    await patchAssistant(store, steps[steps.length - 1]!, "pending");
    const wrapper = mountWorkspace(pinia);
    await settleRender();

    const column = contentColumn(wrapper);
    const tail = column?.querySelector(TAIL_SELECTOR) ?? null;
    expect(
      tail,
      "V1-4 自检：内容区必须存在真实尾巴节点（否则本用例无对象可测；V1-1 已锁其数量为 1）"
    ).not.toBeNull();

    const tailCss = collectCssPool()
      .flatMap((entry) => extractTailRules(stripCssComments(entry.css)).map((rule) => `${rule.selector} { ${rule.body} }`))
      .join("\n");

    // Act：把仓库里的尾巴规则注入文档（jsdom 不做 CSS 注入，这里显式补上，注入内容来自仓库源码）
    const styleEl = document.createElement("style");
    styleEl.textContent = tailCss;
    document.head.appendChild(styleEl);
    const computedDisplay = window.getComputedStyle(tail as Element).display;
    document.head.removeChild(styleEl);

    // Assert
    expect(
      ["inline", "inline-block"],
      `V1-4：尾巴 computed display 必须属于 {inline, inline-block}，实际为 ${JSON.stringify(computedDisplay)}；`
        + `注入的规则：${tailCss || "（仓库 CSS 中不存在 [data-stream-tail] 规则）"}`
    ).toContain(computedDisplay);

    wrapper.unmount();
  });

  it("V1-5: data-tail-len == splitMarkdownBlocks 的源尾巴字符数（R-A3），纯文本尾仍为严格后缀", async () => {
    // Arrange（契约 v5 变更说明）：
    //   v4 曾把 `data-tail-len` 定义为「渲染后可见文本长度」，v5 R-A3 改回**源尾巴字符数**
    //   —— 两者在 `**加粗**` 这类尾巴上必然不等（源 13 字符 vs 可见 9 字符），
    //   因此本用例的夹具**必须含 markdown 语法的尾巴**，否则两种语义无法区分（v1–v4 的盲点）。
    //   期望值全部由 `splitMarkdownBlocks` 现算：不写死数字，也不对实现让步。
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const steps = [
      mdStreamSteps()[0]!,                                        // 纯文本尾巴（保留严格后缀判据）
      "## 已定稿标题\n\n这是**加粗\n尾巴**继续",                    // 跨行加粗尾巴
      "## 已定稿标题\n\n| a | b |\n| --- | --- |\n| 1 | 2",     // 表格尾巴
      "## 已定稿标题\n\n- 一级项\n  - 二级项 A"                    // 嵌套列表尾巴
    ];
    await patchAssistant(store, steps[0]!, "pending");
    const wrapper = mountWorkspace(pinia);
    await settleRender();

    const violations: Array<Record<string, unknown>> = [];
    const observed: Array<{ step: number; declared: number | null; expected: number; tailText: string }> = [];

    // Act + Assert（逐步增长时校验协议自洽）
    for (let step = 0; step < steps.length; step += 1) {
      if (step > 0) {
        await patchAssistant(store, steps[step]!, "pending");
        await settleRender();
      }
      const source = steps[step]!;
      const expected = splitMarkdownBlocks(source).tail.length;
      const probe = probeStreamNodes(contentColumn(wrapper));

      for (const tail of probe.tails) {
        observed.push({ step, declared: tail.declaredLen, expected, tailText: tail.text });

        // Assert ①（R-A3 主判据）：data-tail-len == 源尾巴字符数
        if (tail.declaredLen === null || Number.isNaN(tail.declaredLen)) {
          violations.push({ step, reason: `[${TAIL_LEN_ATTR}] 缺失或非数字`, declared: tail.declaredLen });
        } else if (tail.declaredLen !== expected) {
          violations.push({
            step,
            reason: "data-tail-len 必须等于 splitMarkdownBlocks 的源尾巴字符数（R-A3）",
            declared: tail.declaredLen,
            expected,
            source: JSON.stringify(source)
          });
        }

        // Assert ②：纯文本尾巴才做"严格后缀、不 trim"判据
        //   （含 markdown 语法的尾巴，其可见文本天然不等于源后缀，这不是缺陷）
        const tailSource = source.slice(source.length - expected);
        const isPlainTail = !/[*_`#>|]|^\s*[-+]\s|^\d+\.\s|\n/m.test(tailSource);
        if (isPlainTail && tail.textLength > 0 && !source.endsWith(tail.text)) {
          violations.push({
            step,
            reason: "纯文本尾巴必须是源文本的严格后缀（不得 trim/重排）",
            tail: tail.text.slice(-40)
          });
        }
      }
    }

    // Assert 自检：必须真的观测到非空尾巴
    expect(
      observed.filter((entry) => entry.tailText.length > 0).length,
      `V1-5 自检：必须观测到非空尾巴节点（实际采样：${JSON.stringify(observed.slice(0, 3))}）`
    ).toBeGreaterThan(0);

    expect(
      violations,
      `V1-5：尾巴协议不自洽：${JSON.stringify(violations, null, 1)}`
    ).toHaveLength(0);

    wrapper.unmount();
  });

  it("V1-6: 块闭合帧（内容以空行收尾）无未闭合尾巴文本 ⇒ 数量必须为 0", async () => {
    // Arrange：契约 v2 §1 已定义取值 ——「存在未闭合尾巴文本时 =1；无尾巴时 =0」。
    // 本夹具的资料来源全部闭合（以空行收尾）⇒ 期望 0，且不得丢内容。
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    await patchAssistant(store, "## 已闭合标题\n\n已闭合段落。\n\n", "pending");
    const wrapper = mountWorkspace(pinia);
    await settleRender();

    // Act
    const probe = probeStreamNodes(contentColumn(wrapper));

    // Assert
    expect(
      probe.tailCount,
      `V1-6：无未闭合尾巴文本（全部块已闭合）时 [data-stream-tail] 数量必须为 0（契约 v2 §1），实际 ${probe.tailCount}`
    ).toBe(0);
    expect(legacyTailTotal(probe), "V1-6：块边界帧同样不得出现历史尾巴节点").toBe(0);
    expect(probe.bodyTextLength, "V1-6 自检：已闭合内容必须可见").toBeGreaterThan(0);

    wrapper.unmount();
  });

  it("V1-7: 全文可见性自检 —— 尾巴机制不得吞掉已定稿内容", async () => {
    // Arrange
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const steps = mdStreamSteps();
    await patchAssistant(store, steps[0]!, "pending");
    const wrapper = mountWorkspace(pinia);
    await settleRender();

    // Act：推到最后一帧
    await patchAssistant(store, steps[steps.length - 1]!, "pending");
    await settleRender();
    const column = contentColumn(wrapper);
    const visibleText = `${column?.textContent ?? ""}`;

    // Assert：markdown 语法字符不计，正文标记必须全部可见
    const missing = MD_VISIBLE_MARKERS.filter((marker) => !visibleText.includes(marker));
    expect(
      missing,
      `V1-7：以下正文内容未出现在内容区（尾巴/掩码吞掉了已定稿内容）：${JSON.stringify(missing)}`
    ).toHaveLength(0);

    wrapper.unmount();
  });

  it("V1-8: 默认开关（未写 localStorage）下尾巴处于开启态", async () => {
    // Arrange：契约 §2「缺省 = on」
    setTailSwitch(null);
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useRuntimeStore();
    const steps = mdStreamSteps();
    await patchAssistant(store, steps[steps.length - 1]!, "pending");
    const wrapper = mountWorkspace(pinia);
    await settleRender();

    // Act
    const probe = probeStreamNodes(contentColumn(wrapper));

    // Assert
    expect(probe.bodyTextLength, "V1-8 自检：内容必须已渲染").toBeGreaterThan(20);
    expect(
      probe.tailCount,
      `V1-8：缺省（未写 ${"pony-agent.stream-render.tail-reveal"}）时尾巴必须开启 ⇒ 数量为 1，实际 ${probe.tailCount}`
    ).toBe(1);

    wrapper.unmount();
  });
});
