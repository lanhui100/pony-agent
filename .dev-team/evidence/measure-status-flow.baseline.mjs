// 诊断脚本（临时，非交付物）：测量桌面端消息流在流式输出期间的位移/跳变
// 用法: node .tmp/measure-status-flow.mjs
import pw from "/home/dm/pony-agent/node_modules/@playwright/test/index.js";
const { chromium } = pw;

const URL = "http://127.0.0.1:4175/";
const OUT = process.argv[2] ?? "/home/dm/pony-agent/.tmp/status-flow-metrics.json";

const browser = await chromium.launch({
  executablePath: "/home/dm/.cache/ms-playwright/chromium-1208/chrome-linux64/chrome"
});
const page = await browser.newPage({ viewport: { width: 1280, height: 860 } });
const consoleErrors = [];
page.on("console", (msg) => {
  if (msg.type() === "error") consoleErrors.push(msg.text());
});
page.on("pageerror", (err) => consoleErrors.push(`pageerror: ${err.message}`));

await page.goto(URL, { waitUntil: "domcontentloaded" });
await page.evaluate(() => {
  window.localStorage.clear();
  window.sessionStorage.clear();
});
await page.goto(URL, { waitUntil: "networkidle" });
await page.waitForSelector('[data-testid="workspace-composer-input"]');

// 1) 真实浏览器预览流式提交（320ms 短流）
await page.getByTestId("workspace-composer-input").fill("诊断流式位移");
await page.evaluate(() => {
  window.__samples = [];
  const pick = (sel) => document.querySelector(sel);
  const record = () => {
    const content = pick('[data-testid="workspace-content-column"]');
    const statusSlot = pick('[data-testid="workspace-runtime-status-slot"]');
    const statusBar = pick('[data-testid="agent-runtime-status-bar"]');
    const flow = pick('[data-testid="assistant-streaming-flow"]');
    const userMsg = content?.querySelector("article");
    const scroller = pick(".workspace-timeline-viewport") ?? content?.parentElement;
    window.__samples.push({
      t: performance.now(),
      scrollTop: scroller ? scroller.scrollTop : null,
      scrollHeight: scroller ? scroller.scrollHeight : null,
      clientHeight: scroller ? scroller.clientHeight : null,
      userTop: userMsg ? userMsg.getBoundingClientRect().top : null,
      flowTop: flow ? flow.getBoundingClientRect().top : null,
      flowBottom: flow ? flow.getBoundingClientRect().bottom : null,
      statusTop: statusSlot ? statusSlot.getBoundingClientRect().top : null,
      statusHeight: statusSlot ? statusSlot.getBoundingClientRect().height : null,
      statusBarHeight: statusBar ? statusBar.getBoundingClientRect().height : null,
      statusLabel: statusBar ? statusBar.textContent?.trim() ?? null : null,
      statusDisplay: statusBar ? getComputedStyle(statusBar).display : null
    });
    window.__raf = requestAnimationFrame(record);
  };
  window.__raf = requestAnimationFrame(record);
});
await page.getByTestId("workspace-submit-action").click();
await page.waitForTimeout(2500);
const realRun = await page.evaluate(() => {
  cancelAnimationFrame(window.__raf);
  return window.__samples;
});

