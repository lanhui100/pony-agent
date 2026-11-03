/**
 * Wave-61 冻结契约（真实浏览器门禁 L2-T）— §4 G-4a..G-4e
 *
 * 契约源（只读）：`.dev-team/contracts/wave-61-status-panel.md` §1/§2/§3/§4 + §5.2（判别力补偿）。
 *
 * 为什么必须在真实 Chromium 上跑（与 `tests/acceptance/wave61/*.spec.ts` 的分工）：
 *   jsdom 不执行 Tailwind、不注入 SFC `<style>`，只能做 class/声明面断言；
 *   下列四件事**只能**在真实浏览器里证伪：
 *     ① 圆环的物理归属（`status-metrics-counts` 行内 vs composer）—— 需真实布局树；
 *     ② 入场动效是否真的跑起来（`getAnimations()` / `animationstart` 事件 + 关键帧数值）；
 *     ③ 两行 4 列列中心物理对齐 ≤0.5px；
 *     ④ console error / pageerror 全旅程为 0 + 截图落盘（物理凭据）。
 *
 * §5.2 判别力补偿（Lead 二轮裁决 R7）：
 *   栅格化后「相邻位点左边缘不漂移」（wave-17 C2-2c）天然成立，判别力被削弱。
 *   故本文件补一条**等价强度**的物理断言：同一 token 位点在 `9 / 12 / 1234` 三档文本下，
 *   位点**自身宽度 Δ ≤ 0.5px**（证明 `min-w` 预留真实生效，而非被栅格掩盖；见 G-4c-discriminant）。
 *
 * 独立性声明：
 *   本文件不依赖任何存量 e2e spec（存量 `tests/e2e/browser-preview.spec.ts` 有已知漂移），
 *   可单独运行：
 *     npm run build && npx playwright test tests/e2e/wave61-status-panel.spec.ts
 *
 * 夹具纪律：
 *   - 「有值」态必须走 pinia `$patch`（与 `seedTokenMetricFixture` 同构的注入路径），
 *     不得靠 UI 点击长时间链路制造数据；
 *   - 「空值」态必须是 fresh page（无任何 patch），否则 E-1..E-4 的空值断言不可信；
 *   - 启动闪屏 `#app-loading-screen` 必须显式等待从 DOM 移除后再断言/截图
 *     （全屏 fixed 遮罩会被 Playwright `toBeVisible()` 判为可见，是已知盲区）。
 *
 * 零假测试律：全部用例硬断言；无 skip、无吞异常、无硬编码 sleep（只用显式条件等待）。
 *
 * ── D7 用例缺陷修正（2026-11-03，Lead 复核 G-4 首轮红）───────────────────────────
 *   现象：G-4b 对 `status-metric-turn-count` 的 easing 断言失败，实测 `"linear"`。
 *   根因（用例缺陷，非 SUT）：CSS Animation 的 `animation-timing-function` 在 Web Animations API 中
 *     落到**每个关键帧**的 `easing`（`getKeyframes()[i].easing`），而 `effect.getTiming().easing` 恒为 `"linear"`。
 *   修正：缓动改读 `getKeyframes()[0].easing`；`duration` 仍读 `getTiming().duration`；并新增
 *     「关键帧 ≥2」前置断言。原意图（200ms + 精确缓动 + opacity/translateY/scale 语义）**未弱化**。
 *
 * ── §1 L-2b 增补（wave-61.1，用户追加诉求 + Lead 一票裁决）─────────────────────
 *   圆环后内联百分比：`[data-testid="status-context-usage-percent"]`，文本 `<pct>%`，位于 SVG 之后。
 *   本文件新增 `§L-2b(wave-61.1)`：真实浏览器注入 42000/200000 ⇒ 文本 `21%`、`toHaveCount(1)`、
 *   环后兄弟关系成立、无 `0%`、无 console error。
 *
 * ── D8 用例缺陷修正（2026-11-03，Lead 真实浏览器取证后裁决）─────────────────────
 *   现象：G-4c 计数行第 4 列（圆环位）rect.width 实测 59.41px ⇒ `widthSpread` 超阈。
 *   根因（**非实现缺陷**）：测量发生在**入场动画进行中**，圆环位 `scale(0.92)→1` 使
 *     `getBoundingClientRect().width` 随 transform 缩放（63.5×0.92≈58.42；动画结束后回到 63.5；
 *     `offsetWidth` 两态恒 64）。Lead 已取证。
 *   修正：新增 `waitForStatusPanelAnimationsSettled()`（等待 `animation.finished` 收敛，**禁止硬 sleep**），
 *     G-4c / §5.2(R7) / G-4e 测量与截图前均先收敛，并显式断言「仍 running 的动画数 = 0」。
 *   原意图不变：等列栅格 + 两行列中心对齐 ≤0.5px + `min-w` 预留真实生效。
 *
 * ── 契约 L-3 追加裁决 R8（2026-11-03，实现侧修）─────────────────────────────────
 *   栅格自动流式下隐藏位点会让后位点左移（工具位为 0 时圆环落到第 3 列）；实现已为 8 个位点加
 *   `col-start-1..4` 显式列位。本文件的 G-4c（列中心对齐 ≤0.5px）**不需要改** ——
 *   R8 正是「空值不渲染 × 两行对齐」的必要条件，G-4c 首轮红即为该缺陷的物理证据。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

/** §4 G-4e：证据落盘目录（契约点名的相对路径）。 */
const EVIDENCE_DIR = path.resolve(process.cwd(), "docs", "release", "evidence", "wave-61-status-panel");

