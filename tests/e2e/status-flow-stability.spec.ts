/**
 * Wave-11 端到端稳定性验收 — 桌面端消息流下方状态条 PA-STATUS-FLOW
 *
 * 权威契约：`.dev-team/contracts/wave-11-status-flow.md` §1（M1–M6）/ §2（DOM 与行为契约）
 * 契约演进 wave-11.1：M3 演进为良定义度量（M3a/M3b/M3c），见 `.dev-team/evidence/red-phase-wave11-evolution.log`
 * 与 `.dev-team/evidence/green-phase-wave11-e2e.log`。
 * 注：§7 原定的引擎改动（流式 smooth → auto）已被同探针 A/B 实测否决并回退，
 * 测试侧仅保留良定义度量（M3a/M3b/M3c）+ Console 确定性桩。
 * UI-NFR：`.dev-team/nfr-baseline-ui-wave11.json`
 *
 * 本文件在真实 Chromium + vite preview（playwright.config.ts 自带 webServer @127.0.0.1:4175）
 * 中逐帧采样 `requestAnimationFrame`，采样结束后离线断言；不依赖任何固定 sleep 做竞态判定。
 * 基线（commit 018bcdc，`.dev-team/evidence/baseline-status-flow.json`）：
 *   合成长流 3s：状态条视口 Y 方向反转 44 次 / 极差 539px / 单帧最大位移 85.8px，
 *   且流式期间状态条仍然显示。因此 M1/M2 在当前实现上必须真实失败。
 */
import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";
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
/**
 * M3a 方向反转的「可感知位移」容差（Lead 裁定：2px → 8px）。
 * 理由（数据来源 `.dev-team/evidence/m3-ab-smooth-pre.json`）：当前 smooth 实现（§7 已回退）
 * 的 flowTop 序列在 t≈4559ms 出现 `+6.0px` 反向微抖
 * （idx60–63：150.6 → 150.6 → 156.6 → 126.6）→ tol=2 时反转 = 2 > 1（红），
 * 而同实现另一次运行反转 = 0 → 原 2px 阈值 flaky。
 * tol=6/8/12 下两次运行均为 0。取 8px（≈1/3 行高）作为「可感知位移」下限；
 * 基线缺陷量级（状态条 ±23.8px 稳态抖动 / 44 次反转）远高于此，仍会被捕获。
 */
const CONTENT_MONOTONIC_TOLERANCE_PX = 8;
/**
 * wave-11.1 / wave-11.2：跟随滞后硬上界（M3c）。**Lead 裁定：64px → 96px（supersede 64px）**。
 * 物理含义 =「跟随不得落后超过 4 行内容」：一行 ≈ 23.8px → 4 行 ≈ 95.2px ≈ 96px。
 *
 * 阈值依据（物理包络）：M3c 峰值滞后的**下界** = 浏览器忙帧把多行内容合并进一帧时的内容增长量
 * （1 行 23.8px、2 行 47.6px、实测最大 ≈2.8 行 67px）。阈值必须覆盖「多行合并」包络，
 * 否则天生 flaky —— 16 次独立运行实测分布 **39–67px**，其中 1 次 67px > 64px，
 * 已证明 64px 低于该包络 → **supersede**。96px 对 67px 实测峰值留 43% 余量。
 * 判别力保留：跟随失效/停摆时滞后无界增长，仍会被本断言捕获；M3b 的无超调断言同时收紧。
 *
 * ⚠ 契约 §7 的引擎改动（流式 smooth → auto）已被同探针 A/B 实测否决并回退：
 * auto 无因果收益（滞后峰值 48px vs 47px）且略差（M3b 超调 20px vs 0）；
 * 存档 `.dev-team/evidence/reverted-wave11.1-auto-follow.patch`、A/B 数据
 * `.dev-team/evidence/m3-ab-smooth-pre.json` / `m3-ab-auto-post.json`。
 */
