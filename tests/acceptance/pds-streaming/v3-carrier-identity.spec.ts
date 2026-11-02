/**
 * wave-pds-3 · V3：`.markdown-body` 承载节点身份不变 —— jsdom
 *
 * 契约源（只读）：`.dev-team/contracts/wave-pds-3-streaming-visual.md` §0 + §6 V3
 *   §0：`F3-2b / F3-3a / F3-4`（`.markdown-body` 承载节点身份不变）**保持**，且「改为更强断言（见 V3）」；
 *   §6 V3：`.markdown-body` 承载节点跨内容增长与 streaming→done 身份不变（引用相等）。
 *
 * 「更强」体现在两处（本波新增）：
 *   ① V3-1/V3-2 保留引用相等的原始判据；
 *   ② V3-3 追加「尾巴出现前后」的判据 —— 尾巴接入的常见错误实现是把承载节点连同尾巴一起
 *      重新渲染（`v-html` + 新节点），这会让引用变化而 F3-2b 之外无人发现。
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
  mdStreamSteps,
  mountWorkspace,
  patchAssistant,
  probeStreamNodes,
  settleRender,
  settle
} from "./pds-streaming-harness";

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

async function mountStreaming(initialContent: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  await patchAssistant(store, initialContent, "pending");
  const wrapper = mountWorkspace(pinia);
  await settleRender();
  return { wrapper, store };
}

function carrier(wrapper: ReturnType<typeof mountWorkspace>): Element | null {
  return contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
}

describe("wave-pds-3 V3：承载节点身份不变（jsdom）", () => {
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

  it("V3-1: 内容增长过程中 .markdown-body 引用不变（同步帧 + 落地帧双采样）", async () => {
    // Arrange
    const steps = mdStreamSteps();
    const { wrapper, store } = await mountStreaming(steps[0]!);
    const initial = carrier(wrapper);
    expect(initial, "V3-1 自检：流式首个渲染周期后必须存在 .markdown-body").not.toBeNull();
    const carrierElement = initial as Element;

    // Act + Assert
    for (let step = 1; step < steps.length; step += 1) {
      await patchAssistant(store, steps[step]!, "pending");
      await nextTick();
      expect(
        carrier(wrapper) === carrierElement,
        `V3-1：第 ${step} 步同步帧 .markdown-body 被替换（必须是同一 DOM 引用）`
      ).toBe(true);

      await settle(2, 24);
      expect(
        carrier(wrapper) === carrierElement,
        `V3-1：第 ${step} 步渲染落地后 .markdown-body 被重建（必须是同一 DOM 引用）`
      ).toBe(true);
    }

    expect(`${(carrier(wrapper) as Element | null)?.textContent ?? ""}`, "V3-1 自检：最终内容必须可见")
      .toContain("尾段正在输出");

    wrapper.unmount();
  });

  it("V3-2: streaming→done 切换后 .markdown-body 引用不变且完成帧内容完整", async () => {
    // Arrange
    const steps = mdStreamSteps();
    const { wrapper, store } = await mountStreaming(steps[steps.length - 1]!);
    const streamingCarrier = carrier(wrapper);
    expect(streamingCarrier, "V3-2 自检：流式期间必须存在 .markdown-body").not.toBeNull();
    const carrierElement = streamingCarrier as Element;

    // Act
    await patchAssistant(store, steps[steps.length - 1]!, "done");
    await settleRender();

    // Assert
    const completed = carrier(wrapper);
    expect(
      completed === carrierElement,
      "V3-2：streaming→done 切换不得替换承载节点（引用必须相等）"
    ).toBe(true);

    const completedText = `${completed?.textContent ?? ""}`;
    const missing = MD_VISIBLE_MARKERS.filter((marker) => !completedText.includes(marker));
    expect(missing, `V3-2：完成帧必须含完整正文，缺失：${JSON.stringify(missing)}`).toHaveLength(0);

    wrapper.unmount();
  });

  it("V3-3: 尾巴节点出现前后，承载节点不得被重建（本波新增的更强断言）", async () => {
    // Arrange
    const steps = mdStreamSteps();
    const { wrapper, store } = await mountStreaming(steps[0]!);
    const before = carrier(wrapper);
    expect(before, "V3-3 自检：必须存在承载节点").not.toBeNull();
    const carrierElement = before as Element;

    const carrierRefs: Array<Element | null> = [];
    const tailCounts: number[] = [];

    // Act：逐步增长，逐帧记录「承载节点引用 + 尾巴数量」
    for (let step = 0; step < steps.length; step += 1) {
      if (step > 0) {
        await patchAssistant(store, steps[step]!, "pending");
        await nextTick();
      }
      for (let round = 0; round < 3; round += 1) {
        await settle(1, 16);
        carrierRefs.push(carrier(wrapper));
        tailCounts.push(probeStreamNodes(contentColumn(wrapper)).tailCount);
      }
    }

    // Assert 自检（关键）：必须先观测到尾巴出现，否则「出现前后身份不变」无从谈起
    const firstTailIndex = tailCounts.findIndex((count) => count >= 1);
    expect(
      firstTailIndex,
      `V3-3 自检：采样窗口内必须出现 [data-stream-tail]（实际每帧尾巴数量：${JSON.stringify(tailCounts)}）；`
        + "若尾巴从未出现，本用例对实现没有判别力"
    ).toBeGreaterThanOrEqual(0);

    // Assert（契约主体）：尾巴出现前后所有帧的承载节点均为同一引用
    const replacedAt = carrierRefs
      .map((ref, index) => ({ index, replaced: ref !== carrierElement }))
      .filter((entry) => entry.replaced)
      .map((entry) => entry.index);
    expect(
      replacedAt,
      `V3-3：尾巴出现前后 .markdown-body 不得被替换；被替换的帧序号：${JSON.stringify(replacedAt)}`
    ).toHaveLength(0);

    // Assert 自检 ②：尾巴必须与承载节点同处一个内容区（不是被搬到别处渲染）
    const column = contentColumn(wrapper);
    expect(
      column?.querySelectorAll(TAIL_SELECTOR).length ?? 0,
      "V3-3：尾巴必须位于内容区内（与 .markdown-body 同区）"
    ).toBe(1);

    wrapper.unmount();
  });
});
