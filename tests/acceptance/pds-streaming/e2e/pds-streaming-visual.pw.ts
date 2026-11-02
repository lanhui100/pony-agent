/**
 * wave-pds-3 · V4 / V5 / V6 / V7-b / V8 / V9：真 Chromium 逐帧与降级验收
 *
 * 契约源（只读）：`.dev-team/contracts/wave-pds-3-streaming-visual.md`（**v3 裁定**，§5 + §6）
 *   V4 真浏览器逐帧 `scrollHeight` 不回撤 > 1px；
 *   V5 正文首帧非空后无空白帧；
 *   V6 流式全程 tail 高度 ≤ 行高 × 1.2，数量恒 ≤1；
 *   V7-b 降级三轨 computed `animation-name === "none"` 且 `mask-image: none`；
 *   V8 `p95_stream − p95_idle ≤ 6ms` 且 Long Task(>50ms)=0，**连续 3 次运行全部满足**才算通过；
 *   V9 Console Error = 0。
 *
 * 占位流参数（v3 裁定 ①，**默认值即契约值**，环境变量仅供调试；失败消息会回显实际取值）：
 *   delta 间隔 120ms、单次 chunk ≤ 50 字符、流式采样 900 帧（≥800）、空闲基线 320 帧（≥300）。
 *
 * 为什么必须真浏览器：jsdom 无布局（没有 scrollHeight / getBoundingClientRect / 真实帧节奏），
 * 这三项的判据在 jsdom 里**不存在**，任何"jsdom 版 V4/V5/V6/V8"都是伪测试。
 *
 * 采样方式（可复现）：页内 `requestAnimationFrame` 逐帧记录 `performance.now()` 与几何量，
 * 采样结束后**离线断言**；不使用固定 sleep 判定竞态（沿用 `tests/e2e/wave59-stream-stability.spec.ts`）。
 * 采样窗口：900 帧（≈15s @60Hz，满足 V8 的 ≥800 帧下限），期间合成 markdown 流持续增长。
 *
 * 运行方式（本目录自带 playwright 配置，不改仓库根配置）：
 *   npx playwright test --config tests/acceptance/pds-streaming/e2e/playwright.pds.config.ts
 * 前置：`npm run build`（webServer 用 `vite preview` 提供 dist）。
 */
import { expect, test } from "@playwright/test";
import type { Browser, Page, Route } from "@playwright/test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** V4：逐帧 scrollHeight 单帧回撤容差（契约 V4）。 */
const SCROLL_RETRACT_TOLERANCE_PX = 1;

/**
 * ── 占位流参数（契约 v3 裁定 ①）────────────────────────────────────────
 * 这些是**契约值**：默认值即契约值，环境变量只作调试用（调试值会回显在断言消息里，
 * 便于发现"用调试参数跑出的绿"）。
 */
const CONTRACT_CHUNK_INTERVAL_MS = 120;
/** 单次 delta 的字符上限（契约值）。 */
const CONTRACT_MAX_CHUNK_CHARS = 50;
/** 流式采样帧数（契约值；契约下限为 ≥800）。 */
const CONTRACT_STREAM_FRAMES = 900;
/** 空闲基线采样帧数（契约值；契约下限为 ≥300）。 */
const CONTRACT_IDLE_FRAMES = 320;

const CHUNK_INTERVAL_MS = Number(process.env.PDS_E2E_CHUNK_MS ?? String(CONTRACT_CHUNK_INTERVAL_MS));
const STREAM_FRAMES = Number(process.env.PDS_E2E_STREAM_FRAMES ?? String(CONTRACT_STREAM_FRAMES));
const IDLE_FRAMES = Number(process.env.PDS_E2E_IDLE_FRAMES ?? String(CONTRACT_IDLE_FRAMES));
/** 调试用开关：`PDS_E2E_STREAM=0` 时只采样不追加内容（仅用于排查，不参与 V8 判定）。 */
const STREAM_ENABLED = process.env.PDS_E2E_STREAM !== "0";
/** 环境变量是否偏离契约值（偏离时断言消息会显式标注）。 */
const USING_CONTRACT_PARAMS = CHUNK_INTERVAL_MS === CONTRACT_CHUNK_INTERVAL_MS
  && STREAM_FRAMES === CONTRACT_STREAM_FRAMES
  && IDLE_FRAMES === CONTRACT_IDLE_FRAMES
  && STREAM_ENABLED;

/**
 * ── V8 判据（契约 v3 裁定 ②：余量 + 稳定性双条）───────────────────────
 * ① `p95_stream − p95_idle ≤ 6ms`；
 * ② Long Task(>50ms) 计数 = 0；
 * ③ **连续 3 次运行全部满足**才算通过（单次通过不算；不许挑最好的一次上报）。
 * 绝对阈值 16.7ms 已作废（60Hz 屏空闲 rAF 间隔 p95≈16.9ms，绝对阈值会把"不做事"判红）。
 */
const P95_DELTA_BUDGET_MS = 6;
const LONG_TASK_MS = 50;
const CONSECUTIVE_RUNS = 3;
/** 契约下限（用于采样规模自检）。 */
const MIN_STREAM_FRAMES_FLOOR = 800;
const MIN_IDLE_FRAMES_FLOOR = 300;
/** V5：首个内容帧 → 首个非空正文帧的最大延迟帧数（沿用 wave-59 D-59-7 口径）。 */
const FIRST_TEXT_MAX_FRAME_LAG = 4;

