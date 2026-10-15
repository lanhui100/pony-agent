/**
 * Wave-11 端到端稳定性验收 — 桌面端消息流下方状态条 PA-STATUS-FLOW
 *
 * 权威契约：`.dev-team/contracts/wave-11-status-flow.md` §1（M1–M6）/ §2（DOM 与行为契约）
 * UI-NFR：`.dev-team/nfr-baseline-ui-wave11.json`
 *
 * 本文件在真实 Chromium + vite preview（playwright.config.ts 自带 webServer @127.0.0.1:4175）
 * 中逐帧采样 `requestAnimationFrame`，采样结束后离线断言；不依赖任何固定 sleep 做竞态判定。
 * 基线（commit 018bcdc，`.dev-team/evidence/baseline-status-flow.json`）：
 *   合成长流 3s：状态条视口 Y 方向反转 44 次 / 极差 539px / 单帧最大位移 85.8px，
 *   且流式期间状态条仍然显示。因此 M1/M2 在当前实现上必须真实失败。
 */
import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team/evidence");
fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

const SLOT = '[data-testid="workspace-runtime-status-slot"]';
const BAR = '[data-testid="agent-runtime-status-bar"]';
const ICON = '[data-testid="agent-runtime-status-icon"]';
const CONTENT_COLUMN = '[data-testid="workspace-content-column"]';

/** NFR 冻结阈值（`.dev-team/nfr-baseline-ui-wave11.json`） */
const STATUS_ROW_Y_RANGE_MAX_PX = 1;
const STATUS_SLOT_HEIGHT_RANGE_MAX_PX = 1;
const CONTENT_REVERSALS_MAX = 1;
const CONTENT_SINGLE_FRAME_DISPLACEMENT_MAX_PX = 32;
const CONTENT_MONOTONIC_TOLERANCE_PX = 2;
const SHOW_HIDE_TRANSITION_BUDGET_MS = 400;
const ICON_ANIMATION_DURATION_RANGE_MS = [1200, 2400] as const;

interface StreamFrame {
  t: number;
  statusTop: number | null;
  statusHeight: number | null;
  statusVisible: string | null;
  statusOpacity: string | null;
  statusVisibility: string | null;
  statusAriaHidden: string | null;
  statusPointerEvents: string | null;
  slotAnimating: boolean;
  flowTop: number | null;
  articleTop: number | null;
  isSubmitting: boolean;
}

interface StreamHarness {
  frames: StreamFrame[];
  visibleFalseAt: number | null;
  opacityZeroAt: number | null;
  firstContentAt: number | null;
}

function collectConsoleErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  return errors;
}

async function openHome(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.evaluate(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  // index.html 预加载启动屏（#app-loading-screen）在 mount 后 3.35s 才淡出移除，
  // 会遮挡截图；采样与截图前必须等待其真实离场（避免视觉凭据失效）。
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });
}

async function patchRuntimeStore(page: Page, state: Record<string, unknown>) {
  await page.evaluate((patch) => {
    const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
    const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
    const pinia = globalProperties.globalProperties as Record<string, unknown>;
    const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    (stores.get("runtime") as { $patch: (value: unknown) => void }).$patch(patch);
  }, state);
}

function motionStats(values: number[], tolerancePx: number) {
  let reversals = 0;
  let lastSign = 0;
  let maxJump = 0;
  for (let i = 1; i < values.length; i += 1) {
    const delta = values[i] - values[i - 1];
    if (Math.abs(delta) <= tolerancePx) continue;
    const sign = Math.sign(delta);
    if (lastSign !== 0 && sign !== lastSign) reversals += 1;
    lastSign = sign;
    maxJump = Math.max(maxJump, Math.abs(delta));
  }
  return { reversals, maxJump };
}

function rangeOf(values: number[]) {
  return Math.max(...values) - Math.min(...values);
}