const COUNTS_ROW = '[data-testid="status-metrics-counts"]';
const TOKENS_ROW = '[data-testid="status-metrics-tokens"]';
const RING = '[data-testid="status-context-usage"]';
const LEGACY_RING = '[data-testid="workspace-context-usage-circle"]';
const COMPOSER_SHELL = '[data-testid="workspace-composer-shell"]';

const COUNT_SLOT_TESTIDS = [
  "status-metric-turn-count",
  "status-metric-model-call-count",
  "status-metric-tool-call-count"
] as const;
const TOKEN_SLOT_TESTIDS = [
  "status-metric-input-tokens",
  "status-metric-output-tokens",
  "status-metric-cache-tokens",
  "status-metric-token-speed"
] as const;

/** 契约 §3 M-1 的动效参数（原文数值，不得由测试自行放宽）。 */
const EXPECTED_DURATION_MS = 200;
const EXPECTED_EASING = "cubic-bezier(0.3, 0.7, 0.4, 1)";

interface BrowserSignals {
  consoleErrors: string[];
  pageErrors: string[];
}

/** 契约 G-4d：收集器必须在 `page.goto` **之前**挂载，否则会漏掉启动期错误。 */
function attachSignalCollectors(page: Page): BrowserSignals {
  const signals: BrowserSignals = { consoleErrors: [], pageErrors: [] };
  page.on("console", (message) => {
    if (message.type() === "error") {
      signals.consoleErrors.push(message.text());
    }
  });
  page.on("pageerror", (error) => {
    signals.pageErrors.push(`${error.name}: ${error.message}`);
  });
  return signals;
}

/** 断言 G-4d（零 console error / 零未捕获异常）。 */
function expectNoBrowserErrors(signals: BrowserSignals): void {
  expect(
    signals.consoleErrors,
    `G-4d：浏览器 Console Error 必须为 0，实际 ${signals.consoleErrors.length} 条：\n${signals.consoleErrors.join("\n")}`
  ).toHaveLength(0);
  expect(
    signals.pageErrors,
    `G-4d：浏览器未捕获异常必须为 0，实际 ${signals.pageErrors.length} 条：\n${signals.pageErrors.join("\n")}`
  ).toHaveLength(0);
}

/**
 * 首页骨架：等 shell / composer 可见，并**显式等待启动闪屏从 DOM 移除**。
 * `src/main.ts` 在挂载后 `setTimeout(3000)` 加 `fade-out`，再 350ms 后 `splash.remove()`；
 * 闪屏是 `position:fixed; inset:0; z-index:999999` 全屏遮罩，被覆盖的元素仍会被判「可见」。
 */
async function openHome(page: Page): Promise<void> {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("app-layout-shell")).toBeVisible();
  await expect(page.getByTestId("home-layout-shell")).toBeVisible();
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await expect(
    page.locator("#app-loading-screen"),
    "启动闪屏必须在断言/截图前从 DOM 移除（全屏 fixed 遮罩会被 toBeVisible 判为可见）"
  ).toHaveCount(0);
}

/**
 * 「有值」态夹具（pinia `$patch`，与 wave-17 `seedTokenMetricFixture` 同构）：
 * - `turnTraceHistory` 一条已完成轮次（含 1 条 call_model + 1 条 call_tool）⇒ 轮次/模型调用/工具调用三位计数有值；
 * - `providerCallRecords[].contextInputTokens` ⇒ `contextDisplayTokens`（圆环输入口径）；
 * - `retrievedContext.sessionContext.contextWindowTokens` ⇒ 圆环分母（缺它圆环按 §2 E-3 不渲染）；
 * - 输出/缓存/生成速度沿用既有 per-call 口径（wave-54 C6）。
 */
async function seedStatusPanelFixture(page: Page): Promise<void> {
  await page.evaluate(() => {
    const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
    const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
    const pinia = globalProperties.globalProperties as Record<string, unknown>;
    const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    const runtime = stores.get("runtime") as { $patch: (value: unknown) => void };

    runtime.$patch({
      sessionId: "wave61-fixture-session",
      retrievedContext: {
        sessionContext: {
          contextWindowTokens: 200_000
        }
      },
      turnTraceHistory: [
        {
          turnId: "wave61-fixture-turn",
          sessionId: "wave61-fixture-session",
          title: "wave-61 状态面板物理夹具",
          phase: "completed",
          traceSteps: [],
          traceTimeline: [
            {
              id: "wave61-call-model",
              turnId: "wave61-fixture-turn",
              sequence: 1,
              kind: "call_model",
              label: "调用模型",
              state: "completed",
              inputTokens: 1234,
              outputTokens: 567,
              cacheHitInputTokens: 89,
              firstTokenLatencyMs: 321,
              turnDurationMs: 4200,
              durationMs: 4200
            },
            {
              id: "wave61-call-tool",
              turnId: "wave61-fixture-turn",
              sequence: 2,
              kind: "call_tool",
              label: "调用工具",
              state: "completed"
            }
          ],
          toolActivities: [],
          providerCallRecords: [
            {
              requestKind: "initial_request",
              providerSource: "provider_decision",
              providerMode: "live",
              contextInputTokens: 42_000,
              inputTokens: 1234,
              cacheHitInputTokens: 89,
              cacheMissInputTokens: 1145,
              reasoningTokens: 0,
              outputTokens: 567,
              totalTokens: 1801,
              firstTokenLatencyMs: 321,
              turnDurationMs: 4200,
              latencyKind: "provider_stream",
              prefixMutationReasons: []
            }
          ],
          inputTokens: 1234,
          cacheHitInputTokens: 89,
          outputTokens: 567,
          totalTokens: 1801,
          firstTokenLatencyMs: 321,
          turnDurationMs: 4200,
          updatedAt: 1000,
          sessionSummary: ""
        }
      ]
    });
  });

  // 条件等待（禁止硬编码 sleep）：等到 8 个位点全部落地且渲染出数字文本再开始物理断言。
  await page.waitForFunction(
    ({ countsRow, tokensRow, ring, countSlots, tokenSlots }) => {
      const countsEl = document.querySelector(countsRow);
      const tokensEl = document.querySelector(tokensRow);
      const ringEl = document.querySelector(ring);
      if (!countsEl || !tokensEl || !ringEl) {
        return false;
      }
      const countOk = countSlots.every((testid) => {
        const element = document.querySelector(`[data-testid="${testid}"]`);
        return Boolean(element) && /\d/.test(element?.textContent ?? "");
      });
      const tokenOk = tokenSlots.every((testid) => {
        const element = document.querySelector(`[data-testid="${testid}"]`);
        return Boolean(element) && /\d/.test(element?.textContent ?? "");
      });
      return countOk && tokenOk;
    },
    {
      countsRow: COUNTS_ROW,
      tokensRow: TOKENS_ROW,
      ring: RING,
      countSlots: [...COUNT_SLOT_TESTIDS],
      tokenSlots: [...TOKEN_SLOT_TESTIDS]
    },
    { timeout: 15_000 }
  );
}