/** 断言消息统一回显"本次实际用的参数"，避免拿调试参数冒充契约参数。 */
function paramsBanner(): string {
  return `[params] deltaInterval=${CHUNK_INTERVAL_MS}ms maxChunk=${CONTRACT_MAX_CHUNK_CHARS}chars `
    + `streamFrames=${STREAM_FRAMES} idleFrames=${IDLE_FRAMES} `
    + `${USING_CONTRACT_PARAMS ? "（契约值）" : "（⚠️ 非契约值：调试参数）"}`;
}

/**
 * 占位流的 markdown 原文。delta 序列由它**在 Node 侧切成 ≤50 字符的片**生成 ——
 * 这样「单次 chunk ≤ 50 字符」就不靠约定，而是构建期可校验的事实（见 buildDeltas）。
 */
const RAW_MARKDOWN_BLOCKS = [
  "## 流式标题\n\n",
  "- 列表项一：用于验证行高与块间距\n",
  "- 列表项二：用于验证列表块增长\n\n",
  "第一段正文：模拟真实回答中的连续中文与 English 混排内容，长度接近真实一行以上。\n\n",
  "第二段正文：逐帧采样内容列与正文容器的滚动高度，任何 > 1px 的回撤都视为闪跳。\n\n",
  "```ts\nconst answer = 42;\nfunction wave() { return answer; }\n```\n\n",
  "尾段持续追加，用于验证尾巴始终只是最后一段的行内节点。"
] as const;

/** 按契约值切 delta（单次 ≤50 字符），构建期校验，超限直接抛错而不是静默变长。 */
function buildDeltas(): string[] {
  const deltas: string[] = [];
  for (const block of RAW_MARKDOWN_BLOCKS) {
    for (let index = 0; index < block.length; index += CONTRACT_MAX_CHUNK_CHARS) {
      deltas.push(block.slice(index, index + CONTRACT_MAX_CHUNK_CHARS));
    }
  }
  const oversized = deltas.filter((delta) => delta.length > CONTRACT_MAX_CHUNK_CHARS);
  if (oversized.length > 0) {
    throw new Error(`占位流 delta 超过契约上限 ${CONTRACT_MAX_CHUNK_CHARS} 字符：${JSON.stringify(oversized)}`);
  }
  return deltas;
}

const STREAM_DELTAS = buildDeltas();

/**
 * 证据目录：**固定路径**（不是每次随机），便于领取/复核时只看一个目录。
 * 之前用时间戳目录时，Playwright 在测试失败后重启 worker 会重新求值模块作用域，
 * 导致同一次运行把产物散落到多个 `dsh-pds-wave3-<ts>/` 目录里（实测 3 个），
 * 对"留档基准"这种需要单点引用的证据是有害的。
 */
const RUN_DIR = path.join(os.tmpdir(), "dsh-pds-wave3-artifacts");
/** 仓库根（由本文件位置反推：<root>/tests/acceptance/pds-streaming/e2e/）。 */
const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");

interface StreamSample {
  t: number;
  contentLength: number;
  columnScrollHeight: number | null;
  bodyScrollHeight: number | null;
  bodyTextLength: number;
  tailCount: number;
  /** 尾巴文本长度（V6 的第二个判据：不得跨多行）。 */
  tailTextLength: number;
  /** 尾巴几何高度（V6 主判据）。 */
  tailRectHeight: number | null;
  /** 尾巴所在容器的计算行高（px）。 */
  lineHeightPx: number | null;
  assistantStatus: string | null;
}

let samples: StreamSample[] = [];
let consoleErrors: string[] = [];
let firstContentFrameIndex: number | null = null;
let firstTextFrameIndex: number | null = null;
let shotPath = "";

