/**
 * Wave-59 F3-5 冻结验收（真浏览器）：流式长文逐帧稳定性
 *
 * 契约源（只读）：`.dev-team/contracts/wave-59-desktop-uiux.md` §3 F3-5 + §7 裁定 D-59-7 / D-59-8。
 *
 * 判定口径（契约 §7 D-59-7 正式落契约后）：
 *   ① `[data-testid="workspace-content-column"]` 与 agent 正文容器的 `scrollHeight` 逐帧不得回撤 > 1px；
 *   ② **D-59-7（Lead 裁定，取代原「content>0 任一帧不得为空」）**：
 *      - 「正文首次非空帧」之后不得再出现空文本帧（无「先清空再重绘」白屏闪）；
 *      - 「首个内容帧 → 首个非空正文帧」延迟 ≤ 4 帧（≈66ms，覆盖 16ms render + 解析）；
 *      - **首个 render 周期内的空白不计为白屏闪** —— D-59-1「stale-while-render」允许文字
 *        滞后 ≤ 1 个 render 周期；原字面口径（content>0 任一帧不得为空）已被 D-59-7 作废，
 *        因为它会强迫实现保留文本回退节点，与 F3-1 冲突。
 *   ③ Console Error = 0（含 pageerror）；
 *   ④ 落盘前后对照截图 + 逐帧原始数据到 `${TMPDIR}/dsh-wave59-<ts>/`；
 *   ⑤ **D-59-8**：必须覆盖 streaming→完成态 —— 合成流终态把 assistant 置 `status:"done"`，
 *      断言完成切换前后 `.markdown-body` 节点身份不变、可见文本不回撤、无空白帧。
 *
 * 采样方式：页内 `requestAnimationFrame` 逐帧记录，采样结束后离线断言，
 * 不用固定 sleep 判定竞态（沿用 `tests/e2e/status-flow-stability.spec.ts` 的做法）。
 */
import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

/** 逐帧滚动高度单帧回撤容差（契约 F3-5 ①）。 */
const SCROLL_RETRACT_TOLERANCE_PX = 1;
/** 「正文首次非空」允许的最大延迟帧数（≈1 个 render 周期 + 解析，见文件头说明）。 */
const FIRST_TEXT_MAX_FRAME_LAG = 4;

const RUN_DIR = path.join(
  os.tmpdir(),
  `dsh-wave59-${new Date().toISOString().replace(/[:.]/g, "-")}`
);

interface StreamFrame {
  t: number;
  contentLength: number;
  columnScrollHeight: number | null;
  bodyScrollHeight: number | null;
  /** D-59-8：正文承载节点身份标记（切换完成后不得丢失）。 */
  bodyCarrierTagged: boolean;
  /** D-59-8：该帧是否处于「完成态切换之后」。 */
  afterDoneSwitch: boolean;
  /** D-59-8：assistant 消息状态快照（pending / done）。 */
  assistantStatus: string | null;
  bodyTextLength: number;
  bodyNodePresent: boolean;
}

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

async function openHome(page: Page) {
  await stubReleaseEndpoints(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.evaluate(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });
  await page.goto("/", { waitUntil: "networkidle" });
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  // index.html 的启动屏 #app-loading-screen 会遮挡截图，必须在采样前真实离场。
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, {
    timeout: 15_000
  });
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
 * 启动合成 markdown 长流：
 * - 首个 patch 建立 pending assistant（content 为空），随后按 chunk 递增追加正文（标题 / 列表 / 代码块 / 段落）；
 * - 同帧起 rAF 逐帧采样内容列与正文容器的 scrollHeight / 可见文本长度。
 */