interface CollectedAnimation {
  testid: string | null;
  animationName: string;
  duration: number | string | null;
  /**
   * `effect.getTiming().easing`。CSS Animation 下**恒为 `"linear"`**（仅作诊断信息保留，不作契约判定）——
   * 见下方 D7 用例缺陷记录：`animation-timing-function` 在 WAAPI 里落到**每个关键帧**的 `easing`。
   */
  timingEasing: string | null;
  /** 关键帧（保留断言需要的 easing / opacity / transform 字段）。 */
  keyframes: Array<{
    offset: number | null;
    easing: string | null;
    opacity: string | null;
    transform: string | null;
  }>;
}

/**
 * 在 `$patch` **之前**挂载 `animationstart` 收集器：动画只在其开始的瞬间可被
 * `element.getAnimations()` 精确捕获（结束后可能已从列表里消失），
 * 因此必须在真实事件回调里同步取样（这是"证明动画真的跑起来了"的物理证据）。
 */
async function installAnimationCollector(page: Page): Promise<void> {
  await page.evaluate(() => {
    const store = window as unknown as { __wave61Animations?: CollectedAnimationLocal[] };
    store.__wave61Animations = [];
    type CollectedAnimationLocal = {
      testid: string | null;
      animationName: string;
      duration: number | string | null;
      timingEasing: string | null;
      keyframes: Array<{
        offset: number | null;
        easing: string | null;
        opacity: string | null;
        transform: string | null;
      }>;
    };
    document.addEventListener(
      "animationstart",
      (event) => {
        const target = event.target as Element | null;
        if (!target || typeof target.getAnimations !== "function") {
          return;
        }
        const animationEvent = event as AnimationEvent;
        const animations = target
          .getAnimations()
          .filter((animation) => (animation as CSSAnimation).animationName === animationEvent.animationName);
        for (const animation of animations) {
          const effect = animation.effect as KeyframeEffect | null;
          const timing = effect?.getTiming();
          const computedKeyframes = effect?.getKeyframes?.() ?? [];
          store.__wave61Animations!.push({
            testid: target.getAttribute("data-testid"),
            animationName: animationEvent.animationName,
            duration: (timing?.duration ?? null) as number | string | null,
            timingEasing: (timing?.easing ?? null) as string | null,
            keyframes: computedKeyframes.map((keyframe) => ({
              offset: (keyframe as { offset?: number }).offset ?? null,
              // D7（用例缺陷修正）：CSS Animation 的 animation-timing-function 在 WAAPI 中
              // 落在**每个关键帧**的 easing 上；`getTiming().easing` 恒为 "linear"。
              easing: ((keyframe as { easing?: string }).easing ?? null) as string | null,
              opacity: ((keyframe as { opacity?: string }).opacity ?? null) as string | null,
              transform: ((keyframe as { transform?: string }).transform ?? null) as string | null
            }))
          });
        }
      },
      true
    );
  });
}

async function collectAnimations(page: Page): Promise<CollectedAnimation[]> {
  return page.evaluate(
    () => (window as unknown as { __wave61Animations?: CollectedAnimation[] }).__wave61Animations ?? []
  );
}

/**
 * 等待状态面板两行 8 个位点上的入场动画**全部收敛**后返回「仍 running 的动画数」（期望 0）。
 *
 * ── D8 用例缺陷修正（2026-11-03，Lead 真实浏览器取证后裁决）─────────────────────
 *   现象：G-4c 计数行第 4 列（圆环位）实测 `rect.width = 59.41px`，`widthSpread` 超阈。
 *   根因（**不是实现缺陷**）：测量发生在**入场动画进行中** —— 圆环位入场含 `scale(0.92)→1`，
 *     动画期 `getBoundingClientRect().width` 会被 transform 缩放（63.5 × 0.92 ≈ 58.42）。
 *     Lead 取证：圆环位 动画中 rect.width=58.42 / 动画结束后=63.5；`offsetWidth` 两态恒 64。
 *   修法（采用 A，禁止硬 sleep）：测量前用 Web Animations API 的 `finished` 收敛动画，
 *     并对「是否仍有 running 动画」做显式断言（把根因冻结成门禁，防止回归）。
 *   原意图不变：等列栅格 + 两行列中心对齐 ≤0.5px + `min-w` 预留真实生效。
 *   §5.2(R7) 的三档宽度测量同样必须在收敛后读取（会被同一 transform 污染）。
 */
