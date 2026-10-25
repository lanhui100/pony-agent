/**
 * Wave-44 契约验收（L2-T / L2-AT）— PA-117：[workspace-scroll] 调试输出必须被开关门控
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave44.md` §1 PA117-T1/T2、§2.3、§3。
 *
 * 缺陷事实（A12）：
 *   `src/components/HomeWorkspace.vue:211` `console.info("[workspace-scroll]", payload)` **无开关**；
 *   payload 由 `collectTimelineScrollMetrics()`（`:155-157`）构造，进而触发 6+ 次
 *   `getBoundingClientRect()`（`useTimelineAutoScroll.ts:147-181`），且 hook 的 `emit()`
 *   在调用 `showDebug` 前**又**算一遍 metrics（`:184-190`）⇒ 每个调试事件两次布局读数。
 *   A14 要求「测试/DEV 态默认开」，因此开关语义必须是「DEV 兜底 + 显式覆盖」，不能一刀切删日志。
 *
 * 契约开关优先级（§2.3，运行时求值、不缓存）：
 *   ① localStorage `pony-agent.debug.workspace-scroll === "false"` → 关（kill switch）
 *   ② `=== "true"` → 开
 *   ③ URL `?debug=1` 或 `?debug=scroll` → 开
 *   ④ 否则 `import.meta.env.DEV` 真值 → 开，否则关
 *
 * 覆盖（契约断言 id 对照）：
 *   PA117-T1（L2-T，默认关闭）：`DEV=false` + 无开关 ⇒ 触发同一 debug 事件序列后
 *     `[workspace-scroll]` console.info 计数 **0**、`__ponyScrollDebugBuffer` 不增长 /
 *     `__ponyScrollDebugLatest === undefined`、且该序列内 `getBoundingClientRect` 调用数
 *     **严格小于**启用态（差 ≥1）；差异由「同 fixture 两态各跑一遍」机械给出。
 *   PA117-T2（L2-AT，开关矩阵）：localStorage `"true"`（DEV=false）⇒ 输出恢复且 payload 带真实度量；
 *     localStorage `"false"`（DEV=true）⇒ kill switch 生效（0 输出）；URL `?debug=1` 与
 *     `?debug=scroll`（DEV=false）⇒ 开启。
 *   PA117-T3（L2-AT 对抗，运行时不缓存）：同一已挂载实例内 关 → 开 → 关 必须即时生效
 *     （禁止模块加载期缓存开关）。
 *
 * 零假测试律：无 skip/todo；三条用例均含"关闭态必须静默"的真实缺陷断言，故修复前必须全红；
 * T2 的开启态断言用于阻止「直接删掉 console.info / 删掉 metrics 采集」式假修复。
 *
 * 作者：test-agent（L2-AT）。红相锚定后本文件冻结只读，任何修改须 Lead 批准。
 */
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, nextTick, watch } from "vue";
import { mount } from "@vue/test-utils";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useProviderStore } from "@/stores/providers";
import type { ProviderRegistry } from "@/types/provider";

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

/** 契约 §2.3 ②：本波新增的专用调试开关（workspace-scroll 通道）。 */
const WORKSPACE_SCROLL_DEBUG_KEY = "pony-agent.debug.workspace-scroll";
const WORKSPACE_SCROLL_TAG = "[workspace-scroll]";

// ── 挂载壳（与 tests/HomeWorkspace.spec.ts 同构的最小 stub 集）────────────

let latestViewportEl: HTMLElement | null = null;
const scrollToBottomSpy = vi.fn();