async function waitForAnimationFrames(page: Page, count: number) {
  await page.evaluate(
    (frames) =>
      new Promise<void>((resolve) => {
        let seen = 0;
        const tick = () => {
          seen += 1;
          if (seen >= frames) resolve();
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
    count
  );
}

/** 合成 3s 持续流式增长：patch pinia runtime store + 页内 rAF 逐帧采样。 */
async function startSyntheticStream(page: Page, lines: number, intervalMs: number) {
  await page.evaluate(
    ({ totalLines, interval }) => {
      const w = window as unknown as Record<string, any>;
      const appEl = document.querySelector("#app") as unknown as Record<string, any>;
      const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
      const store = pinia._s.get("runtime");
      const turnId = `w11-turn-${Date.now()}`;

      store.$patch({
        sessionId: "w11-e2e-session",
        sessionOperation: null,
        sessionError: null,
        error: null,
        phase: "calling_model",
        isSubmitting: true,
        activeTurnId: turnId,
        activeTurnStartedAtMs: Date.now(),
        toolActivities: [],
        streamBufferText: "",
        streamBufferReasoning: "",
        messages: [
          { id: `w11-user-${turnId}`, turnId, role: "user", content: "wave-11 合成 3s 流式稳定性诊断" },
          { id: `w11-assistant-${turnId}`, turnId, role: "assistant", content: "", status: "pending" }
        ]
      });

      const pick = (selector: string) => document.querySelector(selector) as HTMLElement | null;
      w.__w11 = {
        frames: [] as StreamFrame[],
        visibleFalseAt: null,
        opacityZeroAt: null,
        firstContentAt: null,
        growthFinished: false,
        stopped: false
      };

      const record = () => {
        const harness = w.__w11;
        if (harness.stopped) return;
        const slot = pick('[data-testid="workspace-runtime-status-slot"]');
        const flow = pick('[data-testid="assistant-streaming-flow"]');
        const article = pick('[data-testid="workspace-content-column"] article');
        const style = slot ? getComputedStyle(slot) : null;
        const visible = slot ? slot.getAttribute("data-visible") : null;
        const opacity = style ? style.opacity : null;
        const now = performance.now();
        if (visible === "false" && harness.visibleFalseAt === null) harness.visibleFalseAt = now;
        if (opacity === "0" && harness.opacityZeroAt === null) harness.opacityZeroAt = now;
        if (flow && harness.firstContentAt === null) harness.firstContentAt = now;
        harness.frames.push({
          t: now,
          statusTop: slot ? slot.getBoundingClientRect().top : null,
          statusHeight: slot ? slot.getBoundingClientRect().height : null,
          statusVisible: visible,
          statusOpacity: opacity,
          statusVisibility: style ? style.visibility : null,
          statusAriaHidden: slot ? slot.getAttribute("aria-hidden") : null,
          statusPointerEvents: style ? style.pointerEvents : null,
          slotAnimating: slot ? slot.getAnimations().length > 0 : false,
          flowTop: flow ? flow.getBoundingClientRect().top : null,
          articleTop: article ? article.getBoundingClientRect().top : null,
          isSubmitting: Boolean(store.isSubmitting)
        });
        harness.raf = requestAnimationFrame(record);
      };
      w.__w11.raf = requestAnimationFrame(record);

      const line = "这是一行用于 wave-11 状态流稳定性诊断的合成文本，长度接近真实回答的一行内容。\n";
      let emitted = 0;
      w.__w11.growth = setInterval(() => {
        emitted += 1;
        const target = store.messages.find((message: Record<string, unknown>) => message.role === "assistant");
        if (target) target.content += line;
        if (emitted >= totalLines) {
          clearInterval(w.__w11.growth);
          w.__w11.growthFinished = true;
        }
      }, interval);
    },
    { totalLines: lines, interval: intervalMs }
  );
}

async function readStreamHarness(page: Page): Promise<StreamHarness> {
  return page.evaluate(() => {
    const w = window as unknown as Record<string, any>;
    const harness = w.__w11;
    harness.stopped = true;
    cancelAnimationFrame(harness.raf);
    if (harness.growth) clearInterval(harness.growth);
    return {
      frames: harness.frames as StreamFrame[],
      visibleFalseAt: harness.visibleFalseAt as number | null,
      opacityZeroAt: harness.opacityZeroAt as number | null,
      firstContentAt: harness.firstContentAt as number | null
    };
  });
}

test.describe("Wave-11 状态条布局稳定性（真实浏览器）", () => {
  test("M1/M2/M3：3s 合成长流期间状态条视口 Y 零抖动、流式期隐藏、内容单调上移", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await openHome(page);

    await startSyntheticStream(page, 50, 60);
    await page.waitForFunction(
      () => (window as unknown as Record<string, any>).__w11?.firstContentAt !== null,
      undefined,
      { timeout: 20_000 }
    );

    await page.screenshot({ path: path.join(EVIDENCE_DIR, "status-flow-streaming-hidden.png") });

    await page.waitForFunction(
      () => (window as unknown as Record<string, any>).__w11?.growthFinished === true,
      undefined,
      { timeout: 20_000 }
    );
    await waitForAnimationFrames(page, 30);

    const harness = await readStreamHarness(page);
    expect(harness.frames.length).toBeGreaterThan(60);

    const streamFrames = harness.frames.filter((frame) => frame.isSubmitting && frame.statusTop !== null);
    expect(streamFrames.length).toBeGreaterThan(60);

    // ── M1：状态条视口 Y 极差 ≤ 1px、方向反转 = 0（稳态帧，排除显隐过渡的 translateY） ──
    const settledFrames = streamFrames.filter((frame) => !frame.slotAnimating);
    expect(settledFrames.length).toBeGreaterThan(30);
    const settledTops = settledFrames.map((frame) => frame.statusTop as number);
    expect(
      rangeOf(settledTops),
      `M1 状态条视口 Y 极差（${rangeOf(settledTops).toFixed(2)}px）必须 ≤ ${STATUS_ROW_Y_RANGE_MAX_PX}px`
    ).toBeLessThanOrEqual(STATUS_ROW_Y_RANGE_MAX_PX);
    expect(
      motionStats(settledTops, 0.6).reversals,
      "M1 状态条视口 Y 方向反转必须为 0（基线实测 44 次）"
    ).toBe(0);

    // NFR：状态条槽位高度在同一 viewport 下恒定
    const heights = streamFrames.map((frame) => frame.statusHeight as number);
    expect(rangeOf(heights)).toBeLessThanOrEqual(STATUS_SLOT_HEIGHT_RANGE_MAX_PX);

    // ── M2：流式期间 data-visible=false 且计算 opacity === "0"，过渡 ≤ 400ms ──
    expect(harness.visibleFalseAt, "M2 流式期间槽位必须进入 data-visible=false").not.toBeNull();
    expect(harness.opacityZeroAt, "M2 流式期间槽位计算 opacity 必须为 0").not.toBeNull();
    expect(
      (harness.opacityZeroAt as number) - (harness.visibleFalseAt as number),
      `M2 显隐过渡耗时必须 ≤ ${SHOW_HIDE_TRANSITION_BUDGET_MS}ms`
    ).toBeLessThanOrEqual(SHOW_HIDE_TRANSITION_BUDGET_MS);

    const lastStreamingFrame = streamFrames[streamFrames.length - 1];
    expect(lastStreamingFrame.statusVisible).toBe("false");
    expect(lastStreamingFrame.statusOpacity).toBe("0");
    expect(lastStreamingFrame.statusVisibility).toBe("hidden");
    expect(lastStreamingFrame.statusAriaHidden).toBe("true");
    expect(lastStreamingFrame.statusPointerEvents).toBe("none");

    // ── M3：流式内容逐帧单调上移（反转 ≤ 1，单帧位移 ≤ 32px） ──
    const flowSeries = harness.frames
      .filter((frame) => frame.isSubmitting && frame.flowTop !== null)
      .map((frame) => frame.flowTop as number);
    const articleSeries = harness.frames
      .filter((frame) => frame.isSubmitting && frame.articleTop !== null)
      .map((frame) => frame.articleTop as number);
    const motionSeries = flowSeries.length >= 30 ? flowSeries : articleSeries;
    expect(motionSeries.length).toBeGreaterThan(30);

    // 反假测试守卫：内容必须真实被上推，否则单调性断言无意义
    const upwardTravel = motionSeries[0] - motionSeries[motionSeries.length - 1];
    expect(upwardTravel, "M3 流式内容必须真实上移").toBeGreaterThan(200);

    const contentMotion = motionStats(motionSeries, CONTENT_MONOTONIC_TOLERANCE_PX);
    expect(contentMotion.reversals).toBeLessThanOrEqual(CONTENT_REVERSALS_MAX);
    expect(contentMotion.maxJump).toBeLessThanOrEqual(CONTENT_SINGLE_FRAME_DISPLACEMENT_MAX_PX);

    expect(consoleErrors, "浏览器 Console Error 必须为 0").toEqual([]);
  });

  test("M4：run 活跃期间插入新消息，状态条视口 Y 位移 ≤ 1px", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await openHome(page);

    const turnId = "w11-m4-turn";
    await patchRuntimeStore(page, {
      sessionId: "w11-m4-session",
      sessionOperation: null,
      sessionError: null,
      error: null,
      phase: "calling_tool",
      isSubmitting: true,
      activeTurnId: turnId,
      activeTurnStartedAtMs: Date.now(),
      toolActivities: [
        {
          id: "w11-m4-act",
          turnId,
          name: "read_file",
          canonicalToolName: "Read",
          displayNameZh: "读取文件",
          status: "running"
        }
      ],
      messages: [
        { id: "w11-m4-user-1", turnId, role: "user", content: "第一条用户消息" },
        { id: "w11-m4-assistant-1", turnId, role: "assistant", content: "", status: "pending" }
      ]
    });

    await expect(page.locator(SLOT)).toBeVisible();
    await expect(page.locator(SLOT)).toHaveAttribute("data-visible", "true");

    const result = await page.evaluate(
      async (baselineFrames) => {
        const w = window as unknown as Record<string, any>;
        const appEl = document.querySelector("#app") as unknown as Record<string, any>;
        const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
        const store = pinia._s.get("runtime");
        const pick = (selector: string) => document.querySelector(selector) as HTMLElement | null;
        const frames: Array<{ statusTop: number | null; slotAnimating: boolean }> = [];
        let stopped = false;

        const record = () => {
          if (stopped) return;
          const slot = pick('[data-testid="workspace-runtime-status-slot"]');
          frames.push({
            statusTop: slot ? slot.getBoundingClientRect().top : null,
            slotAnimating: slot ? slot.getAnimations().length > 0 : false
          });
          requestAnimationFrame(record);
        };
        requestAnimationFrame(record);

        const awaitFrames = (count: number) =>
          new Promise<void>((resolve) => {
            let seen = 0;
            const tick = () => {
              seen += 1;
              if (seen >= count) resolve();
              else requestAnimationFrame(tick);
            };
            requestAnimationFrame(tick);
          });

        await awaitFrames(baselineFrames);
        const beforeTop = frames[frames.length - 1]?.statusTop ?? null;

        // 运行中插入新 user turn（不改变 activeTurnId）
        const messages = store.messages.slice();
        messages.push({
          id: "w11-m4-user-2",
          turnId: "w11-m4-turn-2",
          role: "user",
          content: "运行中插入的第二条用户消息",
          status: "done"
        });
        store.$patch({ messages });

        await awaitFrames(45);
        stopped = true;
        const afterTop = frames[frames.length - 1]?.statusTop ?? null;
        return { frames, beforeTop, afterTop };
      },
      10
    );

    // 反假测试守卫：新消息必须真实渲染（新 turn section + 新用户消息文本）
    await expect(page.getByText("运行中插入的第二条用户消息")).toBeVisible();
    await expect(
      page.locator(`${CONTENT_COLUMN} section[data-turn-id="w11-m4-turn-2"]`)
    ).toHaveCount(1);

    const settledTops = result.frames
      .filter((frame) => frame.statusTop !== null && !frame.slotAnimating)
      .map((frame) => frame.statusTop as number);
    expect(settledTops.length).toBeGreaterThan(30);
    expect(
      rangeOf(settledTops),
      `M4 状态条视口 Y 极差（${rangeOf(settledTops).toFixed(2)}px）必须 ≤ ${STATUS_ROW_Y_RANGE_MAX_PX}px`
    ).toBeLessThanOrEqual(STATUS_ROW_Y_RANGE_MAX_PX);
    expect(Math.abs((result.afterTop as number) - (result.beforeTop as number))).toBeLessThanOrEqual(
      STATUS_ROW_Y_RANGE_MAX_PX
    );

    await page.screenshot({ path: path.join(EVIDENCE_DIR, "status-flow-new-message-stable.png") });
    expect(consoleErrors, "浏览器 Console Error 必须为 0").toEqual([]);
  });

  test("M5：orbit 纯 CSS 动效图标契约，且 prefers-reduced-motion 下 animation-name 为 none", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await openHome(page);

    const turnId = "w11-m5-turn";
    await patchRuntimeStore(page, {
      sessionId: "w11-m5-session",
      sessionOperation: null,
      sessionError: null,
      error: null,
      phase: "calling_tool",
      isSubmitting: true,
      activeTurnId: turnId,
      activeTurnStartedAtMs: Date.now(),
      toolActivities: [
        {
          id: "w11-m5-act",
          turnId,
          name: "read_file",
          canonicalToolName: "Read",
          displayNameZh: "读取文件",
          status: "running"
        }
      ],
      messages: [
        { id: "w11-m5-user-1", turnId, role: "user", content: "读取文件" },
        { id: "w11-m5-assistant-1", turnId, role: "assistant", content: "", status: "pending" }
      ]
    });

    const slot = page.locator(SLOT);
    await expect(slot).toHaveAttribute("data-visible", "true");
    const slotOpacity = await slot.evaluate((element) => getComputedStyle(element).opacity);
    expect(Number(slotOpacity)).toBeGreaterThanOrEqual(0.99);

    const icon = page.locator(ICON);
    await expect(icon).toBeVisible();

    const probe = await icon.evaluate((element) => {
      const root = element as HTMLElement;
      const descendants = Array.from(root.querySelectorAll("*")) as HTMLElement[];
      const nodes = [root, ...descendants];
      const animated = nodes.filter((node) => getComputedStyle(node).animationName !== "none");
      const dotByTestId = root.querySelector('[data-testid*="dot"]') as HTMLElement | null;
      const ringByTestId = root.querySelector('[data-testid*="ring"]') as HTMLElement | null;
      const dot = dotByTestId ?? animated[0] ?? null;
      const bar = root.closest('[data-testid="agent-runtime-status-bar"]') as HTMLElement | null;
      const barNodes = bar ? ([bar, ...Array.from(bar.querySelectorAll("*"))] as HTMLElement[]) : [];
      return {
        classes: Array.from(root.classList),
        directChildCount: root.children.length,
        descendantCount: descendants.length,
        animatedCount: animated.length,
        animatedDurations: animated.map((node) => getComputedStyle(node).animationDuration),
        dotTestId: dotByTestId ? dotByTestId.getAttribute("data-testid") : null,
        ringTestId: ringByTestId ? ringByTestId.getAttribute("data-testid") : null,
        dotAnimationName: dot ? getComputedStyle(dot).animationName : null,
        iconHasLegacyClass: nodes.some(
          (node) => node.classList.contains("snake-spinner") || node.classList.contains("animate-spin")
        ),
        barHasLegacyClass: barNodes.some(
          (node) => node.classList.contains("snake-spinner") || node.classList.contains("animate-spin")
        )
      };
    });

    expect(probe.classes, "M5 图标必须携带 runtime-status-orbit").toContain("runtime-status-orbit");
    expect(probe.directChildCount, "M5 orbit 图标必须包含 ring/dot 两个子节点").toBeGreaterThanOrEqual(2);
    expect(probe.iconHasLegacyClass, "M5 不得残留 snake-spinner/animate-spin").toBe(false);
    expect(probe.barHasLegacyClass, "M5 状态条内不得残留 snake-spinner/animate-spin").toBe(false);
    expect(probe.animatedCount, "M5 必须存在真实运行的 CSS 动画").toBeGreaterThanOrEqual(1);
    expect(probe.dotAnimationName, "M5 dot 节点必须存在且带 CSS 动画").not.toBeNull();
    expect(probe.dotAnimationName).not.toBe("none");

    const durations = probe.animatedDurations
      .flatMap((value) => value.split(","))
      .map((value) => value.trim())
      .filter((value) => value.length > 0);
    expect(durations.length).toBeGreaterThanOrEqual(1);
    for (const value of durations) {
      const seconds = value.endsWith("ms") ? Number.parseFloat(value) / 1000 : Number.parseFloat(value);
      expect(seconds).toBeGreaterThanOrEqual(ICON_ANIMATION_DURATION_RANGE_MS[0] / 1000);
      expect(seconds).toBeLessThanOrEqual(ICON_ANIMATION_DURATION_RANGE_MS[1] / 1000);
    }

    await page.screenshot({ path: path.join(EVIDENCE_DIR, "status-flow-orbit-icon.png") });

    // reduced-motion：所有图标动画必须为 none（前置已证明存在非 none 动画，故本断言非空转）
    await page.emulateMedia({ reducedMotion: "reduce" });
    const reducedAnimationNames = await icon.evaluate((element) => {
      const root = element as HTMLElement;
      const nodes = [root, ...Array.from(root.querySelectorAll("*"))] as HTMLElement[];
      return nodes.map((node) => getComputedStyle(node).animationName);
    });
    expect(reducedAnimationNames.length).toBeGreaterThan(1);
    expect(reducedAnimationNames.every((name) => name === "none")).toBe(true);

    expect(consoleErrors, "浏览器 Console Error 必须为 0").toEqual([]);
  });
});