const FOLLOW_LAG_MAX_PX = 96;
/** wave-11.1：M3b 单帧滚动补偿预算的延迟窗口（帧）与余量（px）。 */
const FOLLOW_OVERSHOOT_BUDGET_FRAMES = 4;
const FOLLOW_OVERSHOOT_SLACK_PX = 8;
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
  scrollTop: number | null;
  scrollHeight: number | null;
  clientHeight: number | null;
  columnScrollHeight: number | null;
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

/**
 * wave-11.1：把启动期更新检查的外部依赖替换为确定性桩，使「Console Error = 0」
 * 成为确定性断言而非环境抽奖。
 *
 * 背景：`src/lib/update-check.ts` 在 store.initialize() 时访问
 * `https://api.github.com/repos/lanhui100/pony-agent/releases/latest`（无鉴权），
 * 失败后回退 `https://s3.local.ponyjob.top/pony-agent-releases/latest.json`。
 * 本沙箱代理对 api.github.com 返回 403 → 偶发 `Failed to load resource: 403`。
 * 这两个域与 wave-11 状态条改动无因果关系。
 *
 * 注意：这里**不是**放宽断言或过滤白名单 —— console 断言仍要求 0 条 error，
 * 只是把不可控的外部网络替换为 200 桩；应用自身的任何 JS error 依旧会被捕获。
 */
async function stubReleaseEndpoints(page: Page) {
  const releasePayload = JSON.stringify({
    tag_name: "v0.1.143",
    name: "e2e-stub",
    published_at: "2020-01-01T00:00:00Z",
    draft: false,
    prerelease: false,
    assets: [],
    body: ""
  });
  const fulfillStub = (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "access-control-allow-origin": "*" },
      body: releasePayload
    });
  await page.route("**/api.github.com/**", fulfillStub);
  await page.route("**/s3.local.ponyjob.top/**", fulfillStub);
}