async function stubReleaseEndpoints(page: Page) {
  const releasePayload = JSON.stringify({
    tag_name: "v0.1.168",
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

async function openHome(page: Page, seedLocalStorage: Record<string, string> = {}) {
  await stubReleaseEndpoints(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.evaluate(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });
  // 尾巴开关=缺省（on）；释放速率放大，避免「内容还没释放出来」污染帧预算采样。
  await page.evaluate((seed) => {
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    for (const [key, value] of Object.entries(seed)) {
      window.localStorage.setItem(key, value);
    }
  }, seedLocalStorage);
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });
}

async function startSyntheticStream(
  page: Page,
  totalFrames: number,
  chunkIntervalMs: number,
  streamEnabled: boolean,
  deltas: string[] = STREAM_DELTAS
) {
  await page.evaluate(
    ({ frames: maxFrames, interval, enabled, deltas: streamDeltas }) => {
      const w = window as unknown as Record<string, any>;
      const appEl = document.querySelector("#app") as unknown as Record<string, any>;
      const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
      const store = pinia._s.get("runtime");
      const turnId = `pds-turn-${Date.now()}`;
      const assistantId = `pds-assistant-${turnId}`;

      store.$patch({
        sessionId: "pds-e2e-session",
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
          { id: `pds-user-${turnId}`, turnId, role: "user", content: "wave-pds-3 逐帧视觉验收" },
          { id: assistantId, turnId, role: "assistant", content: "", status: "pending" }
        ]
      });

      const pickColumn = () =>
        document.querySelector('[data-testid="workspace-content-column"]') as HTMLElement | null;

      w.__pds = {
        frames: [] as unknown[],
        stopped: false,
        growthFinished: false,
        firstContentFrameIndex: null as number | null,
        firstTextFrameIndex: null as number | null
      };

      const record = () => {
        const harness = w.__pds;
        if (harness.stopped) return;
        const message = store.messages.find((item: Record<string, unknown>) => item.id === assistantId);
        const contentLength = typeof message?.content === "string" ? message.content.length : 0;
        const column = pickColumn();
        const body = column
          ? ((column.querySelector(".markdown-body") as HTMLElement | null)
            ?? (column.querySelector('[data-testid="assistant-streaming-flow"]') as HTMLElement | null)
            ?? (column.querySelector(".assistant-plain-text") as HTMLElement | null))
          : null;
        const tails = column ? Array.from(column.querySelectorAll("[data-stream-tail]")) : [];
        const tail = tails[0] as HTMLElement | undefined;
        const tailRect = tail ? tail.getBoundingClientRect() : null;
        const tailStyle = tail ? window.getComputedStyle(tail) : null;
        const lineHeightRaw = tailStyle ? tailStyle.lineHeight : null;
        const lineHeightPx = lineHeightRaw && lineHeightRaw.endsWith("px")
          ? Number.parseFloat(lineHeightRaw)
          : null;
        const bodyText = body ? (body.textContent ?? "").trim() : "";
        const frameIndex = harness.frames.length;

        if (contentLength > 0 && harness.firstContentFrameIndex === null) {
          harness.firstContentFrameIndex = frameIndex;
        }
        if (bodyText.length > 0 && harness.firstTextFrameIndex === null) {
          harness.firstTextFrameIndex = frameIndex;
        }

        harness.frames.push({
          t: performance.now(),
          contentLength,
          columnScrollHeight: column ? column.scrollHeight : null,
          bodyScrollHeight: body ? body.scrollHeight : null,
          bodyTextLength: bodyText.length,
          tailCount: tails.length,
          tailTextLength: tail ? (tail.textContent ?? "").length : 0,
          tailRectHeight: tailRect ? tailRect.height : null,
          lineHeightPx,
          assistantStatus: typeof message?.status === "string" ? message.status : null
        });

        if (harness.frames.length >= maxFrames) {
          harness.stopped = true;
          return;
        }
        harness.raf = requestAnimationFrame(record);
      };

      w.__pds.raf = requestAnimationFrame(record);

      let emitted = 0;
      w.__pds.growth = setInterval(() => {
        if (w.__pds.stopped) {
          clearInterval(w.__pds.growth);
          w.__pds.growthFinished = true;
          return;
        }
        if (!enabled) {
          // 空闲基线模式：只采样帧，不追加内容（用于区分「环境上限」与「实现劣化」）
          return;
        }
        const chunk = streamDeltas[emitted % streamDeltas.length];
        const target = store.messages.find((item: Record<string, unknown>) => item.id === assistantId);
        if (target) target.content += chunk;
        emitted += 1;
      }, interval);
    },
    { frames: totalFrames, interval: chunkIntervalMs, enabled: streamEnabled, deltas }
  );
}

async function readHarness(page: Page) {
  return page.evaluate(() => {
    const harness = (window as unknown as Record<string, any>).__pds;
    harness.stopped = true;
    cancelAnimationFrame(harness.raf);
    if (harness.growth) clearInterval(harness.growth);
    return {
      frames: harness.frames as StreamSample[],
      firstContentFrameIndex: harness.firstContentFrameIndex as number | null,
      firstTextFrameIndex: harness.firstTextFrameIndex as number | null
    };
  });
}

function percentile(values: number[], p: number): number {
  if (values.length === 0) return Number.NaN;
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[index]!;
}

function frameDeltas(frames: StreamSample[]): number[] {
  const deltas: number[] = [];
  for (let index = 1; index < frames.length; index += 1) {
    deltas.push(frames[index]!.t - frames[index - 1]!.t);
  }
  return deltas;
}

function retractions(values: Array<number | null>) {
  const drops: Array<{ index: number; from: number; to: number; delta: number }> = [];
  for (let index = 1; index < values.length; index += 1) {
    const previous = values[index - 1];
    const next = values[index];
    if (previous == null || next == null) continue;
    const delta = next - previous;
    if (delta < -SCROLL_RETRACT_TOLERANCE_PX) {
      drops.push({ index, from: previous, to: next, delta: Number(delta.toFixed(2)) });
    }
  }
  return drops;
}

// ─────────────────── V7-b / V8 专用页内探针与观察器 ───────────────────

interface PdsComputedNode {
  selector: string;
  className: string;
  animationName: string;
  animationDuration: string;
}

const PDS_ANIMATED_SELECTORS = [
  ".transition-item-enter",
  ".assistant-streaming-fade",
  "[data-pds-tool-row]"
] as const;