async function waitForStatusPanelAnimationsSettled(page: Page): Promise<number> {
  return page.evaluate(
    async ({ countsRow, tokensRow, ring }) => {
      const roots = [countsRow, tokensRow, ring]
        .map((selector) => document.querySelector(selector))
        .filter((element): element is Element => element != null);
      if (roots.length === 0) {
        return -1;
      }
      const collectRunning = (): Animation[] =>
        roots
          .flatMap((root) => [root, ...Array.from(root.querySelectorAll("*"))])
          .flatMap((element) => element.getAnimations())
          .filter((animation) => animation.playState === "running");

      // 收敛循环（显式条件，无固定 sleep）：每个 running 动画 await 其 finished。
      for (let attempt = 0; attempt < 60; attempt += 1) {
        const running = collectRunning();
        if (running.length === 0) {
          return 0;
        }
        await Promise.all(running.map((animation) => animation.finished.catch(() => undefined)));
      }
      return collectRunning().length;
    },
    { countsRow: COUNTS_ROW, tokensRow: TOKENS_ROW, ring: RING }
  );
}

/** 缓动归一化比对：浏览器会把 `cubic-bezier(0.3, 0.7, 0.4, 1)` 序列化成带空格的规范形式。 */
function normalizeEasing(value: string | null): string {
  return (value ?? "").replace(/\s+/g, "").toLowerCase();
}

/**
 * 把关键帧的 transform 归一成 { translateY, scale }。
 * `getKeyframes()` 返回的是**计算值**：Chromium 既可能给出 `translateY(4px)` / `scale(0.92)`，
 * 也可能给出等价的 `matrix(...)`，两种都接受（数值等价，不是放宽契约）。
 */
function transformNumbers(value: string | null): { translateY: number | null; scale: number | null } {
  const text = (value ?? "").replace(/\s+/g, "");
  let translateY: number | null = null;
  let scale: number | null = null;

  const translateMatch = /translatey\((-?[\d.]+)px\)/i.exec(text);
  if (translateMatch) {
    translateY = Number(translateMatch[1]);
  }
  const scaleMatch = /scale\((-?[\d.]+)\)/i.exec(text);
  if (scaleMatch) {
    scale = Number(scaleMatch[1]);
  }
  const matrixMatch = /matrix\(([^)]+)\)/i.exec(text);
  if (matrixMatch) {
    const parts = matrixMatch[1]!.split(",").map((part) => Number(part.trim()));
    if (parts.length === 6) {
      scale = scale ?? parts[0]!;
      translateY = translateY ?? parts[5]!;
    }
  }
  return { translateY, scale };
}