async function startSyntheticMarkdownStream(page: Page, chunkCount: number, intervalMs: number) {
  await page.evaluate(
    ({ totalChunks, interval }) => {
      const w = window as unknown as Record<string, any>;
      const appEl = document.querySelector("#app") as unknown as Record<string, any>;
      const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
      const store = pinia._s.get("runtime");
      const turnId = `w59-turn-${Date.now()}`;
      const assistantId = `w59-assistant-${turnId}`;

      store.$patch({
        sessionId: "w59-e2e-session",
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
          { id: `w59-user-${turnId}`, turnId, role: "user", content: "wave-59 合成 markdown 长流稳定性诊断" },
          { id: assistantId, turnId, role: "assistant", content: "", status: "pending" }
        ]
      });

      const chunks = [
        "## 流式标题\n\n",
        "- 列表项一：用于验证行高与块间距\n",
        "- 列表项二：用于验证列表块增长\n",
        "- 列表项三：用于验证长列表滚动态\n\n",
        "```ts\nconst answer = 42;\nfunction wave59() { return answer; }\n```\n\n",
        "第一段正文：这一段用于模拟真实回答中的连续中文与 English 混排内容，长度接近真实一行以上。\n\n",
        "第二段正文：流式过程中逐帧采样内容列与正文容器的滚动高度，任何 > 1px 的回撤都视为闪跳。\n\n",
        "1. 编号列表项一\n",
        "2. 编号列表项二\n\n",
        "### 小节标题\n\n",
        "第三段正文：补充内容，继续拉长滚动高度以放大任何回撤。\n\n",
        "> 引用块：用于验证块级元素的稳定性。\n\n",
        "```js\nconst list = [1, 2, 3, 4, 5];\nconsole.log(list.map((n) => n * 2));\n```\n\n",
        "第四段正文：接近结束，仍持续逐块追加。\n\n",
        "第五段正文：结束前的最后一段，用于观察完成前是否出现高度回撤。\n\n"
      ];

      w.__w59 = {
        frames: [] as StreamFrame[],
        stopped: false,
        firstContentFrameIndex: null as number | null,
        firstTextFrameIndex: null as number | null,
        // D-59-8：完成态切换帧号（null = 尚未切换）
        doneSwitchFrameIndex: null as number | null,
        // D-59-8：切换前最后一次采样到的可见文本长度（完成后不得回撤）
        textLengthAtSwitch: null as number | null,
        // D-59-8：承载节点身份标记属性名（写在 .markdown-body 上，切换后必须仍在同一节点上）
        carrierTagAttr: "data-w59-carrier-tag",
        carrierTagValue: "w59-body-carrier"
      };

      const pickColumn = () =>
        document.querySelector('[data-testid="workspace-content-column"]') as HTMLElement | null;
      const pickBody = () => {
        const column = pickColumn();
        if (!column) return null;
        return (
          (column.querySelector(".markdown-body") as HTMLElement | null) ??
          (column.querySelector('[data-testid="assistant-streaming-flow"]') as HTMLElement | null) ??
          (column.querySelector(".assistant-plain-text") as HTMLElement | null)
        );
      };

      const record = () => {
        const harness = w.__w59;
        if (harness.stopped) return;
        const message = store.messages.find((item: Record<string, unknown>) => item.id === assistantId);
        const contentLength = typeof message?.content === "string" ? message.content.length : 0;
        const column = pickColumn();
        const body = pickBody();
        const bodyText = body ? (body.textContent ?? "").trim() : "";
        const frameIndex = harness.frames.length;
        if (contentLength > 0 && harness.firstContentFrameIndex === null) {
          harness.firstContentFrameIndex = frameIndex;
        }
        if (bodyText.length > 0 && harness.firstTextFrameIndex === null) {
          harness.firstTextFrameIndex = frameIndex;
        }
        const carrierTagged = Boolean(
          body && body.getAttribute(harness.carrierTagAttr) === harness.carrierTagValue
        );
        harness.frames.push({
          t: performance.now(),
          contentLength,
          columnScrollHeight: column ? column.scrollHeight : null,
          bodyScrollHeight: body ? body.scrollHeight : null,
          bodyTextLength: bodyText.length,
          bodyNodePresent: Boolean(body),
          bodyCarrierTagged: carrierTagged,
          afterDoneSwitch: harness.doneSwitchFrameIndex !== null,
          assistantStatus: typeof message?.status === "string" ? message.status : null
        });
        harness.raf = requestAnimationFrame(record);
      };
      w.__w59.raf = requestAnimationFrame(record);

      let emitted = 0;
      w.__w59.growth = setInterval(() => {
        const chunk = chunks[emitted % chunks.length];
        const target = store.messages.find((item: Record<string, unknown>) => item.id === assistantId);
        if (target) target.content += chunk;
        emitted += 1;
        if (emitted >= totalChunks) {
          clearInterval(w.__w59.growth);

          // ── D-59-8：streaming → 完成态切换 ────────────────────────────────
          // ① 先给当前正文承载节点打身份标记（同一元素在切换后必须仍带该标记）；
          // ② 记录切换前一刻的可见文本长度（完成后不得回撤）；
          // ③ 再把 assistant 置 done（生产语义：turn 完结，status→done）。
          const column = pickColumn();
          const body = column
            ? (column.querySelector(".markdown-body") as HTMLElement | null)
            : null;
          const harness = w.__w59;
          const textAtSwitch = body ? (body.textContent ?? "").trim().length : 0;
          if (body) {
            body.setAttribute(harness.carrierTagAttr, harness.carrierTagValue);
          }
          harness.textLengthAtSwitch = textAtSwitch;
          harness.doneSwitchFrameIndex = harness.frames.length;

          store.$patch({ isSubmitting: false, phase: "completed" });
          const doneTarget = store.messages.find((item: Record<string, unknown>) => item.id === assistantId);
          if (doneTarget) doneTarget.status = "done";

          w.__w59.growthFinished = true;
        }
      }, interval);
    },
    { totalChunks: chunkCount, interval: intervalMs }
  );
}