/** 铺一个同时含「工具面板 + reasoning 尾巴」的最小状态，保证 PDS 动画节点真实存在。 */
async function patchMotionNodes(page: Page) {
  await page.evaluate(() => {
    const appEl = document.querySelector("#app") as unknown as Record<string, any>;
    const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
    const store = pinia._s.get("runtime");
    const turnId = "pds-v7b-turn";
    store.$patch({
      sessionOperation: null,
      phase: "calling_model",
      error: null,
      isSubmitting: true,
      activeTurnId: turnId,
      activeTurnStartedAtMs: Date.now(),
      messages: [
        {
          id: "pds-v7b-tool",
          turnId,
          role: "tool",
          content: "",
          status: "done",
          tokenCount: 3,
          toolName: "Search",
          canonicalToolName: "Search",
          displayNameZh: "搜索",
          detail: "搜索：pony",
          durationSeconds: 0.2
        },
        {
          id: "pds-v7b-assistant",
          turnId,
          role: "assistant",
          content: "答案正文",
          status: "pending",
          reasoningContent: "思考片段一二",
          modelName: "OpenAI/GPT-5"
        }
      ]
    });
  });
  await page.waitForFunction(() => Boolean(document.querySelector(".conversation-tool-panel")), undefined, {
    timeout: 15_000
  });
  // reasoning 尾巴只在「show-reasoning-content 开启 + 增量 ≥3 字」时出现；
  // 该开关由 localStorage 决定（openHome 已按参数 seed）。这里再追加一次增量，等「显式条件」出现；
  // 若 5s 内未出现则**不作为失败**（reasoning 面板不是 V7-b 的必要节点，工具面板已足够），
  // 但会把实际采集到的节点打出来，避免"少采了节点却看起来通过"。
  await page.evaluate(() => {
    const appEl = document.querySelector("#app") as unknown as Record<string, any>;
    const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
    const store = pinia._s.get("runtime");
    const target = store.messages.find((item: Record<string, unknown>) => item.id === "pds-v7b-assistant");
    if (target) target.reasoningContent = "思考片段一二三四五六";
  });
  await page
    .waitForFunction(() => Boolean(document.querySelector(".assistant-streaming-fade")), undefined, { timeout: 5_000 })
    .catch(() => undefined);
}

/** 插入尾巴样式探针（读样式表计算值；降级态下应用本身不允许渲染尾巴，故用探针）。 */
async function installTailProbe(page: Page) {
  await page.evaluate(() => {
    const column = document.querySelector('[data-testid="workspace-content-column"]');
    if (!column) throw new Error("V7-b：找不到内容列，无法插入尾巴探针");
    const probe = document.createElement("span");
    probe.setAttribute("data-stream-tail", "");
    probe.setAttribute("data-tail-len", "1");
    probe.setAttribute("data-pds-tail-probe", "true");
    probe.textContent = "x";
    column.appendChild(probe);
  });
}

async function readTailProbeMask(page: Page) {
  return page.evaluate(() => {
    const probe = document.querySelector('[data-pds-tail-probe="true"]') as HTMLElement | null;
    if (!probe) return null;
    const style = window.getComputedStyle(probe);
    return {
      maskImage: style.maskImage ?? "",
      webkitMaskImage: (style as unknown as Record<string, string>).webkitMaskImage ?? "",
      display: style.display
    };
  });
}

async function readPdsComputedStyles(page: Page) {
  return page.evaluate((selectors) => {
    const nodes: Array<{ selector: string; className: string; animationName: string; animationDuration: string }> = [];
    for (const selector of selectors) {
      for (const element of Array.from(document.querySelectorAll(selector))) {
        const style = window.getComputedStyle(element as Element);
        nodes.push({
          selector,
          className: (element as HTMLElement).className,
          animationName: style.animationName ?? "",
          animationDuration: style.animationDuration ?? ""
        });
      }
    }
    return { nodes };
  }, PDS_ANIMATED_SELECTORS as unknown as string[]);
}

/** 安装 Long Task 观察器（entryType: longtask）。安装失败会被记录，V8 自检要求它必须成功。 */
async function installLongTaskObserver(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as Record<string, any>;
    w.__pdsLongTasks = [];
    w.__pdsLongTaskError = undefined;
    try {
      const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          w.__pdsLongTasks.push({ name: entry.name, startTime: entry.startTime, duration: entry.duration });
        }
      });
      observer.observe({ entryTypes: ["longtask"] });
      w.__pdsLongTaskObserver = observer;
    } catch (error) {
      w.__pdsLongTaskError = String(error);
    }
  });
}

async function readLongTasks(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as Record<string, any>;
    return (w.__pdsLongTasks ?? []) as Array<{ name: string; startTime: number; duration: number }>;
  });
}

async function readLongTaskObserverError(page: Page): Promise<string | undefined> {
  return page.evaluate(() => {
    const w = window as unknown as Record<string, any>;
    return w.__pdsLongTaskError as string | undefined;
  });
}