/** 关键帧透明度归一（Chromium 给出 "0" / "1"；容忍 "0.0" 之类的等价写法）。 */
function opacityNumber(value: string | null): number | null {
  if (value == null) {
    return null;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

// ── G-4a：圆环物理归属 ─────────────────────────────────────────────────────

test("G-4a: 真实 Chromium 中圆环位于 status-metrics-counts 行内，且 composer 区域无圆环", async ({ page }) => {
  // Arrange：收集器先挂，再走真实旅程 + 夹具注入（有值态才有圆环）。
  const signals = attachSignalCollectors(page);
  await openHome(page);
  await seedStatusPanelFixture(page);

  // Assert 1（契约主体：圆环在计数行内，且全页唯一）
  await expect(
    page.locator(`${COUNTS_ROW} ${RING}`),
    "G-4a：圆环必须位于 [data-testid=\"status-metrics-counts\"] 计数行内（§1 L-1/L-2）"
  ).toHaveCount(1);
  await expect(page.locator(RING), "G-4a：圆环位在全页必须恰好 1 处").toHaveCount(1);

  // Assert 2（圆环位序：计数行第 4 个位点）
  const slotOrder = await page.evaluate(
    ({ countsRow }) => {
      const row = document.querySelector(countsRow);
      if (!row) {
        return [];
      }
      return Array.from(row.children).map((child) => child.getAttribute("data-testid"));
    },
    { countsRow: COUNTS_ROW }
  );
  expect(
    slotOrder,
    "G-4a：计数行 4 个位点顺序必须为 轮次 → 模型调用 → 工具调用 → 上下文圆环"
  ).toEqual([...COUNT_SLOT_TESTIDS, "status-context-usage"]);

  // Assert 3（契约主体：离开方 —— composer 内不得残留圆环的任何形态）
  await expect(
    page.locator(LEGACY_RING),
    "G-4a：composer 旧圆环 [data-testid=\"workspace-context-usage-circle\"] 必须整个消失（§1 L-1）"
  ).toHaveCount(0);
  await expect(
    page.locator(`${COMPOSER_SHELL} ${RING}`),
    "G-4a：composer 外壳内不得出现圆环新位点（唯一位点在状态面板计数行）"
  ).toHaveCount(0);
  const composerRingSvgCount = await page.evaluate(
    ({ composerShell }) => {
      const shell = document.querySelector(composerShell);
      if (!shell) {
        return -1;
      }
      return shell.querySelectorAll("svg circle[stroke-dashoffset]").length;
    },
    { composerShell: COMPOSER_SHELL }
  );
  expect(
    composerRingSvgCount,
    `G-4a：composer 内不得存在带 stroke-dashoffset 的圆环 SVG（返回 -1 表示 composer 外壳未找到）；实际 ${composerRingSvgCount}`
  ).toBe(0);

  // Assert 3b（契约 §1 L-2 圆环语义：可及名逐字保真）
  await expect(
    page.locator(`${RING}[aria-label="上下文用量"], ${RING} [aria-label="上下文用量"]`),
    'G-4a：圆环必须携带 aria-label="上下文用量"（§1 L-2 语义保真）'
  ).toHaveCount(1);

  // Assert 4（物理形态：双 circle + 进度环 dashoffset 可读）
  await expect(page.locator(`${RING} circle`), "G-4a：圆环必须是 SVG 双 circle（轨道 + 进度）").toHaveCount(2);
  const dashOffset = await page.locator(`${RING} circle`).nth(1).getAttribute("stroke-dashoffset");
  expect(dashOffset, "G-4a：进度 circle 必须携带 stroke-dashoffset").not.toBeNull();
  expect(Number(dashOffset), "G-4a：stroke-dashoffset 必须是有限数值").toBeGreaterThanOrEqual(0);

  expectNoBrowserErrors(signals);
});

// ── G-4b：空数据不渲染 + 有数据入场动画命中 ─────────────────────────────────

test("G-4b: 空数据 → 位点/DOM 不存在（count=0）；注入数据 → 位点出现且入场动画 getAnimations() 命中", async ({ page }) => {
  // Arrange：收集器先挂（导航前）；fresh page 无任何 patch ⇒ 空值态。
  const signals = attachSignalCollectors(page);
  await openHome(page);

  // Assert 1（空值态：§2 E-1..E-4 ⇒ 两行与圆环、位点全部不渲染）
  await expect(page.locator(COUNTS_ROW), "G-4b：空数据下计数行必须不渲染（§2 E-4）").toHaveCount(0);
  await expect(page.locator(TOKENS_ROW), "G-4b：空数据下 token 行必须不渲染（§2 E-4）").toHaveCount(0);
  await expect(page.locator(RING), "G-4b：空数据下圆环位必须不渲染（§2 E-3）").toHaveCount(0);
  for (const testid of [...COUNT_SLOT_TESTIDS, ...TOKEN_SLOT_TESTIDS]) {
    await expect(
      page.locator(`[data-testid="${testid}"]`),
      `G-4b：空数据下位点 ${testid} 必须整个 DOM 不渲染（禁止 — / 0 占位）`
    ).toHaveCount(0);
  }

  // Act：装动画收集器 → 注入有值夹具（pinia $patch）
  await installAnimationCollector(page);
  await seedStatusPanelFixture(page);

  // Assert 2（有值态：位点全部出现）
  await expect(page.locator(COUNTS_ROW), "G-4b：注入数据后计数行必须出现").toHaveCount(1);
  await expect(page.locator(TOKENS_ROW), "G-4b：注入数据后 token 行必须出现").toHaveCount(1);
  await expect(page.locator(RING), "G-4b：注入数据后圆环位必须出现").toHaveCount(1);

  // Assert 3（契约主体：入场动画真的跑起来了 —— animationstart + getAnimations 同步取样）
  const animations = await collectAnimations(page);
  const numericSlotAnimations = animations.filter((entry) =>
    [...COUNT_SLOT_TESTIDS, ...TOKEN_SLOT_TESTIDS].includes(String(entry.testid))
  );
  expect(
    numericSlotAnimations.length,
    `G-4b：数值位点从「不存在」→「有值」必须播放入场动画；实际捕获 ${animations.length} 条 animationstart（位点内 ${numericSlotAnimations.length} 条）：${JSON.stringify(animations.map((entry) => `${entry.testid}/${entry.animationName}/${entry.duration}/${entry.keyframes[0]?.easing ?? "(no-keyframe)"}`))}`
  ).toBeGreaterThan(0);

  const ringAnimations = animations.filter((entry) => entry.testid === "status-context-usage");
  expect(
    ringAnimations.length,
    `G-4b：圆环位必须有入场动画（§3 M-1：圆环位额外 scale(0.92)→1）；实际捕获：${JSON.stringify(animations.map((entry) => entry.testid))}`
  ).toBeGreaterThan(0);

  // Assert 4（契约参数：200ms + cubic-bezier(0.3, 0.7, 0.4, 1)）
  //
  // ── D7 用例缺陷修正（2026-11-03，Lead 复核 G-4 首轮红：`status-metric-turn-count` easing 实测 "linear"）──
  //   缺陷根因：原断言读 `effect.getTiming().easing`。对 **CSS Animation**，WAAPI 把
  //   `animation-timing-function` 解析到**每个关键帧**的 `easing` 字段（`getKeyframes()[i].easing`），
  //   而 `effect.getTiming().easing` 恒为 `"linear"` ⇒ 原断言在正确实现下必然失败（用例缺陷，非 SUT 缺陷）。
  //   修正：缓动改读 `getKeyframes()[0].easing`；时长仍读 `getTiming().duration`（=200ms，原本已通过）。
  //   原意图**未弱化**：仍精确断言 200ms 与 `cubic-bezier(0.3, 0.7, 0.4, 1)`，并额外要求
  //   「关键帧 ≥2 且首/末帧 opacity/transform 语义正确」（见 Assert 5）。
  for (const entry of [...numericSlotAnimations, ...ringAnimations]) {
    // Assert 4a（关键帧可读：首/末帧必须存在 —— easing 的承载面）
    expect(
      entry.keyframes.length,
      `G-4b：${entry.testid} 的入场关键帧必须可读且至少 2 帧（缓动承载在关键帧上）；实际 ${JSON.stringify(entry.keyframes)}（animation=${entry.animationName}）`
    ).toBeGreaterThanOrEqual(2);

    // Assert 4b（时长：取自 effect.getTiming().duration）
    expect(
      Number(entry.duration),
      `G-4b：${entry.testid} 入场动画时长必须为 ${EXPECTED_DURATION_MS}ms；实际 ${entry.duration}（animation=${entry.animationName}）`
    ).toBe(EXPECTED_DURATION_MS);

    // Assert 4c（缓动：取自 getKeyframes()[0].easing —— 见 D7 根因说明）
    const keyframeEasing = entry.keyframes[0]!.easing;
    expect(
      normalizeEasing(keyframeEasing),
      `G-4b：${entry.testid} 入场动画缓动必须为 ${EXPECTED_EASING}（WAAPI 口径：getKeyframes()[0].easing；`
        + `getTiming().easing 对 CSS Animation 恒为 "linear"，仅作诊断：${JSON.stringify(entry.timingEasing)}）；实际 "${keyframeEasing}"`
    ).toBe(normalizeEasing(EXPECTED_EASING));
  }

  // Assert 5（关键帧语义：opacity 0→1 + translateY(4px)→0）
  const sample = numericSlotAnimations[0]!;
  expect(sample.keyframes.length, `G-4b：${sample.testid} 的入场关键帧必须可读`).toBeGreaterThanOrEqual(2);
  const firstFrame = sample.keyframes[0]!;
  const lastFrame = sample.keyframes[sample.keyframes.length - 1]!;
  expect(
    opacityNumber(firstFrame.opacity),
    `G-4b：${sample.testid} 入场首帧 opacity 必须为 0；实际 ${JSON.stringify(sample.keyframes)}`
  ).toBe(0);
  expect(
    opacityNumber(lastFrame.opacity),
    `G-4b：${sample.testid} 入场末帧 opacity 必须为 1；实际 ${JSON.stringify(sample.keyframes)}`
  ).toBe(1);
  const firstTranslateY = transformNumbers(firstFrame.transform).translateY;
  expect(
    firstTranslateY,
    `G-4b：${sample.testid} 入场首帧必须 translateY(4px)；实际 transform=${firstFrame.transform}`
  ).not.toBeNull();
  expect(
    firstTranslateY as number,
    `G-4b：${sample.testid} 入场首帧必须 translateY(4px)；实际 transform=${firstFrame.transform}`
  ).toBeCloseTo(4, 2);
  const lastTranslateY = transformNumbers(lastFrame.transform).translateY;
  expect(
    lastTranslateY === null || Math.abs(lastTranslateY) < 0.01,
    `G-4b：${sample.testid} 入场末帧位移必须归零；实际 transform=${lastFrame.transform}`
  ).toBe(true);

  // Assert 6（圆环额外 scale(0.92)→1）
  const ringSample = ringAnimations[0]!;
  const ringFirstFrame = ringSample.keyframes[0]!;
  const ringFirstScale = transformNumbers(ringFirstFrame.transform).scale;
  expect(
    ringFirstScale,
    `G-4b：圆环位入场首帧必须 scale(0.92)；实际 transform=${ringFirstFrame.transform}（关键帧：${JSON.stringify(ringSample.keyframes)}）`
  ).not.toBeNull();
  expect(
    ringFirstScale as number,
    `G-4b：圆环位入场首帧必须 scale(0.92)；实际 transform=${ringFirstFrame.transform}`
  ).toBeCloseTo(0.92, 2);

  expectNoBrowserErrors(signals);
});

// ── §1 L-2b（wave-61.1 追补）：圆环后内联百分比 ──────────────────────────────

test("§L-2b(wave-61.1): 真实浏览器中圆环位内联百分比为 21%（42000/200000），唯一且无 console error", async ({ page }) => {
  // Arrange：收集器先挂（登录期错误也不放过），注入夹具 -> contextDisplayTokens=42000 / window=200000
  const signals = attachSignalCollectors(page);
  await openHome(page);
  await seedStatusPanelFixture(page);

  // Assert 1（锚点自检：圆环位与百分比节点都必须存在且唯一）
  await expect(page.locator(RING), "§L-2b：圆环位必须存在").toHaveCount(1);
  const percent = page.locator('[data-testid="status-context-usage-percent"]');
  await expect(
    percent,
    "§L-2b：圆环位内必须存在唯一的 [data-testid=\"status-context-usage-percent\"]（wave-61.1 追加：环后内联百分比）"
  ).toHaveCount(1);

  // Assert 2（契约主体：文本严格等于 42000/200000 ⇒ 21%）
  await expect(
    percent,
    "§L-2b：百分比文本必须为 21%（clamp(round(42000/200000*100),1,100)）"
  ).toHaveText("21%");

  // Assert 3（位置纪律：该文本是圆环 SVG 的后继兄弟 = 「环后」，且与圆环同属一个位点）
  const placement = await page.evaluate(({ ring }) => {
    const ringEl = document.querySelector(ring);
    const percentEl = document.querySelector('[data-testid="status-context-usage-percent"]');
    const svgEl = ringEl ? ringEl.querySelector("svg") : null;
    return {
      ringFound: ringEl != null,
      percentFound: percentEl != null,
      svgFound: svgEl != null,
      insideRing: ringEl != null && percentEl != null && ringEl.contains(percentEl),
      afterSvg: svgEl != null && percentEl != null && svgEl.nextElementSibling === percentEl,
      siblingTag: svgEl?.nextElementSibling?.getAttribute("data-testid") ?? null
    };
  }, { ring: RING });

  expect(placement.ringFound, "§L-2b：圆环位必须存在（位置断言前提）").toBe(true);
  expect(placement.percentFound, "§L-2b：百分比节点必须存在（位置断言前提）").toBe(true);
  expect(placement.svgFound, "§L-2b：圆环 SVG 必须存在（环后判断基准）").toBe(true);
  expect(
    placement.insideRing,
    "§L-2b：百分比必须渲染在圆环位内部（同一位点，不得漂到行级）"
  ).toBe(true);
  expect(
    placement.afterSvg,
    `§L-2b：百分比必须是圆环 SVG 的后继兄弟（环后）；实际 SVG 后继 = ${placement.siblingTag ?? "(无)"}`
  ).toBe(true);

  // Assert 4（圆环位整体文本亦包含百分比，且不出现 0% 空环文案）
  await expect(page.locator(RING), "§L-2b：圆环位整体文本必须含 21%").toContainText("21%");
  await expect(page.locator(RING), "§L-2b：不得出现 0% 空环文案（pct 下限 1）").not.toContainText("0%");

  // Assert 5（G-4d 同源：全程零 console error / 零未捕获异常）
  expectNoBrowserErrors(signals);
});

// ── G-4c：两行 4 列中心物理对齐 ≤0.5px ─────────────────────────────────────

test("G-4c: 两行同为 4 列时，上下 4 个列中心的 x 坐标差 ≤ 0.5px", async ({ page }) => {
  // Arrange
  const signals = attachSignalCollectors(page);
  await openHome(page);
  await seedStatusPanelFixture(page);

  // Act 1（D8）：等入场动画收敛后再测量 —— 动画期 `rect.width` 会被 `scale(0.92)` 缩放而失真。
  const runningAfterSettle = await waitForStatusPanelAnimationsSettled(page);
  expect(
    runningAfterSettle,
    `G-4c（D8）：列宽/列中心测量前，位点入场动画必须已全部收敛（仍有 ${runningAfterSettle} 条 running；`
      + "动画期 getBoundingClientRect 会被 transform 缩放污染，实测圆环位 58.42 vs 63.5）"
  ).toBe(0);

  // Act 2：一次 evaluate 内同步读取两行全部列中心（避免 Vue 重渲染导致读数漂移）
  const geometry = await page.evaluate(
    ({ countsRow, tokensRow }) => {
      const readRow = (selector: string) => {
        const row = document.querySelector(selector);
        if (!row) {
          return null;
        }
        return Array.from(row.children).map((child) => {
          const rect = child.getBoundingClientRect();
          return {
            testid: child.getAttribute("data-testid"),
            centerX: rect.left + rect.width / 2,
            width: rect.width
          };
        });
      };
      return { counts: readRow(countsRow), tokens: readRow(tokensRow) };
    },
    { countsRow: COUNTS_ROW, tokensRow: TOKENS_ROW }
  );

  // Assert 1（锚点自检：两行都必须存在且恰好 4 个子位点）
  expect(geometry.counts, "G-4c：计数行必须存在").not.toBeNull();
  expect(geometry.tokens, "G-4c：token 行必须存在").not.toBeNull();
  expect(geometry.counts!.length, "G-4c：计数行必须恰为 4 列（3 计数 + 圆环）").toBe(4);
  expect(geometry.tokens!.length, "G-4c：token 行必须恰为 4 列（输入/输出/缓存/速度）").toBe(4);

  // Assert 2（契约主体：逐列列中心物理对齐 ≤0.5px）
  for (let index = 0; index < 4; index += 1) {
    const countSlot = geometry.counts![index]!;
    const tokenSlot = geometry.tokens![index]!;
    const delta = Math.abs(countSlot.centerX - tokenSlot.centerX);
    expect(
      delta,
      `G-4c：第 ${index + 1} 列列中心必须对齐（≤0.5px）；计数行 ${countSlot.testid} centerX=${countSlot.centerX}，token 行 ${tokenSlot.testid} centerX=${tokenSlot.centerX}，差值 ${delta}px`
    ).toBeLessThanOrEqual(0.5);
  }

  // Assert 3（合理性哨兵：4 列宽度相等，否则"中心对齐"可能由两行同时退化得到）
  const widthSpread = (slots: Array<{ width: number }>) =>
    Math.max(...slots.map((slot) => slot.width)) - Math.min(...slots.map((slot) => slot.width));
  expect(
    widthSpread(geometry.counts!),
    `G-4c：计数行 4 列宽度必须相等（等列栅格哨兵）；实际宽度：${JSON.stringify(geometry.counts!.map((slot) => slot.width))}`
  ).toBeLessThanOrEqual(0.5);
  expect(
    widthSpread(geometry.tokens!),
    `G-4c：token 行 4 列宽度必须相等（等列栅格哨兵）；实际宽度：${JSON.stringify(geometry.tokens!.map((slot) => slot.width))}`
  ).toBeLessThanOrEqual(0.5);

  expectNoBrowserErrors(signals);
});

// ── §5.2 判别力补偿（R7）：min-w 预留真实生效 ─────────────────────────────

test("§5.2(R7): 同一 token 位点在 9 / 12 / 1234 三档文本下自身宽度 Δ ≤ 0.5px（min-w 预留真实生效）", async ({ page }) => {
  // Arrange
  const signals = attachSignalCollectors(page);
  await openHome(page);
  await seedStatusPanelFixture(page);
  await expect(page.locator(TOKENS_ROW)).toBeVisible();

  // Act 1（D8）：三档宽度测量必须在入场动画收敛后读取（否则 `scale(0.92)` 会污染 rect.width）。
  const runningAfterSettle = await waitForStatusPanelAnimationsSettled(page);
  expect(
    runningAfterSettle,
    `§5.2(R7)（D8）：位点自身宽度测量前，入场动画必须已全部收敛（仍有 ${runningAfterSettle} 条 running）`
  ).toBe(0);

  // Act 2：在真实布局里改写位点内数字文本节点，同步读取三档下的自身宽度与溢出量
  const probe = await page.evaluate(
    ({ tokensRow, tokenTestid }) => {
      const row = document.querySelector(tokensRow);
      const slot = row ? row.querySelector(`[data-testid="${tokenTestid}"]`) : null;
      if (!row || !slot) {
        return { found: false as const, reason: "token 行或目标位点不存在" };
      }
      const walker = document.createTreeWalker(slot, NodeFilter.SHOW_TEXT);
      let textNode: Text | null = null;
      while (walker.nextNode()) {
        const node = walker.currentNode as Text;
        if (/\d/.test(node.data)) {
          textNode = node;
        }
      }
      if (!textNode) {
        return { found: false as const, reason: "位点内没有数字文本节点（无法做位数三档探针）" };
      }
      const originalText = textNode.data;
      const samples = ["9", "12", "1234"];
      const widths: number[] = [];
      const overflows: number[] = [];
      for (const sample of samples) {
        textNode.data = sample;
        widths.push(slot.getBoundingClientRect().width);
        overflows.push(slot.scrollWidth - slot.clientWidth);
      }
      textNode.data = originalText;
      const rect = slot.getBoundingClientRect();
      return {
        found: true as const,
        originalText,
        widths,
        overflows,
        columnWidth: row.getBoundingClientRect().width / 4,
        rectWidth: rect.width
      };
    },
    { tokensRow: TOKENS_ROW, tokenTestid: TOKEN_SLOT_TESTIDS[0]! }
  );

  // Assert 1（锚点自检：探针必须真的改写到了位点数字文本，否则断言恒真）
  expect(
    probe.found,
    `§5.2(R7)：位点数字文本探针必须生效；失败原因：${probe.found === false ? probe.reason : "(未知)"}`
  ).toBe(true);
  const measured = probe as Extract<typeof probe, { found: true }>;

  // Assert 2（契约主体：三档自身宽度 Δ ≤ 0.5px —— 等价替代被栅格掩盖的 C2-2c）
  const spread = Math.max(...measured.widths) - Math.min(...measured.widths);
  expect(
    spread,
    `§5.2(R7)：9 / 12 / 1234 三档下位点自身宽度漂移必须 ≤ 0.5px（证明 min-w 预留生效）；实测宽度 ${JSON.stringify(measured.widths)}，Δ=${spread}px，列宽=${measured.columnWidth}px`
  ).toBeLessThanOrEqual(0.5);

  // Assert 3（内容不换行/不溢出：最宽档不得超过位点内宽）
  const maxOverflow = Math.max(...measured.overflows);
  expect(
    maxOverflow,
    `§5.2(R7)：三档文本不得溢出位点宽度（否则会换行/挤压）；实测溢出 ${JSON.stringify(measured.overflows)}`
  ).toBeLessThanOrEqual(0.5);

  expectNoBrowserErrors(signals);
});

// ── G-4d：console error = 0 + pageerror = 0 ───────────────────────────────

test("G-4d: 首页加载 + 数据注入 + 状态面板渲染全程 Console Error = 0 且未捕获异常 = 0", async ({ page }) => {
  // Arrange：收集器必须在 page.goto 之前挂载（attachSignalCollectors 内即注册在 page 上）。
  const signals = attachSignalCollectors(page);

  // Act：完整旅程 —— 加载首页 → 等闪屏移除 → 注入有值夹具 → 位点/圆环落地。
  await openHome(page);
  await seedStatusPanelFixture(page);
  await expect(page.locator(COUNTS_ROW)).toHaveCount(1);
  await expect(page.locator(TOKENS_ROW)).toHaveCount(1);
  await expect(page.locator(RING)).toHaveCount(1);

  // Assert：零 console.error、零未捕获异常。
  expectNoBrowserErrors(signals);
});

// ── G-4e：关键旅程截图落盘 ─────────────────────────────────────────────────

test("G-4e: 关键旅程截图落盘 docs/release/evidence/wave-61-status-panel/（存在且 > 10KB）", async ({ page }) => {
  // Arrange
  const signals = attachSignalCollectors(page);
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

  // Act 1：进入首页并等待闪屏移除（否则截图只会拍到全屏遮罩）。
  await openHome(page);
  await expect(page.locator("#app-loading-screen")).toHaveCount(0);

  // Act 2：有值态（状态面板 8 个位点 + 圆环全部可见）
  await seedStatusPanelFixture(page);
  await expect(page.locator(COUNTS_ROW)).toBeVisible();
  await expect(page.locator(TOKENS_ROW)).toBeVisible();
  await expect(page.locator(RING)).toBeVisible();
  // D8 同源要求：截图取**稳态**（入场动画已收敛），避免拍到 scale(0.92) 的中间帧。
  expect(
    await waitForStatusPanelAnimationsSettled(page),
    "G-4e（D8）：截图前入场动画必须已收敛（否则会拍到 scale(0.92) 中间帧）"
  ).toBe(0);
  await expect(page.getByTestId("home-right-sidebar-shell")).toHaveAttribute("data-open", "true");

  // Act 3：截图（全页 + 视口两张，互为佐证）
  const fullPagePath = path.join(EVIDENCE_DIR, "wave61-status-panel-fullpage.png");
  const viewportPath = path.join(EVIDENCE_DIR, "wave61-status-panel-viewport.png");
  await page.screenshot({ path: fullPagePath, fullPage: true });
  await page.screenshot({ path: viewportPath });

  // Assert 1（物理文件必须存在且 > 10KB —— 不能只断言"截图函数没抛错"）
  for (const screenshotPath of [fullPagePath, viewportPath]) {
    expect(fs.existsSync(screenshotPath), `G-4e：截图必须落盘：${screenshotPath}`).toBe(true);
    const sizeBytes = fs.statSync(screenshotPath).size;
    expect(
      sizeBytes,
      `G-4e：截图必须非空（> 10KB），实际 ${sizeBytes} 字节：${screenshotPath}`
    ).toBeGreaterThan(10 * 1024);
  }

  // Assert 2（截图后复核：遮罩不得重新出现，面板仍在视口内）
  await expect(page.locator("#app-loading-screen"), "G-4e：截图后启动闪屏仍不得存在于 DOM").toHaveCount(0);
  await expect(page.locator(RING)).toBeInViewport();

  expectNoBrowserErrors(signals);
});