async function stopAndReadFrames(page: Page): Promise<{
  frames: StreamFrame[];
  firstContentFrameIndex: number | null;
  firstTextFrameIndex: number | null;
  doneSwitchFrameIndex: number | null;
  textLengthAtSwitch: number | null;
}> {
  return page.evaluate(() => {
    const w = window as unknown as Record<string, any>;
    const harness = w.__w59;
    harness.stopped = true;
    cancelAnimationFrame(harness.raf);
    if (harness.growth) clearInterval(harness.growth);
    return {
      frames: harness.frames as StreamFrame[],
      firstContentFrameIndex: harness.firstContentFrameIndex as number | null,
      firstTextFrameIndex: harness.firstTextFrameIndex as number | null,
      doneSwitchFrameIndex: harness.doneSwitchFrameIndex as number | null,
      textLengthAtSwitch: harness.textLengthAtSwitch as number | null
    };
  });
}

function retractions(values: Array<number | null>) {
  const drops: Array<{ index: number; from: number; to: number; delta: number }> = [];
  for (let index = 1; index < values.length; index += 1) {
    const prev = values[index - 1];
    const next = values[index];
    if (prev == null || next == null) continue;
    const delta = next - prev;
    if (delta < -SCROLL_RETRACT_TOLERANCE_PX) {
      drops.push({ index, from: prev, to: next, delta: Number(delta.toFixed(2)) });
    }
  }
  return drops;
}