test.describe("wave-pds-3 V4/V5/V6/V7-b/V8/V9：真 Chromium 逐帧与降级验收", () => {
  let browser: Browser;

  test.beforeAll(async ({ browser: browserFixture }) => {
    browser = browserFixture;
    fs.mkdirSync(RUN_DIR, { recursive: true });
    const page = await browser.newPage();
    consoleErrors = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => consoleErrors.push(`pageerror: ${error.message}`));

    await openHome(page);
    await startSyntheticStream(page, STREAM_FRAMES, CHUNK_INTERVAL_MS, STREAM_ENABLED);
    await page.waitForFunction(
      () => (window as unknown as Record<string, any>).__pds?.stopped === true,
      undefined,
      { timeout: 120_000 }
    );
    const harness = await readHarness(page);
    samples = harness.frames;
    firstContentFrameIndex = harness.firstContentFrameIndex;
    firstTextFrameIndex = harness.firstTextFrameIndex;

    shotPath = path.join(RUN_DIR, "stream-final.png");
    await page.screenshot({ path: shotPath, fullPage: false });

    fs.writeFileSync(
      path.join(RUN_DIR, "samples.json"),
      JSON.stringify(
        {
          runDir: RUN_DIR,
          chunkIntervalMs: CHUNK_INTERVAL_MS,
          streamEnabled: STREAM_ENABLED,
          consoleErrors,
          firstContentFrameIndex,
          firstTextFrameIndex,
          frameCount: samples.length,
          p95: percentile(frameDeltas(samples), 95),
          samples
        },
        null,
        2
      )
    );
    await page.close();
  });

  test("V9: Console Error = 0（含 pageerror）", () => {
    // Assert 自检：采样窗口内必须真的有流式内容，否则「没有报错」是因为什么都没跑
    expect(
      samples.length,
      `V9 自检：必须采到足够帧（契约下限 ${MIN_STREAM_FRAMES_FLOOR}）；${paramsBanner()}`
    ).toBeGreaterThanOrEqual(MIN_STREAM_FRAMES_FLOOR - 1);
    expect(
      samples.filter((sample) => sample.contentLength > 0).length,
      "V9 自检：采样窗口内必须有内容增长"
    ).toBeGreaterThan(100);

    // Assert
    expect(
      consoleErrors,
      `V9：浏览器 Console Error 必须为 0；实际：${JSON.stringify(consoleErrors)}`
    ).toHaveLength(0);
  });

  test("V4: 逐帧 scrollHeight 不回撤 > 1px（内容列 + 正文容器）", () => {
    // Assert 自检
    expect(
      samples.length,
      `V4 自检：必须采到足够帧（契约下限 ${MIN_STREAM_FRAMES_FLOOR}）；${paramsBanner()}`
    ).toBeGreaterThanOrEqual(MIN_STREAM_FRAMES_FLOOR - 1);

    // Act
    const columnDrops = retractions(samples.map((sample) => sample.columnScrollHeight));
    const bodyDrops = retractions(samples.map((sample) => sample.bodyScrollHeight));

    // Assert
    expect(
      columnDrops,
      `V4：内容列 scrollHeight 逐帧不得回撤 > ${SCROLL_RETRACT_TOLERANCE_PX}px；回撤点：${JSON.stringify(columnDrops)}`
        + `（原始帧数据：${path.join(RUN_DIR, "samples.json")}）`
    ).toHaveLength(0);
    expect(
      bodyDrops,
      `V4：正文容器 scrollHeight 逐帧不得回撤 > ${SCROLL_RETRACT_TOLERANCE_PX}px；回撤点：${JSON.stringify(bodyDrops)}`
    ).toHaveLength(0);
  });

  test("V5: 正文首帧非空后无空白帧", () => {
    // Assert 自检
    expect(firstTextFrameIndex, "V5 自检：正文必须在采样窗口内出现过非空文本").not.toBeNull();
    expect(firstContentFrameIndex, "V5 自检：内容长度必须增长过").not.toBeNull();

    // Assert ①：首个内容帧 → 首个非空帧的延迟（stale-while-render 允许 ≤1 个渲染周期）
    expect(
      (firstTextFrameIndex as number) - (firstContentFrameIndex as number),
      `V5：首个内容帧到首个非空正文帧的延迟不得超过 ${FIRST_TEXT_MAX_FRAME_LAG} 帧`
    ).toBeLessThanOrEqual(FIRST_TEXT_MAX_FRAME_LAG);

    // Assert ②：首个非空帧之后不得再出现「content > 0 但正文为空」的帧
    const blankFrames = samples
      .map((sample, index) => ({ index, ...sample }))
      .filter(
        (sample) => sample.index > (firstTextFrameIndex as number)
          && sample.contentLength > 0
          && sample.bodyTextLength === 0
      );
    expect(
      blankFrames.map((sample) => ({ index: sample.index, contentLength: sample.contentLength })),
      "V5：正文已出现后不得再有空白帧（白屏闪）"
    ).toHaveLength(0);
  });

  test("V6: 流式全程 tail 高度 ≤ 行高 × 1.2，且数量恒 ≤1（存在未闭合尾巴文本时为 1）", () => {
    // Arrange：只统计「流式仍在进行且尾巴已渲染」的帧
    const tailFrames = samples
      .map((sample, index) => ({ index, ...sample }))
      .filter((sample) => sample.tailCount > 0);

    // Assert 自检：尾巴必须真的出现过（否则「高度不超一行」是无对象的假绿）
    expect(
      tailFrames.length,
      `V6 自检：采样窗口内必须有足够帧渲染了 [data-stream-tail]（实际 ${tailFrames.length} 帧；`
        + `这是本波实现前**必然为红**的判据，实现缺失时不允许用「0 帧跳过」冒充通过）`
    ).toBeGreaterThan(100);

    // Assert ①（契约 v2 §1）：任何帧数量都不得超过 1
    const countViolations = samples
      .map((sample, index) => ({ index, ...sample }))
      .filter((sample) => sample.tailCount > 1);
    expect(
      countViolations.map((sample) => ({ index: sample.index, tailCount: sample.tailCount })),
      "V6：流式全程 [data-stream-tail] 数量必须恒 ≤1（契约 v2 §1）"
    ).toHaveLength(0);

    // Assert ②：几何高度 ≤ 行高 × 1.2（真实布局，jsdom 无此能力）
    const measured = tailFrames.filter(
      (sample) => sample.tailRectHeight != null && sample.lineHeightPx != null && sample.lineHeightPx > 0
    );
    expect(
      measured.length,
      "V6 自检：必须能同时解析出 tail 高度与行高（否则几何判据无从计算）"
    ).toBeGreaterThan(100);

    const overHeight = measured.filter(
      (sample) => (sample.tailRectHeight as number) > (sample.lineHeightPx as number) * 1.2
    );
    expect(
      overHeight.map((sample) => ({
        index: sample.index,
        tailRectHeight: sample.tailRectHeight,
        lineHeightPx: sample.lineHeightPx,
        tailTextLength: sample.tailTextLength
      })),
      "V6：尾巴文本高度不得超过行高 × 1.2（尾巴必须始终是尾段、不得折成多行）"
    ).toHaveLength(0);
  });

  /**
   * V7-b（契约 v2 §6）：Chromium 下 computed `animation-name === "none"` 且尾巴 `mask-image: none`。
   *
   * 为什么必须在 Chromium：jsdom 的 CSSOM 不实现 `animation-name` / `mask-image` 取值，
   * 也不按媒体查询求解级联（v1 交付已实测），所以该判据只能在这里做。
   *
   * 判据分三段（缺任一段都是假绿）：
   *   ① 对照组（正常态）：PDS 节点上确有非 none 的 animation-name，且尾巴探针的 mask-image 是渐变；
   *   ② 系统轨道：`emulateMedia({ reducedMotion: "reduce" })` 后所有 PDS 节点 animation-name === none，
   *      且尾巴 `mask-image === none`；
   *   ③ 应用轨道：去掉媒体模拟、给祖先节点打 `data-pds-reduce-motion="true"`，重复 ② 的断言。
   *
   * 尾巴探针说明：降级态下应用**不允许渲染**真实尾巴（V7-1/V7-2 已锁），因此这里插入一个
   * 独立的 `<span data-stream-tail data-tail-len="1">` 探针元素来读取**样式表**的真实计算值。
   * 这是"样式表契约存在性"的物理证据，不是把应用节点当成通过依据。
   */
  test("V7-b [Chromium]: 降级（媒体 + data-pds-reduce-motion）下 animation-name === none 且 mask-image: none", async ({ browser: browserFixture }) => {
    test.setTimeout(120_000);
    const page = await browserFixture.newPage();
    await openHome(page, { "pony-agent.ui.show-reasoning-content": "true" });
    await patchMotionNodes(page);
    await installTailProbe(page);

    // ── ① 对照组（正常态）─────────────────────────────────────────────────
    const normalStyles = await readPdsComputedStyles(page);
    const normalMask = await readTailProbeMask(page);

    // ── ② 系统轨道：prefers-reduced-motion: reduce ─────────────────────
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
    const mediaReducedStyles = await readPdsComputedStyles(page);
    const mediaReducedMask = await readTailProbeMask(page);

    // ── ③ 应用轨道：祖先 data-pds-reduce-motion="true" ──────────────────
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await page.evaluate(() => {
      document.documentElement.setAttribute("data-pds-reduce-motion", "true");
    });
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
    const attributeReducedStyles = await readPdsComputedStyles(page);
    const attributeReducedMask = await readTailProbeMask(page);

    /**
     * 三条轨道全部收集齐再一次性断言（fail-fast 只报第一条，会让执行者来回跑三轮）。
     * 每条违规都带 track + 具体节点，便于定位是实现缺失还是选择器写错。
     */
    const violations: Array<Record<string, unknown>> = [];

    if (normalStyles.nodes.length === 0) {
      violations.push({ track: "control", problem: "未采集到任何 PDS 动画节点（自检失败）" });
    }
    if (!normalStyles.nodes.some((node) => node.animationName !== "none")) {
      violations.push({
        track: "control",
        problem: "正常态下没有任何 PDS 节点带 animation-name（自检失败：此时降级为 none 无判别力）",
        nodes: normalStyles.nodes
      });
    }
    if (!/gradient/.test(`${normalMask?.maskImage ?? ""} ${normalMask?.webkitMaskImage ?? ""}`)) {
      violations.push({
        track: "control",
        problem: "正常态下尾巴必须带渐隐遮罩 mask-image: linear-gradient(...)",
        detail: normalMask
      });
    }
    for (const node of mediaReducedStyles.nodes.filter((entry) => entry.animationName !== "none")) {
      violations.push({
        track: "media",
        problem: "系统 reduced-motion 下 animation-name 必须为 none",
        node
      });
    }
    if (!(mediaReducedMask?.maskImage === "none" || mediaReducedMask?.webkitMaskImage === "none")) {
      violations.push({
        track: "media",
        problem: "系统 reduced-motion 下尾巴 mask-image 必须为 none",
        detail: mediaReducedMask
      });
    }
    for (const node of attributeReducedStyles.nodes.filter((entry) => entry.animationName !== "none")) {
      violations.push({
        track: "attribute",
        problem: "祖先 data-pds-reduce-motion=\"true\" 下 animation-name 必须为 none",
        node
      });
    }
    if (!(attributeReducedMask?.maskImage === "none" || attributeReducedMask?.webkitMaskImage === "none")) {
      violations.push({
        track: "attribute",
        problem: "祖先 data-pds-reduce-motion=\"true\" 下尾巴 mask-image 必须为 none",
        detail: attributeReducedMask
      });
    }

    // Assert（契约 v2 §6 V7-b：三轨全量）
    expect(
      violations,
      `V7-b：降级 computed-style 判据未满足（契约 v2 §2/§3.5）；违规：${JSON.stringify(violations, null, 1)}；`
        + `采集到的节点：${JSON.stringify(normalStyles.nodes)}`
    ).toHaveLength(0);

    await page.evaluate(() => {
      document.documentElement.removeAttribute("data-pds-reduce-motion");
    });
    await page.close();
  });

  /**
   * V8（契约 **v3** §5）：余量 + 稳定性**双条**判据。
   *
   * 单次运行的判据（每次运行都在**新页面**上跑：先空闲基线，再流式采样）：
   *   ① `p95_stream − p95_idle ≤ 6ms`；
   *   ② Long Task(>50ms) 计数 = 0。
   * 稳定性判据：**连续 3 次运行全部满足**才算通过 —— 单次通过不算，也不许挑最好的一次上报。
   *
   * 为什么这么定：60Hz 上 rAF 调度噪声会让单跑的 p95 上跳/下跳各 ~1ms，
   * 单次判定既会假红也会假绿；"连续 3 次"才是可判定的稳定性表述（这是收紧，不是放宽）。
   *
   * 三次运行的实测表会**落盘**到 `tests/acceptance/pds-streaming/receipts/e2e/v8-relative-runs.json`
   * （含每次的时间戳），交付说明必须引用该表。
   */
  test(`V8: 连续 ${CONSECUTIVE_RUNS} 次 均满足 p95 增量 ≤ ${P95_DELTA_BUDGET_MS}ms 且 Long Task(>50ms)=0`, async ({ browser: browserFixture }) => {
    test.setTimeout(600_000);

    const runs: Array<{
      index: number;
      startedAt: string;
      chunkIntervalMs: number;
      maxChunkChars: number;
      idleFrames: number;
      streamFrames: number;
      idleP50: number;
      idleP95: number;
      streamP50: number;
      streamP95: number;
      streamMax: number;
      p95Delta: number;
      longTaskCount: number;
      growthChars: number;
      verdict: "pass" | "fail";
    }> = [];

    // Act：连续 N 次，每次都是「新页面 → 空闲基线 → 流式采样」
    for (let index = 1; index <= CONSECUTIVE_RUNS; index += 1) {
      const startedAt = new Date().toISOString();
      const page = await browserFixture.newPage();
      await openHome(page);

      // 1) 空闲基线（同一页面、零 delta）
      await startSyntheticStream(page, IDLE_FRAMES, CHUNK_INTERVAL_MS, false);
      await page.waitForFunction(
        () => (window as unknown as Record<string, any>).__pds?.stopped === true,
        undefined,
        { timeout: 120_000 }
      );
      const idleHarness = await readHarness(page);
      const idleDeltas = frameDeltas(idleHarness.frames);
      const idleP50 = percentile(idleDeltas, 50);
      const idleP95 = percentile(idleDeltas, 95);

      // 2) Long Task 观察器（流式开始前安装，避免把页面装载开销算进去）
      await installLongTaskObserver(page);

      // 3) 流式采样（同一页面，≥800 帧）
      await startSyntheticStream(page, STREAM_FRAMES, CHUNK_INTERVAL_MS, true);
      await page.waitForFunction(
        () => (window as unknown as Record<string, any>).__pds?.stopped === true,
        undefined,
        { timeout: 180_000 }
      );
      const streamHarness = await readHarness(page);
      const streamDeltas = frameDeltas(streamHarness.frames);
      const streamP50 = percentile(streamDeltas, 50);
      const streamP95 = percentile(streamDeltas, 95);
      const streamMax = streamDeltas.length > 0 ? Math.max(...streamDeltas) : Number.NaN;
      const longTasks = await readLongTasks(page);
      const observerError = await readLongTaskObserverError(page);
      const growthChars = streamHarness.frames[streamHarness.frames.length - 1]!.contentLength;
      await page.close();

      // 观察器不可用 ⇒ 无法证明"计数为 0"，直接计入失败（不许静默放行）
      expect(
        observerError,
        `V8 第 ${index} 次自检：Long Task 观察器必须成功安装（PerformanceObserver entrytype longtask）；`
          + `实际错误：${observerError}`
      ).toBeUndefined();
      expect(
        idleDeltas.length,
        `V8 第 ${index} 次自检：空闲基线必须 ≥ ${MIN_IDLE_FRAMES_FLOOR} 帧（实际 ${idleDeltas.length}）；${paramsBanner()}`
      ).toBeGreaterThanOrEqual(MIN_IDLE_FRAMES_FLOOR - 1);
      expect(
        streamDeltas.length,
        `V8 第 ${index} 次自检：流式采样必须 ≥ ${MIN_STREAM_FRAMES_FLOOR} 帧（实际 ${streamDeltas.length}）；${paramsBanner()}`
      ).toBeGreaterThanOrEqual(MIN_STREAM_FRAMES_FLOOR - 1);

      const p95Delta = streamP95 - idleP95;
      const longTaskCount = longTasks.filter((task) => task.duration > LONG_TASK_MS).length;
      runs.push({
        index,
        startedAt,
        chunkIntervalMs: CHUNK_INTERVAL_MS,
        maxChunkChars: CONTRACT_MAX_CHUNK_CHARS,
        idleFrames: idleDeltas.length,
        streamFrames: streamDeltas.length,
        idleP50,
        idleP95,
        streamP50,
        streamP95,
        streamMax,
        p95Delta,
        longTaskCount,
        growthChars,
        verdict: p95Delta <= P95_DELTA_BUDGET_MS && longTaskCount === 0 ? "pass" : "fail"
      });
    }

    // 落盘：三次运行全量（含时间戳），交付说明引用此表
    const tablePath = path.join(
      REPO_ROOT,
      "tests/acceptance/pds-streaming/receipts/e2e/v8-relative-runs.json"
    );
    fs.mkdirSync(path.dirname(tablePath), { recursive: true });
    fs.writeFileSync(
      tablePath,
      JSON.stringify(
        {
          criterion: `p95_stream - p95_idle <= ${P95_DELTA_BUDGET_MS}ms 且 Long Task(>${LONG_TASK_MS}ms)=0，连续 ${CONSECUTIVE_RUNS} 次全部满足`,
          contractParams: {
            chunkIntervalMs: CONTRACT_CHUNK_INTERVAL_MS,
            maxChunkChars: CONTRACT_MAX_CHUNK_CHARS,
            streamFrames: CONTRACT_STREAM_FRAMES,
            idleFrames: CONTRACT_IDLE_FRAMES
          },
          actualParams: {
            chunkIntervalMs: CHUNK_INTERVAL_MS,
            streamFrames: STREAM_FRAMES,
            idleFrames: IDLE_FRAMES,
            usingContractParams: USING_CONTRACT_PARAMS
          },
          generatedAt: new Date().toISOString(),
          runs
        },
        null,
        2
      )
    );

    // Assert 自检：每次运行都必须真的在流式（内容增长），否则等于测了三次空闲
    const noGrowth = runs.filter((run) => !(run.growthChars > 0)).map((run) => run.index);
    expect(noGrowth, `V8 自检：第 ${JSON.stringify(noGrowth)} 次运行没有内容增长；${paramsBanner()}`).toHaveLength(0);

    // Assert（契约 v3 §5 双条 + 连续性）
    const failing = runs.filter((run) => run.verdict === "fail");
    expect(
      failing,
      `V8：连续 ${CONSECUTIVE_RUNS} 次必须全部满足「p95 增量 ≤ ${P95_DELTA_BUDGET_MS}ms 且 Long Task=0」；`
        + `${paramsBanner()}；实测：` + JSON.stringify(
        runs.map((run) => ({
          次: run.index,
          idleP95: Number(run.idleP95.toFixed(2)),
          streamP95: Number(run.streamP95.toFixed(2)),
          delta: Number(run.p95Delta.toFixed(2)),
          longTask: run.longTaskCount,
          判定: run.verdict
        })),
        null,
        1
      ) + `（落盘：${tablePath}）`
    ).toHaveLength(0);
  });

  /**
   * V8-b：空闲基线**采集**（契约 v2 后不再是断言项，改为留档数据）。
   *
   * 理由：v3 已把绝对阈值作废，环境基线的意义是"实现后对比的基准值"，而不是通过条件。
   * 这里只断言「采样确实有效」（帧数 ≥300 且页面确实在跑），然后把 p50/p95 打印并落盘。
   */
  test("V8-b [ENV/数据]: 采集空闲基线帧预算（留档基准，不做通过判定）", async ({ browser: browserFixture }) => {
    // Arrange
    const page = await browserFixture.newPage();
    await openHome(page);
    await startSyntheticStream(page, IDLE_FRAMES, CHUNK_INTERVAL_MS, false);
    await page.waitForFunction(
      () => (window as unknown as Record<string, any>).__pds?.stopped === true,
      undefined,
      { timeout: 90_000 }
    );
    const harness = await readHarness(page);
    await page.close();

    const deltas = frameDeltas(harness.frames);
    const p50 = percentile(deltas, 50);
    const p95 = percentile(deltas, 95);
    const payload = {
      note: "契约 v3 §5：绝对阈值作废；本文件是实现后 V8「余量+稳定性」判据的空闲基准",
      frames: deltas.length,
      p50,
      p95,
      max: deltas.length > 0 ? Math.max(...deltas) : null,
      longTaskThresholdMs: LONG_TASK_MS,
      p95DeltaBudgetMs: P95_DELTA_BUDGET_MS,
      consecutiveRuns: CONSECUTIVE_RUNS,
      params: paramsBanner(),
      measuredAt: new Date().toISOString()
    };
    fs.writeFileSync(
      path.join(RUN_DIR, "idle-baseline.json"),
      JSON.stringify(payload, null, 2)
    );
    // 打印到 stdout，便于 CI 日志留痕
    console.log(
      `[V8-b][ENV] 空闲基线帧预算：p50=${p50.toFixed(2)}ms p95=${p95.toFixed(2)}ms 帧数=${deltas.length}；${paramsBanner()}`
    );

    // Assert（仅采样有效性，不做环境阈值判定）
    expect(
      deltas.length,
      `V8-b 自检：空闲基线必须采到 ≥${MIN_IDLE_FRAMES_FLOOR} 帧（实际 ${deltas.length}）；${paramsBanner()}`
    ).toBeGreaterThanOrEqual(MIN_IDLE_FRAMES_FLOOR - 1);
    expect(
      Number.isFinite(p95) && p95 > 0,
      `V8-b 自检：必须解析出有效 p95（实际 ${p95}）`
    ).toBe(true);
  });

  test("V9-b（凭据）: 采样截图必须落盘非空", () => {
    expect(fs.existsSync(shotPath), `凭据截图未落盘：${shotPath}`).toBe(true);
    expect(fs.statSync(shotPath).size).toBeGreaterThan(0);
  });
});
