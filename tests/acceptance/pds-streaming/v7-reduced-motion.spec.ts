/**
 * wave-pds-3 · V7：reduced-motion 降级（双轨）—— jsdom
 *
 * 契约源（只读）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`
 *   §2（**v2**）：系统 `prefers-reduced-motion: reduce` 或**祖先** `data-pds-reduce-motion="true"` ⇒ **强制关闭**
 *      （不渲染尾巴 + 全部动画 `animation:none`）；开关读取为 **mount-only**（热切换不在本波范围）；
 *   §3.5：降级下所有 PDS 动画 `animation:none` / `transition:none`，`[data-stream-tail] { mask-image: none }`；
 *   §6 V7：reduced-motion 下 tail 不渲染 + 行为证据 + 双轨规则存在（jsdom）；
 *   §6 **V7-b**：Chromium 下 computed `animation-name === "none"` 且 `mask-image: none`（见 `e2e/*.pw.ts`）。
 *
 * ⚠️ 环境限制（实测，非托辞）：**V7 的字面判据 `animationName === "none"` 在 jsdom 中不可执行**。
 *   实测（jsdom 26 / cssstyle）：`getComputedStyle(el).getPropertyValue("animation-name")` 恒返回空串，
 *   `mask-image` 亦返回空串；只有 `animation` / `transition` **简写**与 `display` 可读。
 *   同时 jsdom 不实现 `window.matchMedia`，也不按媒体查询求解级联 —— 因此
 *   `@media (prefers-reduced-motion: reduce)` 内的声明无法通过 computed style 观测。
 *   ⇒ 本文件把「动画关闭」拆成两块**可判定**的证据：
 *     ① 行为证据（V7-1/V7-2）：降级下尾巴不渲染（DOM 层面可判定，且必须有「非降级态尾巴存在」的对照组，
 *        否则就是「0 个尾巴不含尾巴」的假绿）；
 *     ② 规则证据（V7-4/V7-5）：降级轨道（媒体查询 + 根属性）内必须存在 `animation:none` /
 *        `mask-image:none` 声明，且 `primitives.css` 必须被真正 import（当前从未被 import ⇒ 规则不生效）。
 *   真实动画行为由浏览器侧用例 `e2e/pds-streaming-visual.pw.ts`（V6/V8）覆盖。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import {
  MD_VISIBLE_MARKERS,
  REDUCE_MOTION_ATTR,
  collectCssPool,
  contentColumn,
  declaresMotionOff,
  findReducedMotionRules,
  installDomStubs,
  legacyTailTotal,
  mdStreamSteps,
  mountWorkspace,
  patchAssistant,
  probeStreamNodes,
  readRepoFile,
  setRootReduceMotion,
  setTailSwitch,
  settleRender,
  stripJsComments,
  stubReducedMotion
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

const FINAL_CONTENT = mdStreamSteps()[mdStreamSteps().length - 1]!;

async function mountStreaming() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  await patchAssistant(store, FINAL_CONTENT, "pending");
  const wrapper = mountWorkspace(pinia);
  await settleRender();
  return { wrapper, store };
}

describe("wave-pds-3 V7：reduced-motion 降级（jsdom）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    setTailSwitch(null);
    setRootReduceMotion(false);
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
    installDomStubs();
  });

  afterEach(() => {
    setRootReduceMotion(false);
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("V7-1: 系统 prefers-reduced-motion 下尾巴不渲染（含非降级对照组）", async () => {
    // Arrange：对照组 —— 非降级态必须先证明尾巴机制存在
    stubReducedMotion(false);
    const normal = await mountStreaming();
    const normalProbe = probeStreamNodes(contentColumn(normal.wrapper));
    normal.wrapper.unmount();

    expect(
      normalProbe.tailCount,
      "V7-1 对照组：非降级态必须渲染尾巴（数量 1）；否则「降级下为 0」是假绿（无尾巴自然为 0）"
    ).toBe(1);

    // Act：降级态
    stubReducedMotion(true);
    const reduced = await mountStreaming();
    const reducedProbe = probeStreamNodes(contentColumn(reduced.wrapper));

    // Assert
    expect(
      reducedProbe.tailCount,
      `V7-1：系统 reduced-motion 下不得渲染 [data-stream-tail]，实际 ${reducedProbe.tailCount}`
    ).toBe(0);
    expect(legacyTailTotal(reducedProbe), "V7-1：降级下同样不得出现历史尾巴节点").toBe(0);

    reduced.wrapper.unmount();
  });

  it("V7-2: 祖先 data-pds-reduce-motion=\"true\" 下尾巴不渲染（含非降级对照组）", async () => {
    // Arrange：对照组
    setRootReduceMotion(false);
    stubReducedMotion(false);
    const normal = await mountStreaming();
    const normalProbe = probeStreamNodes(contentColumn(normal.wrapper));
    normal.wrapper.unmount();
    expect(
      normalProbe.tailCount,
      "V7-2 对照组：未设根属性时必须渲染尾巴（数量 1）；否则本用例无判别力"
    ).toBe(1);

    // Act
    setRootReduceMotion(true);
    const reduced = await mountStreaming();
    const reducedProbe = probeStreamNodes(contentColumn(reduced.wrapper));

    // Assert
    expect(
      reducedProbe.tailCount,
      `V7-2：祖先 ${REDUCE_MOTION_ATTR}="true" 下不得渲染 [data-stream-tail]（契约 v2 §2），实际 ${reducedProbe.tailCount}`
    ).toBe(0);

    reduced.wrapper.unmount();
  });

  it("V7-3: 降级下全文可见（降级不等于丢内容）", async () => {
    // Arrange
    stubReducedMotion(true);
    const { wrapper } = await mountStreaming();

    // Act
    const column = contentColumn(wrapper);
    const visibleText = `${column?.textContent ?? ""}`;
    const probe = probeStreamNodes(column);

    // Assert
    const missing = MD_VISIBLE_MARKERS.filter((marker) => !visibleText.includes(marker));
    expect(
      missing,
      `V7-3：降级下正文必须完整可见（尾巴不渲染时内容必须全部走 stableHtml），缺失：${JSON.stringify(missing)}`
    ).toHaveLength(0);
    expect(probe.bodyTextLength, "V7-3 自检：降级下承载节点文本非空").toBeGreaterThan(20);

    wrapper.unmount();
  });

  it("V7-4: primitives.css 必须真正被 src 引用（否则 §3 全部动效与降级规则都不生效）", async () => {
    // Arrange：契约交付前提 —— 现状是「从未被 import」。
    const candidates = ["src/styles.css", "src/main.ts", "src/App.vue"];
    const references = candidates
      .map((path) => ({ path, code: stripJsComments(safeSource(path)) }))
      .filter((entry) => /streaming-motion\/(primitives|tokens)\.css/.test(entry.code))
      .map((entry) => entry.path);

    // Act + Assert
    expect(
      references,
      "V7-4：primitives.css / tokens.css 必须被 src 真正引用（@import 或 import），"
        + `实际扫描：${candidates.join(", ")}；命中：${JSON.stringify(references)}`
    ).not.toHaveLength(0);
  });

  it("V7-5: 降级双轨规则必须覆盖尾巴 / reasoning 尾巴 / 工具入场（animation|transition|mask-image: none）", async () => {
    // Arrange：契约 §3.5 强制双轨；三个选择器锚点均来自契约正文
    const pool = collectCssPool();
    const requiredTokens = [
      { token: "data-stream-tail", label: "正文尾巴（§3.1）" },
      { token: "assistant-streaming-fade", label: "reasoning 尾巴（§3.2）" },
      { token: "transition-item-enter", label: "工具行/面板入场（§3.3）" }
    ];

    // Act：按轨道逐个核对
    const missing: Array<Record<string, unknown>> = [];
    for (const required of requiredTokens) {
      for (const track of ["media", "attribute"] as const) {
        const hits = pool.flatMap((entry) =>
          findReducedMotionRules(entry.css, required.token)
            .filter((rule) => rule.track === track && declaresMotionOff(rule.body))
            .map((rule) => ({ path: entry.path, selector: rule.selector, body: rule.body }))
        );
        if (hits.length === 0) {
          missing.push({ token: required.token, label: required.label, track });
        }
      }
    }

    // Assert
    expect(
      missing,
      `V7-5：以下降级轨道缺少「关闭动效」声明（契约 §3.5）：${JSON.stringify(missing)}；`
        + "扫描来源：" + pool.map((entry) => entry.path).join(", ")
    ).toHaveLength(0);
  });

  it("V7-6: 降级下尾巴的渐变遮罩必须被关闭（mask-image: none 声明存在）", async () => {
    // Arrange
    const pool = collectCssPool();

    // Act
    const maskOffRules = pool.flatMap((entry) =>
      findReducedMotionRules(entry.css, "data-stream-tail")
        .filter((rule) => /(?:^|;)\s*mask-image\s*:\s*none\b/.test(rule.body))
        .map((rule) => ({ path: entry.path, track: rule.track, selector: rule.selector }))
    );

    // Assert
    expect(
      maskOffRules,
      "V7-6：降级下必须存在 `[data-stream-tail] { mask-image: none }` 的双轨声明（契约 §3.5）"
    ).not.toHaveLength(0);
  });

  it("V7-7: 降级下不得残留「渐变遮罩仍在、动画被关」的半降级（遮罩声明与开启动效互斥）", async () => {
    // Arrange：把「降级态仍保留 mask-image 渐变」列为违规——它会让尾巴在降级下依旧逐字浮现，
    // 与 prefers-reduced-motion 的语义冲突（契约 §3.5）。
    const pool = collectCssPool();
    const violations = pool.flatMap((entry) =>
      findReducedMotionRules(entry.css, "data-stream-tail")
        .filter((rule) => /mask-image\s*:\s*linear-gradient/.test(rule.body))
        .map((rule) => ({ path: entry.path, track: rule.track, body: rule.body }))
    );

    // Act + Assert
    expect(
      violations,
      `V7-7：降级轨道内不得保留 mask-image 渐变（半降级）：${JSON.stringify(violations)}`
    ).toHaveLength(0);
  });
});

function safeSource(relativePath: string): string {
  try {
    return readRepoFile(relativePath);
  } catch {
    return "";
  }
}