test.describe("wave-59 F3-5：流式长文逐帧稳定性（真浏览器）", () => {
  test("合成 markdown 长流下：滚动高度单调不减、无空白帧、Console Error = 0", async ({ page }) => {
    test.setTimeout(120_000);
    fs.mkdirSync(RUN_DIR, { recursive: true });

    // Arrange
    const consoleErrors = collectConsoleErrors(page);
    await openHome(page);
    await startSyntheticMarkdownStream(page, 14, 120);

    // 首帧非空后立刻截图（before 对照），并等流结束
    await page.waitForFunction(() => {
      const harness = (window as unknown as Record<string, any>).__w59;
      return harness?.firstTextFrameIndex !== null;
    }, undefined, { timeout: 20_000 });
    const beforeShotPath = path.join(RUN_DIR, "before-stream-mid.png");
    await page.screenshot({ path: beforeShotPath });

    await page.waitForFunction(() => (window as unknown as Record<string, any>).__w59?.growthFinished === true, undefined, {
      timeout: 30_000
    });
    // 结束后再采若干帧，覆盖最终稳定态
    await page.evaluate(
      () =>
        new Promise<void>((resolve) => {
          let seen = 0;
          const tick = () => {
            seen += 1;
            if (seen >= 30) resolve();
            else requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        })
    );

    // Act
    const {
      frames,
      firstContentFrameIndex,
      firstTextFrameIndex,
      doneSwitchFrameIndex,
      textLengthAtSwitch
    } = await stopAndReadFrames(page);
    const afterShotPath = path.join(RUN_DIR, "after-stream-final.png");
    await page.screenshot({ path: afterShotPath, fullPage: false });
    fs.writeFileSync(
      path.join(RUN_DIR, "frames.json"),
      JSON.stringify(
        {
          runDir: RUN_DIR,
          consoleErrors,
          firstContentFrameIndex,
          firstTextFrameIndex,
          doneSwitchFrameIndex,
          textLengthAtSwitch,
          frames
        },
        null,
        2
      )
    );

    // Assert（采样自检：帧数足够，且确实抓到了内容增长）
    expect(frames.length, "F3-5 自检：逐帧采样必须覆盖到足够帧数").toBeGreaterThan(20);
    const lastFrame = frames[frames.length - 1]!;
    expect(lastFrame.contentLength, "F3-5 自检：合成流必须真实增长到内容长度 > 0").toBeGreaterThan(0);

    // Assert ①：内容列 scrollHeight 不得回撤 > 1px
    const columnDrops = retractions(frames.map((frame) => frame.columnScrollHeight));
    expect(
      columnDrops,
      `F3-5 ①：workspace-content-column scrollHeight 逐帧不得回撤 > ${SCROLL_RETRACT_TOLERANCE_PX}px；`
        + `回撤点：${JSON.stringify(columnDrops)}（原始帧数据：${path.join(RUN_DIR, "frames.json")}）`
    ).toHaveLength(0);

    // Assert ①'：agent 正文容器 scrollHeight 不得回撤 > 1px
    const bodyDrops = retractions(frames.map((frame) => frame.bodyScrollHeight));
    expect(
      bodyDrops,
      `F3-5 ①：agent 正文容器 scrollHeight 逐帧不得回撤 > ${SCROLL_RETRACT_TOLERANCE_PX}px；`
        + `回撤点：${JSON.stringify(bodyDrops)}（原始帧数据：${path.join(RUN_DIR, "frames.json")}）`
    ).toHaveLength(0);

    // Assert ②：首次非空帧之后，不得再出现「content > 0 但正文为空」的帧
    expect(firstTextFrameIndex, "F3-5 ②：正文文本必须在采样窗口内出现过（否则流式渲染整体失效）").not.toBeNull();
    expect(
      firstContentFrameIndex,
      "F3-5 ②：内容长度必须在采样窗口内增长过"
    ).not.toBeNull();
    expect(
      (firstTextFrameIndex as number) - (firstContentFrameIndex as number),
      `F3-5 ②：首个内容帧到首个非空正文帧的延迟不得超过 ${FIRST_TEXT_MAX_FRAME_LAG} 帧（D-59-1 允许 ≤1 个 render 周期）`
    ).toBeLessThanOrEqual(FIRST_TEXT_MAX_FRAME_LAG);

    const blankFrames = frames
      .map((frame, index) => ({ index, ...frame }))
      .filter((frame) => frame.index > (firstTextFrameIndex as number) && frame.contentLength > 0 && frame.bodyTextLength === 0);
    expect(
      blankFrames,
      `F3-5 ②：正文已出现后不得再有空白帧（白屏闪）；空白帧：${JSON.stringify(blankFrames.slice(0, 8))}`
    ).toHaveLength(0);

    // Assert ⑤（D-59-8）：streaming → 完成态切换的三项物理断言
    expect(
      doneSwitchFrameIndex,
      "F3-5 ⑤（D-59-8）：采样窗口必须覆盖 streaming→完成态切换（doneSwitchFrameIndex 不得为 null）"
    ).not.toBeNull();
    const switchIndex = doneSwitchFrameIndex as number;
    const framesAfterSwitch = frames
      .map((frame, index) => ({ index, ...frame }))
      .filter((frame) => frame.index >= switchIndex);

    // ⑤-1 采样自检：切换后必须真采到若干帧
    expect(
      framesAfterSwitch.length,
      `F3-5 ⑤（D-59-8）：完成态切换后必须采到若干帧（实际 ${framesAfterSwitch.length} 帧；`
        + `切换帧号=${switchIndex}，总帧数=${frames.length}）`
    ).toBeGreaterThanOrEqual(5);

    // ⑤-2 节点身份不变：完成切换后，正文承载节点必须始终是切换前打了身份标记的**同一个** .markdown-body
    const identityLost = framesAfterSwitch.filter((frame) => !frame.bodyNodePresent || !frame.bodyCarrierTagged);
    expect(
      identityLost.map((frame) => ({ index: frame.index, bodyNodePresent: frame.bodyNodePresent, bodyCarrierTagged: frame.bodyCarrierTagged })),
      "F3-5 ⑤（D-59-8）：完成态切换后 .markdown-body 节点身份必须不变（不得替换子树）；"
        + "下列帧丢失了切换前的身份标记或承载节点："
    ).toHaveLength(0);

    // ⑤-3 文本不回撤：切换后可见文本长度不得低于切换前一刻
    const switchBaseline = textLengthAtSwitch as number;
    const textRegressions = framesAfterSwitch.filter((frame) => frame.bodyTextLength < switchBaseline);
    expect(
      textRegressions.map((frame) => ({ index: frame.index, bodyTextLength: frame.bodyTextLength, baseline: switchBaseline })),
      `F3-5 ⑤（D-59-8）：完成切换后可见文本不得回撤（切换前基线 ${switchBaseline} 字符）；下列帧低于基线：`
    ).toHaveLength(0);

    // ⑤-4 完成帧无空白：切换后任一帧都不得出现空正文
    const blankAfterSwitch = framesAfterSwitch.filter((frame) => frame.bodyTextLength === 0);
    expect(
      blankAfterSwitch.map((frame) => ({ index: frame.index, contentLength: frame.contentLength })),
      "F3-5 ⑤（D-59-8）：完成切换后不得出现空白帧"
    ).toHaveLength(0);

    // ⑤-5 状态自检：切换后必须真实进入 done（否则本段断言等于没覆盖完成态）
    const doneFrames = framesAfterSwitch.filter((frame) => frame.assistantStatus === "done");
    expect(
      doneFrames.length,
      `F3-5 ⑤（D-59-8）自检：切换后必须采到 assistant status="done" 的帧；实际 ${doneFrames.length} 帧`
    ).toBeGreaterThan(0);

    // Assert ③：Console Error = 0
    expect(
      consoleErrors,
      `F3-5 ③：浏览器 Console Error 必须为 0；实际：${JSON.stringify(consoleErrors)}`
    ).toHaveLength(0);

    // Assert ④：前后对照截图必须落盘（凭据目录见 frames.json 的 runDir）
    expect(fs.existsSync(beforeShotPath), `F3-5 ④：截图未落盘：${beforeShotPath}`).toBe(true);
    expect(fs.existsSync(afterShotPath), `F3-5 ④：截图未落盘：${afterShotPath}`).toBe(true);
    expect(fs.statSync(beforeShotPath).size).toBeGreaterThan(0);
    expect(fs.statSync(afterShotPath).size).toBeGreaterThan(0);
  });
});