const ScrollAreaStub = defineComponent({
  setup(_props, { slots, expose }) {
    const viewportEl = document.createElement("div");
    latestViewportEl = viewportEl;
    expose({ viewportEl, scrollToBottom: scrollToBottomSpy });
    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

const MarkdownRendererStub = defineComponent({
  props: {
    content: { type: String, default: "" },
    streaming: { type: Boolean, default: false },
    wrapperClass: { type: String, default: "" },
    toneClass: { type: String, default: "" },
    forceMarkdownStreaming: { type: Boolean, default: false }
  },
  emits: ["render-complete"],
  setup(props, { emit }) {
    watch(
      () => [props.content, props.streaming] as const,
      ([content, streaming]) => {
        window.setTimeout(() => {
          emit("render-complete", { contentLength: content.length, streaming });
        }, 0);
      },
      { immediate: true }
    );
    return () => h("div", { class: "markdown-stub" }, props.content);
  }
});

const ButtonStub = defineComponent({
  props: {
    disabled: { type: Boolean, default: false },
    title: { type: String, default: "" }
  },
  emits: ["click"],
  setup(props, { emit, slots }) {
    return () =>
      h(
        "button",
        {
          class: "button-stub",
          type: "button",
          disabled: props.disabled,
          title: props.title,
          onClick: () => emit("click")
        },
        slots.default ? slots.default() : []
      );
  }
});

function createProviderRegistry(): ProviderRegistry {
  return {
    selectedProviderId: "provider-openai",
    providers: [
      {
        id: "provider-openai",
        name: "OpenAI",
        protocol: "openai",
        baseUrl: "https://api.openai.com/v1",
        apiKeyEnvVar: "OPENAI_API_KEY",
        apiKeyValue: "",
        apiKeyPresent: false,
        selectedModelId: "model-gpt5",
        models: [
          {
            id: "model-gpt5",
            name: "GPT-5",
            model: "gpt-5",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: null,
            reasoningBudgetTokens: null,
            capabilityPreset: "open-ai-reasoning",
            capabilities: {
              contextWindowTokens: 128000,
              supportsTools: true,
              supportsStreaming: true,
              supportsImageInput: false,
              supportsReasoning: true
            }
          }
        ]
      }
    ]
  } as unknown as ProviderRegistry;
}

function mountWorkspace() {
  const providerStore = useProviderStore();
  providerStore.$patch({ registry: createProviderRegistry(), selectedReasoningEffort: null });

  return mount(
    {
      render() {
        return h(TooltipProvider, null, { default: () => h(HomeWorkspace) });
      }
    },
    {
      global: {
        directives: {
          motion: { mounted() {}, updated() {}, beforeUpdate() {} }
        },
        stubs: {
          ScrollArea: ScrollAreaStub,
          MarkdownRenderer: MarkdownRendererStub,
          Button: ButtonStub,
          Transition: true,
          TransitionGroup: true
        }
      }
    }
  );
}

async function advanceAnimationFrames(count = 3): Promise<void> {
  for (let index = 0; index < count; index += 1) {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  }
}

// ── 计量探针 ────────────────────────────────────────────────────────────

type RectProbe = { total: number };

function installRectProbe(): RectProbe {
  const probe: RectProbe = { total: 0 };
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function () {
    probe.total += 1;
    return {
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
      width: 0,
      height: 0,
      x: 0,
      y: 0,
      toJSON() {
        return {};
      }
    } as DOMRect;
  });
  return probe;
}

function scrollDebugPayloads(): Array<Record<string, unknown>> {
  return (console.info as unknown as { mock: { calls: unknown[][] } }).mock.calls
    .filter((call) => call[0] === WORKSPACE_SCROLL_TAG)
    .map((call) => (call[1] ?? {}) as Record<string, unknown>);
}

function resetConsoleInfoCalls(): void {
  (console.info as unknown as { mock: { calls: unknown[][] } }).mock.calls.length = 0;
}

function readScrollDebugBuffer(): { defined: boolean; length: number; latest: unknown } {
  const debugWindow = window as typeof window & {
    __ponyScrollDebugBuffer?: Array<Record<string, unknown>>;
    __ponyScrollDebugLatest?: Record<string, unknown>;
  };
  return {
    defined: Array.isArray(debugWindow.__ponyScrollDebugBuffer),
    length: debugWindow.__ponyScrollDebugBuffer?.length ?? 0,
    latest: debugWindow.__ponyScrollDebugLatest
  };
}

function clearScrollDebugBuffer(): void {
  const debugWindow = window as typeof window & {
    __ponyScrollDebugBuffer?: Array<Record<string, unknown>>;
    __ponyScrollDebugLatest?: Record<string, unknown>;
  };
  delete debugWindow.__ponyScrollDebugBuffer;
  delete debugWindow.__ponyScrollDebugLatest;
}

/** 一次确定性的滚动调试触发序列（viewport wheel → 同步派生多条 debug 事件）。 */
function triggerScrollDebugSequence(times = 1): void {
  if (!latestViewportEl) {
    throw new Error("viewport is not mounted");
  }
  for (let index = 0; index < times; index += 1) {
    latestViewportEl.dispatchEvent(new Event("wheel"));
  }
}

type SwitchOptions = {
  dev: boolean;
  storage: "true" | "false" | null;
  url?: string;
};

/**
 * 在指定开关组合下挂载组件并执行同一触发序列，返回该序列的物理测量值。
 * 每次测量独立挂载/卸载，保证两态差分是「同 fixture、同序列」。
 */
async function measureSequence(options: SwitchOptions): Promise<{
  logs: number;
  payloads: Array<Record<string, unknown>>;
  rectTotal: number;
  buffer: { defined: boolean; length: number; latest: unknown };
}> {
  vi.stubEnv("DEV", options.dev);
  window.localStorage.removeItem(WORKSPACE_SCROLL_DEBUG_KEY);
  if (options.storage != null) {
    window.localStorage.setItem(WORKSPACE_SCROLL_DEBUG_KEY, options.storage);
  }
  window.history.replaceState({}, "", options.url ?? "/");
  clearScrollDebugBuffer();
  resetConsoleInfoCalls();

  const rectProbe = installRectProbe();
  const wrapper = mountWorkspace();
  await nextTick();
  await advanceAnimationFrames(3);
  rectProbe.total = 0;
  resetConsoleInfoCalls();

  triggerScrollDebugSequence();
  await nextTick();

  const result = {
    logs: scrollDebugPayloads().length,
    payloads: scrollDebugPayloads(),
    rectTotal: rectProbe.total,
    buffer: readScrollDebugBuffer()
  };

  wrapper.unmount();
  vi.restoreAllMocks();
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.spyOn(console, "error").mockImplementation(() => {});
  return result;
}

describe("PA-117 · [workspace-scroll] 调试输出开关门控", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    window.history.replaceState({}, "", "/");
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    scrollToBottomSpy.mockClear();
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllEnvs();
    document.body.innerHTML = "";
    window.localStorage.clear();
    window.history.replaceState({}, "", "/");
    latestViewportEl = null;
  });

  it("PA117-T1: 默认关闭（DEV=false + 无开关）时零日志、零缓冲写入，且布局读数严格少于启用态", async () => {
    // Arrange/Act：同 fixture、同序列，两态各跑一遍。
    const offState = await measureSequence({ dev: false, storage: null });
    const onState = await measureSequence({ dev: false, storage: "true" });

    // Assert 1：关闭态 console.info 必须为 0。
    expect(offState.logs).toBe(0);

    // Assert 2：关闭态不得写 debug 缓冲（`__ponyScrollDebugBuffer` 不增长 / latest 未定义）。
    // 契约措辞为「不增长」，故只断言长度与 latest，不锁死数组是否被创建。
    expect(offState.buffer.length).toBe(0);
    expect(offState.buffer.latest).toBeUndefined();

    // Assert 3：关闭态该序列的布局读数必须严格小于启用态（差 ≥1）。
    expect(offState.rectTotal).toBeLessThan(onState.rectTotal);
    expect(onState.rectTotal - offState.rectTotal).toBeGreaterThanOrEqual(1);

    // Assert 4：启用态是有效参照（否则差分无意义）——必须确有输出且确有测量。
    expect(onState.logs).toBeGreaterThan(0);
    expect(onState.rectTotal).toBeGreaterThan(0);
  });

  it("PA117-T2 (L2-AT 开关矩阵): localStorage true/false 与 URL ?debug=1|scroll 必须按契约优先级生效", async () => {
    // ① localStorage "true"（DEV=false）⇒ 开启，且 payload 带真实度量。
    const storageOn = await measureSequence({ dev: false, storage: "true" });
    expect(storageOn.logs).toBeGreaterThan(0);
    expect(storageOn.rectTotal).toBeGreaterThan(0);
    const firstPayload = storageOn.payloads[0]!;
    expect(typeof firstPayload.event).toBe("string");
    expect(String(firstPayload.event).length).toBeGreaterThan(0);
    expect(firstPayload).toHaveProperty("distanceToBottom");
    expect(firstPayload).toHaveProperty("viewportResolved");
    expect(firstPayload).toHaveProperty("composerTop");

    // ② localStorage "false"（DEV=true）⇒ kill switch 优先于 DEV 兜底，必须关闭。
    const storageOff = await measureSequence({ dev: true, storage: "false" });
    expect(storageOff.logs).toBe(0);
    expect(storageOff.buffer.length).toBe(0);

    // ③ URL ?debug=1（DEV=false）⇒ 开启。
    const urlDebugOne = await measureSequence({ dev: false, storage: null, url: "/?debug=1" });
    expect(urlDebugOne.logs).toBeGreaterThan(0);

    // ④ URL ?debug=scroll（DEV=false）⇒ 开启。
    const urlDebugScroll = await measureSequence({ dev: false, storage: null, url: "/?debug=scroll" });
    expect(urlDebugScroll.logs).toBeGreaterThan(0);
  });

  it("PA117-T3 (L2-AT 对抗): 同一已挂载实例内 关→开→关 必须即时生效（开关不得被缓存）", async () => {
    // Arrange：DEV=false + 无开关，挂载一次，后续只切换开关（不重新挂载）。
    vi.stubEnv("DEV", false);
    window.localStorage.removeItem(WORKSPACE_SCROLL_DEBUG_KEY);
    window.history.replaceState({}, "", "/");
    clearScrollDebugBuffer();
    resetConsoleInfoCalls();

    const rectProbe = installRectProbe();
    const wrapper = mountWorkspace();
    await nextTick();
    await advanceAnimationFrames(3);

    // Act 1（关闭态，缺陷断言）
    rectProbe.total = 0;
    resetConsoleInfoCalls();
    triggerScrollDebugSequence();
    await nextTick();

    // Assert 1：关闭态必须静默。
    expect(scrollDebugPayloads()).toHaveLength(0);

    // Act 2：同一实例内打开开关（localStorage "true"），再次触发。
    window.localStorage.setItem(WORKSPACE_SCROLL_DEBUG_KEY, "true");
    resetConsoleInfoCalls();
    triggerScrollDebugSequence();
    await nextTick();

    // Assert 2：必须立即恢复输出（不依赖重新挂载）。
    expect(scrollDebugPayloads().length).toBeGreaterThan(0);

    // Act 3：同一实例内关闭（显式 "false"），再次触发。
    window.localStorage.setItem(WORKSPACE_SCROLL_DEBUG_KEY, "false");
    resetConsoleInfoCalls();
    rectProbe.total = 0;
    triggerScrollDebugSequence();
    await nextTick();

    // Assert 3：必须立即恢复静默 + 零布局读数。
    expect(scrollDebugPayloads()).toHaveLength(0);
    expect(rectProbe.total).toBe(0);

    wrapper.unmount();
  });
});