// 2) 合成长流：直接 patch store，模拟 3 秒持续流式增长
const synthetic = await page.evaluate(async () => {
  const appEl = document.querySelector("#app");
  const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
  const store = pinia._s.get("runtime");
  const turnId = "diag-turn-1";
  const assistant = {
    id: "diag-assistant-1",
    turnId,
    role: "assistant",
    content: "",
    status: "pending"
  };
  store.$patch({
    sessionId: "diag-session",
    sessionOperation: null,
    phase: "calling_model",
    error: null,
    isSubmitting: true,
    activeTurnId: turnId,
    activeTurnStartedAtMs: Date.now(),
    toolActivities: [],
    messages: [
      { id: "diag-user-1", turnId, role: "user", content: "诊断长流" },
      assistant
    ]
  });
  window.__samples = [];
  const content = () => document.querySelector('[data-testid="workspace-content-column"]');
  const pick = (sel) => document.querySelector(sel);
  const record = () => {
    const col = content();
    const statusSlot = pick('[data-testid="workspace-runtime-status-slot"]');
    const statusBar = pick('[data-testid="agent-runtime-status-bar"]');
    const flow = pick('[data-testid="assistant-streaming-flow"]');
    const userMsg = col?.querySelector("article");
    const scroller = pick(".workspace-timeline-viewport") ?? col?.parentElement;
    window.__samples.push({
      t: performance.now(),
      scrollTop: scroller ? scroller.scrollTop : null,
      scrollHeight: scroller ? scroller.scrollHeight : null,
      clientHeight: scroller ? scroller.clientHeight : null,
      userTop: userMsg ? userMsg.getBoundingClientRect().top : null,
      flowTop: flow ? flow.getBoundingClientRect().top : null,
      flowBottom: flow ? flow.getBoundingClientRect().bottom : null,
      statusTop: statusSlot ? statusSlot.getBoundingClientRect().top : null,
      statusHeight: statusSlot ? statusSlot.getBoundingClientRect().height : null,
      statusLabel: statusBar ? statusBar.textContent?.trim() ?? null : null
    });
    window.__raf = requestAnimationFrame(record);
  };
  window.__raf = requestAnimationFrame(record);

  const line = "这是一行用于诊断流式布局位移的文本，长度接近真实回答的每一行。\n";
  for (let i = 0; i < 40; i++) {
    await new Promise((r) => setTimeout(r, 60));
    const target = store.messages.find((m) => m.role === "assistant");
    if (target) target.content += line;
  }
  await new Promise((r) => setTimeout(r, 1200));
  const target = store.messages.find((m) => m.role === "assistant");
  if (target) target.status = "done";
  store.$patch({ isSubmitting: false, phase: "completed", activeTurnId: null });
  await new Promise((r) => setTimeout(r, 800));
  cancelAnimationFrame(window.__raf);
  return window.__samples;
});

function analyze(samples) {
  const frames = samples.filter((s) => s.userTop != null);
  const deltas = [];
  for (let i = 1; i < frames.length; i++) {
    deltas.push(frames[i].userTop - frames[i - 1].userTop);
  }
  const significant = deltas.filter((d) => Math.abs(d) > 0.6);
  let reversals = 0;
  let lastSign = 0;
  let maxJump = 0;
  for (const d of significant) {
    const sign = Math.sign(d);
    if (lastSign !== 0 && sign !== lastSign) reversals += 1;
    lastSign = sign;
    maxJump = Math.max(maxJump, Math.abs(d));
  }
  const statusHeights = [...new Set(samples.map((s) => Math.round(s.statusHeight ?? -1)))];
  const labels = [...new Set(samples.map((s) => s.statusLabel))];
  return {
    frames: frames.length,
    durationMs: frames.length ? Math.round(frames[frames.length - 1].t - frames[0].t) : 0,
    maxFrameJumpPx: Number(maxJump.toFixed(2)),
    motionReversals: reversals,
    statusSlotHeights: statusHeights,
    statusLabels: labels,
    statusSlotEverMissing: samples.some((s) => s.statusTop == null),
    firstUserTop: frames.length ? Number(frames[0].userTop.toFixed(2)) : null,
    lastUserTop: frames.length ? Number(frames[frames.length - 1].userTop.toFixed(2)) : null
  };
}

const report = {
  realRun: { analysis: analyze(realRun), samples: realRun },
  synthetic: { analysis: analyze(synthetic), samples: synthetic },
  consoleErrors
};
const fs = await import("node:fs");
fs.writeFileSync(OUT, JSON.stringify(report, null, 2));
console.log(JSON.stringify({ realRun: report.realRun.analysis, synthetic: report.synthetic.analysis, consoleErrors }, null, 2));
await browser.close();
