/**
 * wave-pds-3 · V2：回退开关（`pony-agent.stream-render.tail-reveal`）—— jsdom
 *
 * 契约源（只读）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`
 *   §2 回退开关：`localStorage["pony-agent.stream-render.tail-reveal"]` 缺省 = on，值 `"off"` = 关闭；
 *      关闭后不渲染 `[data-stream-tail]`，全部内容走 stableHtml，行为等同 wave-59；
 *   §6 V2：开关置 `"off"` 后 tail 数量恒为 0，且既有 14 例基线行为不变。
 *
 * 关于 V2 的第二半句「既有 14 例基线行为不变」：
 *   本文件**不重抄**那 14 例（重抄即等于给冻结基线留一份可漂移的副本）。它由两处真凭证覆盖：
 *   ① `v10-baseline-freeze.spec.ts` 的真实子进程执行（exit code + 14/23 计数 + 文件哈希锚定）；
 *   ② 本文件 V2-3 的**等价性断言**：关闭态下承载节点身份、内容完整度、节点数量与 wave-59 语义一致。
 *
 * 环境能力边界：契约没有规定开关的**读取时机**（每帧读 / 挂载时读 / 变化不监听）。
 * 因此本文件只覆盖「挂载前设置」的确定性路径，运行期热切换单列为 V2-4 并**明确标注为契约未定义**，
 * 不对未定义行为下断言。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";
import { useRuntimeStore } from "@/stores/runtime";
import {
  MD_VISIBLE_MARKERS,
  TAIL_SELECTOR,
  contentColumn,
  installDomStubs,
  legacyTailTotal,
  mdStreamSteps,
  mountWorkspace,
  patchAssistant,
  probeStreamNodes,
  setTailSwitch,
  settle,
  settleRender
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

async function mountStreamingWorkspace(initialContent: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  await patchAssistant(store, initialContent, "pending");
  const wrapper = mountWorkspace(pinia);
  await settleRender();
  return { wrapper, store, pinia };
}

describe("wave-pds-3 V2：尾巴回退开关（jsdom）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
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

  it("V2-1: 开关置 \"off\" 后 tail 数量恒为 0（内容增长全过程逐帧采样）", async () => {
    // Arrange
    setTailSwitch("off");
    const steps = mdStreamSteps();
    const { wrapper, store } = await mountStreamingWorkspace(steps[0]!);
    const samples: Array<{ step: number; phase: "sync" | "settled"; probe: StreamNodesProbe }> = [
      { step: 0, phase: "settled", probe: probeStreamNodes(contentColumn(wrapper)) }
    ];

    // Act
    for (let step = 1; step < steps.length; step += 1) {
      await patchAssistant(store, steps[step]!, "pending");
      await nextTick();
      samples.push({ step, phase: "sync", probe: probeStreamNodes(contentColumn(wrapper)) });
      await settle(2, 24);
      samples.push({ step, phase: "settled", probe: probeStreamNodes(contentColumn(wrapper)) });
    }

    // Assert（契约主体）
    const violations = samples
      .filter((entry) => entry.probe.tailCount !== 0)
      .map((entry) => ({ step: entry.step, phase: entry.phase, tailCount: entry.probe.tailCount }));
    expect(
      violations,
      `V2-1：开关为 "off" 时 [data-stream-tail] 数量必须恒为 0；违规采样：${JSON.stringify(violations)}`
    ).toHaveLength(0);

    // Assert 自检：关闭态不是「不渲染」而是「全部内容走 stableHtml」——正文必须完整可见
    const last = samples[samples.length - 1]!.probe;
    expect(last.bodyPresent, "V2-1 自检：关闭态仍必须有 .markdown-body 承载节点").toBe(true);
    expect(
      last.bodyTextLength,
      "V2-1 自检：关闭态正文必须完整渲染（不是白屏）"
    ).toBeGreaterThan(20);

    wrapper.unmount();
  });

  it("V2-2: 开关为 \"off\" 时，正文标记全部可见且无历史尾巴节点（行为等同 wave-59）", async () => {
    // Arrange
    setTailSwitch("off");
    const steps = mdStreamSteps();
    const lastStep = steps[steps.length - 1]!;
    const { wrapper } = await mountStreamingWorkspace(lastStep);

    // Act
    const column = contentColumn(wrapper);
    const visibleText = `${column?.textContent ?? ""}`;
    const probe = probeStreamNodes(column);

    // Assert（可见性：markdown 语法字符不计）
    const missing = MD_VISIBLE_MARKERS.filter((marker) => !visibleText.includes(marker));
    expect(
      missing,
      `V2-2：关闭态下正文必须完整可见，缺失标记：${JSON.stringify(missing)}`
    ).toHaveLength(0);

    // Assert（wave-59 语义：三类历史尾巴节点仍为 0）
    expect(
      legacyTailTotal(probe),
      `V2-2：关闭态不得出现 .mr-streaming-raw / .streaming-unrendered-suffix / .mr-streaming-char；实际：${JSON.stringify(probe.legacy)}`
    ).toBe(0);

    expect(probe.tailCount, "V2-2：关闭态不得渲染尾巴节点").toBe(0);

    wrapper.unmount();
  });

  it("V2-3: 关闭态等价性 —— 承载节点身份跨内容增长与 streaming→done 不变（wave-59 §F3 语义）", async () => {
    // Arrange
    setTailSwitch("off");
    const steps = mdStreamSteps();
    const { wrapper, store } = await mountStreamingWorkspace(steps[0]!);
    const carrier = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
    expect(carrier, "V2-3 自检：关闭态流式期间必须存在 .markdown-body 承载节点").not.toBeNull();
    const carrierElement = carrier as Element;

    // Act + Assert（内容增长）
    for (let step = 1; step < steps.length; step += 1) {
      await patchAssistant(store, steps[step]!, "pending");
      await nextTick();
      const syncFrame = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
      expect(
        syncFrame === carrierElement,
        `V2-3：关闭态第 ${step} 步同步帧的 .markdown-body 被替换（身份必须不变）`
      ).toBe(true);
      await settle(2, 24);
      const settledFrame = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
      expect(
        settledFrame === carrierElement,
        `V2-3：关闭态第 ${step} 步渲染落地后 .markdown-body 被重建（身份必须不变）`
      ).toBe(true);
    }

    // Act（streaming → done）
    await patchAssistant(store, steps[steps.length - 1]!, "done");
    await settleRender();
    const completed = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;

    // Assert
    expect(
      completed === carrierElement,
      "V2-3：关闭态 streaming→done 切换不得替换承载节点（wave-59 F3-3a 语义）"
    ).toBe(true);
    expect(
      `${(completed as Element | null)?.textContent ?? ""}`,
      "V2-3：完成帧必须含完整内容"
    ).toContain("尾段正在输出");

    wrapper.unmount();
  });

  it("V2-4: 开关值非 \"off\"（如 \"on\"）时尾巴仍开启 —— 只有精确 \"off\" 才关闭", async () => {
    // Arrange
    setTailSwitch("on");
    const steps = mdStreamSteps();
    const { wrapper } = await mountStreamingWorkspace(steps[steps.length - 1]!);

    // Act
    const probe = probeStreamNodes(contentColumn(wrapper));

    // Assert
    expect(probe.bodyTextLength, "V2-4 自检：内容必须已渲染").toBeGreaterThan(20);
    expect(
      probe.tailCount,
      `V2-4：值 "on" 不等于 "off" ⇒ 尾巴必须开启（数量 1），实际 ${probe.tailCount}`
    ).toBe(1);

    wrapper.unmount();
  });

  it("V2-5: 关闭态与开启态的正文文本一致（开关只影响尾巴节点，不影响可见文本）", async () => {
    // Arrange：同一内容分别以 off / on 挂载，比较内容区纯文本
    const steps = mdStreamSteps();
    const lastStep = steps[steps.length - 1]!;

    setTailSwitch("off");
    const off = await mountStreamingWorkspace(lastStep);
    const offText = (contentColumn(off.wrapper)?.textContent ?? "").replace(/\s+/g, "");
    off.wrapper.unmount();

    window.localStorage.clear();
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    setTailSwitch("on");
    const on = await mountStreamingWorkspace(lastStep);
    const onProbe = probeStreamNodes(contentColumn(on.wrapper));
    const onText = `${contentColumn(on.wrapper)?.textContent ?? ""}`.replace(/\s+/g, "");

    // Assert 自检：开启态确实渲染了尾巴（否则两边等价是必然的，用例无判别力）
    expect(
      onProbe.tailCount,
      "V2-5 自检：开启态必须存在尾巴节点，否则本用例退化为「无尾巴 vs 无尾巴」"
    ).toBe(1);

    // Assert：文本层面一致（尾巴文本本身也是正文的一部分）
    expect(
      onText,
      "V2-5：开/关两态的正文可见文本必须一致（开关不得吞字或重复吐字）"
    ).toBe(offText);

    on.wrapper.unmount();
  });

  it("V2-6: 关闭态下不得挂载任何 [data-stream-tail] 后代（含隐藏节点）", async () => {
    // Arrange：契约 §2「关闭 ⇒ 不渲染」，而不是「渲染后隐藏」——否则 DOM 协议与 V1 的计数失去意义
    setTailSwitch("off");
    const steps = mdStreamSteps();
    const { wrapper } = await mountStreamingWorkspace(steps[steps.length - 1]!);

    // Act
    const column = contentColumn(wrapper);

    // Assert
    expect(
      column?.querySelectorAll(TAIL_SELECTOR).length ?? 0,
      "V2-6：关闭态必须完全不渲染 [data-stream-tail]（不得以隐藏节点形式存在）"
    ).toBe(0);

    wrapper.unmount();
  });
});