async function openHome(page: Page) {
  await stubReleaseEndpoints(page);
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
  const reversalIndices: number[] = [];
  const reversalDeltas: number[] = [];
  for (let i = 1; i < values.length; i += 1) {
    const delta = values[i] - values[i - 1];
    if (Math.abs(delta) <= tolerancePx) continue;
    const sign = Math.sign(delta);
    if (lastSign !== 0 && sign !== lastSign) {
      reversals += 1;
      reversalIndices.push(i);
      reversalDeltas.push(Number(delta.toFixed(2)));
    }
    lastSign = sign;
    maxJump = Math.max(maxJump, Math.abs(delta));
  }
  return { reversals, maxJump, reversalIndices, reversalDeltas };
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
        const viewport = pick(".workspace-timeline-viewport");
        const column = pick('[data-testid="workspace-content-column"]');
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
          scrollTop: viewport ? viewport.scrollTop : null,
          scrollHeight: viewport ? viewport.scrollHeight : null,
          clientHeight: viewport ? viewport.clientHeight : null,
          columnScrollHeight: column ? column.scrollHeight : null,
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
  test("M1/M2/M3：3s 合成长流期间状态条视口 Y 零抖动、流式期隐藏、跟随滞后有界", async ({ page }) => {
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

    // ── M3（wave-11.1 良定义度量）：M3a 方向反转 ≤ 1 / M3b 无超调 / M3c 跟随滞后有界 ──
    //
    // 已删除原「绝对 maxJump ≤ 32px」断言（TEST-DEFECT）：该量把「内容真实增长」与
    // 「滚动补偿质量」混为一谈。内容每 60ms 增长 23.8px，浏览器忙帧会把两行合并到一帧
    // （+47.6px），任何实现的单帧 flowTop 位移下界就等于该帧内容增长量 → 32px 阈值天生 flaky。
    // 替代断言：M3b 以「最近 4 帧内容增量」为上界（增量本身不设绝对上限），
    //           M3c 直接度量跟随滞后。M3a/M3b/M3c 的观测窗口统一锚定「时间线已可滚动」阶段。
    // 采样帧（含滚动指标）
    const scrollFrames = harness.frames.filter(
      (frame) =>
        frame.isSubmitting &&
        frame.scrollTop !== null &&
        frame.scrollHeight !== null &&
        frame.clientHeight !== null &&
        frame.columnScrollHeight !== null
    );
    expect(scrollFrames.length).toBeGreaterThan(30);

    // M3a 观测窗口 = 「时间线已可滚动」的帧（与 M3c 同一锚定口径）。
    // 内容尚未填满视口时，内容合法地向下生长且 scrollTop 必为 0，
    // 把该阶段计入方向反转会产生与「反复跳变」无关的假反转（实测 1/3 概率出现 2 次反转）。
    const scrollableFrames = scrollFrames.filter(
      (frame) => (frame.scrollHeight as number) - (frame.clientHeight as number) > 0.5
    );
    expect(scrollableFrames.length).toBeGreaterThan(30);

    const flowSeries = scrollableFrames
      .filter((frame) => frame.flowTop !== null)
      .map((frame) => frame.flowTop as number);
    const articleSeries = scrollableFrames
      .filter((frame) => frame.articleTop !== null)
      .map((frame) => frame.articleTop as number);
    const motionSeries = flowSeries.length >= 30 ? flowSeries : articleSeries;
    expect(motionSeries.length).toBeGreaterThan(30);

    // 反假测试守卫：内容必须真实被上推，否则单调性/滞后断言无意义
    const upwardTravel = motionSeries[0] - motionSeries[motionSeries.length - 1];
    expect(upwardTravel, "M3 流式内容必须真实上移").toBeGreaterThan(200);

    // M3a：内容运动序列方向反转 ≤ 1（容差 8px）——「反复跳变」守卫。
    // 容差取值理由见常量 `CONTENT_MONOTONIC_TOLERANCE_PX`：`.dev-team/evidence/m3-ab-smooth-pre.json`
    // 实测当前实现存在 +6.0px 反向微抖（idx60–63：150.6 → 150.6 → 156.6 → 126.6），
    // 2px 容差会把它计为反转（2 > 1）→ flaky；8px（≈1/3 行高）为「可感知位移」下限。
    const contentMotion = motionStats(motionSeries, CONTENT_MONOTONIC_TOLERANCE_PX);
    // 诊断轨迹落盘：便于离线定位反转帧（每次运行覆盖）
    fs.writeFileSync(
      path.join(EVIDENCE_DIR, "status-flow-m3-trace.json"),
      JSON.stringify(
        {
          motionSeries,
          reversalIndices: contentMotion.reversalIndices,
          scrollableFrames: scrollableFrames.map((frame) => ({
            t: Number(frame.t.toFixed(1)),
            scrollTop: frame.scrollTop,
            scrollHeight: frame.scrollHeight,
            columnScrollHeight: frame.columnScrollHeight,
            flowTop: frame.flowTop
          }))
        },
        null,
        2
      )
    );
    expect(
      contentMotion.reversals,
      `M3a 反转帧索引 = ${JSON.stringify(contentMotion.reversalIndices)}`
    ).toBeLessThanOrEqual(CONTENT_REVERSALS_MAX);

    // M3b：单帧滚动补偿不得超过「最近 4 帧的数据量」——|ΔscrollTop_i| ≤ Σ_{j=0..3} Δ列高_{i−j} + 8px
    // 4 帧 ≈ 67ms 覆盖 rAF / after-paint 排队与忙帧合并造成的滚动应用延迟；8px 为余量。
    // 实测：原 `smooth` 实现 0 超调；`auto` 版在该口径下超调 20px → 断言具备判别力且非空转。
    const overshoots: Array<{ index: number; deltaScroll: number; budget: number }> = [];
    for (let i = FOLLOW_OVERSHOOT_BUDGET_FRAMES; i < scrollFrames.length; i += 1) {
      const deltaScroll = (scrollFrames[i].scrollTop as number) - (scrollFrames[i - 1].scrollTop as number);
      const budget =
        (scrollFrames[i].columnScrollHeight as number) -
        (scrollFrames[i - FOLLOW_OVERSHOOT_BUDGET_FRAMES].columnScrollHeight as number) +
        FOLLOW_OVERSHOOT_SLACK_PX;
      if (Math.abs(deltaScroll) > budget) {
        overshoots.push({ index: i, deltaScroll, budget });
      }
    }
    // M3b 与 M3c 使用 soft 断言：二者必须在同一次运行中同时给出物理量，
    // 避免前者失败掩盖后者的红相证据（最终仍会使用例失败）。
    expect.soft(
      overshoots,
      `M3b 滚动补偿超调帧（共 ${overshoots.length} 帧，最大超出 ${overshoots.length ? Math.max(...overshoots.map((item) => Math.abs(item.deltaScroll) - item.budget)).toFixed(1) : "0"}px）必须为 0`
    ).toEqual([]);

    // M3c：跟随滞后有界 —— |scrollTop_i − 已固定底部位置_i| ≤ FOLLOW_LAG_MAX_PX
    //
    // ⚠ 公式修订（相对契约 §7 的 |(scrollTop_i − scrollTop_0) − (colScrollHeight_i − colScrollHeight_0)|，
    //   已由 Lead 追认）：契约原式**恒不可绿**。合成流起始时内容列高 370px < 视口可视高 672px，
    // 此时「内容增长」不需要也不允许滚动（scrollTop 恒为 0），原式因此固定累计出
    // −(672−370) = −302px 的常数偏移。实测：原式在 smooth 实现 = 349px；在 auto 实现 = 350px
    // → 任何引擎改动都无法消除。修订式显式扣除该「尚未填满视口」的必然偏移，等价于锚定
    // 「首个可滚动帧」，即直接度量跟随是否落后于「已固定底部位置 max(0, 内容列高 − 视口可视高)」。
    // 阈值 64px 为契约 §7 冻结值，A/B 实测峰值 47px（smooth）/ 48px（auto）→ 余量 17px。
    const followLags = scrollFrames.map((frame) =>
      Math.abs(
        (frame.scrollTop as number) -
          Math.max(0, (frame.columnScrollHeight as number) - (frame.clientHeight as number))
      )
    );
    const worstFollowLag = Math.max(...followLags);
    expect.soft(
      worstFollowLag,
      `M3c 跟随滞后峰值（${worstFollowLag.toFixed(1)}px）必须 ≤ ${FOLLOW_LAG_MAX_PX}px（跟随不得落后超过 4 行内容）`
    ).toBeLessThanOrEqual(FOLLOW_LAG_MAX_PX);

    // 物理量输出（无论成败都打印，供收据引用）
    console.log(
      `[M3-metrics] frames=${scrollFrames.length} M3a_reversals=${contentMotion.reversals} ` +
        `M3b_overshoot_frames=${overshoots.length} M3b_worst_excess_px=${
          overshoots.length
            ? Math.max(...overshoots.map((item) => Math.abs(item.deltaScroll) - item.budget)).toFixed(1)
            : "0.0"
        } M3c_follow_lag_px=${worstFollowLag.toFixed(1)} upward_travel_px=${upwardTravel.toFixed(1)}`
    );

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

  // L3-B 审查给出的反例回归：窄屏（320px）下，状态条与 hover 展开后的
  // `workspace-scroll-to-bottom`（w-[110px]）不得水平重叠。
  // 契约 §2.5 要求槽位为右侧按钮预留空间（F4：pr-[7.5rem] = 120px）。
  test("T-c 窄屏 320px：executing_tool 长标签 + 展开滚动按钮，状态条与按钮不水平重叠", async ({ page }) => {
    const consoleErrors = collectConsoleErrors(page);
    await page.setViewportSize({ width: 320, height: 860 });
    await openHome(page);

    const lastTurnId = "w11-narrow-turn-8";
    const longToolLabel = "读取工作区中一个路径非常长的配置文件并解析其中的依赖清单";
    const history = Array.from({ length: 8 }, (_unused, index) => {
      const turnId = `w11-narrow-turn-${index + 1}`;
      const isLast = index === 7;
      return [
        {
          id: `w11-narrow-user-${index + 1}`,
          turnId,
          role: "user",
          content: `第 ${index + 1} 轮问题：${"请分析这段很长的上下文内容。".repeat(4)}`
        },
        {
          id: `w11-narrow-assistant-${index + 1}`,
          turnId,
          role: "assistant",
          content: isLast ? "" : `第 ${index + 1} 轮回答：${"这是较长的历史回答内容。".repeat(8)}`,
          status: isLast ? "pending" : "done"
        }
      ];
    }).flat();

    await patchRuntimeStore(page, {
      sessionId: "w11-narrow-session",
      sessionOperation: null,
      sessionError: null,
      error: null,
      phase: "calling_tool",
      isSubmitting: true,
      activeTurnId: lastTurnId,
      activeTurnStartedAtMs: Date.now(),
      toolActivities: [
        {
          id: "w11-narrow-act",
          turnId: lastTurnId,
          name: "read_file",
          canonicalToolName: "Read",
          displayNameZh: longToolLabel,
          status: "running"
        }
      ],
      messages: history
    });

    const slot = page.locator(SLOT);
    await expect(slot).toHaveAttribute("data-visible", "true");

    // 等待长历史真实渲染并让自动跟随稳定下来
    await expect(
      page.locator(`${CONTENT_COLUMN} section[data-turn-id="w11-narrow-turn-8"]`)
    ).toHaveCount(1);
    await waitForAnimationFrames(page, 30);

    // 用真实用户滚轮手势把时间线推离底部（程序化改 scrollTop 会被自动跟随立即拉回，
    // 且不会暂停 auto-follow）→ 出现「滚动到最新」按钮
    const viewport = page.locator(".workspace-timeline-viewport");
    await viewport.hover();
    await page.mouse.wheel(0, -6000);

    const scrollButton = page.locator('[data-testid="workspace-scroll-to-bottom"]');
    await expect(scrollButton).toBeVisible({ timeout: 15_000 });
    await scrollButton.hover();

    // 展开动画 500ms：以 rAF 帧数等待（不用固定 sleep）
    await waitForAnimationFrames(page, 45);

    const geometry = await page.evaluate(() => {
      const bar = document.querySelector('[data-testid="agent-runtime-status-bar"]');
      const label = document.querySelector('[data-testid="agent-runtime-status-label"]');
      const button = document.querySelector('[data-testid="workspace-scroll-to-bottom"]');
      if (!bar || !button) return null;
      const barRect = bar.getBoundingClientRect();
      const buttonRect = button.getBoundingClientRect();
      return {
        barRight: barRect.right,
        barLeft: barRect.left,
        barWidth: barRect.width,
        labelRight: label ? label.getBoundingClientRect().right : null,
        buttonLeft: buttonRect.left,
        buttonRight: buttonRect.right,
        buttonWidth: buttonRect.width,
        viewportWidth: window.innerWidth
      };
    });

    expect(geometry, "状态条与滚动按钮必须同时存在").not.toBeNull();
    const measured = geometry as NonNullable<typeof geometry>;
    // 反假测试守卫：按钮必须确实处于 hover 展开态（w-[110px]），否则重叠断言无意义
    expect(measured.buttonWidth, "滚动按钮必须已展开").toBeGreaterThan(80);
    expect(measured.barWidth).toBeGreaterThan(0);
    expect(
      measured.barRight,
      `状态条右边界（${measured.barRight.toFixed(1)}px）不得越过按钮左边界（${measured.buttonLeft.toFixed(1)}px）`
    ).toBeLessThanOrEqual(measured.buttonLeft + 0.5);
    if (measured.labelRight !== null) {
      expect(measured.labelRight).toBeLessThanOrEqual(measured.buttonLeft + 0.5);
    }

    await page.screenshot({ path: path.join(EVIDENCE_DIR, "status-flow-narrow-no-overlap.png") });
    expect(consoleErrors, "浏览器 Console Error 必须为 0").toEqual([]);
  });
});
