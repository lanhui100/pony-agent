// 诊断探针（临时）：判定流式期单帧位移到底来自「内容真实增长」还是「滚动追赶超调」
import pw from "/home/dm/pony-agent/node_modules/@playwright/test/index.js";
const { chromium } = pw;
import fs from "node:fs";

const URL = process.env.PROBE_URL ?? "http://127.0.0.1:4176/";
const OUT = "/home/dm/pony-agent/.tmp/m3-probe.json";

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const consoleErrors = [];
page.on("console", (m) => { if (m.type() === "error") consoleErrors.push(m.text()); });
page.on("pageerror", (e) => consoleErrors.push(`pageerror: ${e.message}`));

await page.goto(URL, { waitUntil: "domcontentloaded" });
await page.evaluate(() => { window.localStorage.clear(); window.sessionStorage.clear(); });
await page.goto(URL, { waitUntil: "networkidle" });
await page.waitForSelector('[data-testid="workspace-composer-input"]');
await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, { timeout: 20000 });

await page.evaluate(({ totalLines, interval }) => {
  const w = window;
  const appEl = document.querySelector("#app");
  const pinia = appEl.__vue_app__.config.globalProperties.$pinia;
  const store = pinia._s.get("runtime");
  const turnId = `probe-turn-${Date.now()}`;
  store.$patch({
    sessionId: "probe-session", sessionOperation: null, sessionError: null, error: null,
    phase: "calling_model", isSubmitting: true, activeTurnId: turnId,
    activeTurnStartedAtMs: Date.now(), toolActivities: [], streamBufferText: "", streamBufferReasoning: "",
    messages: [
      { id: `probe-user-${turnId}`, turnId, role: "user", content: "probe" },
      { id: `probe-assistant-${turnId}`, turnId, role: "assistant", content: "", status: "pending" }
    ]
  });
  const pick = (s) => document.querySelector(s);
  const viewport = pick(".workspace-timeline-viewport");
  w.__probe = { frames: [], growth: [] };
  const record = () => {
    const flow = pick('[data-testid="assistant-streaming-flow"]');
    const slot = pick('[data-testid="workspace-runtime-status-slot"]');
    w.__probe.frames.push({
      t: performance.now(),
      scrollTop: viewport ? viewport.scrollTop : null,
      scrollHeight: viewport ? viewport.scrollHeight : null,
      flowTop: flow ? flow.getBoundingClientRect().top : null,
      flowHeight: flow ? flow.getBoundingClientRect().height : null,
      slotTop: slot ? slot.getBoundingClientRect().top : null,
      slotVisible: slot ? slot.getAttribute("data-visible") : null,
      slotOpacity: slot ? getComputedStyle(slot).opacity : null,
      slotVisibility: slot ? getComputedStyle(slot).visibility : null
    });
    w.__probe.raf = requestAnimationFrame(record);
  };
  w.__probe.raf = requestAnimationFrame(record);
  const line = "这是一行用于 wave-11 状态流稳定性诊断的合成文本，长度接近真实回答的一行内容。\n";
  let emitted = 0;
  w.__probe.growth = setInterval(() => {
    emitted += 1;
    const target = store.messages.find((m) => m.role === "assistant");
    if (target) target.content += line;
    if (emitted >= totalLines) { clearInterval(w.__probe.growth); w.__probe.done = true; }
  }, interval);
}, { totalLines: 50, interval: 60 });

await page.waitForFunction(() => window.__probe?.done === true, undefined, { timeout: 30000 });
await page.waitForTimeout(1500);

const captured = await page.evaluate(() => {
  cancelAnimationFrame(window.__probe.raf);
  return { frames: window.__probe.frames, debug: window.__ponyScrollDebugBuffer ?? [] };
});
const data = captured.frames;
fs.writeFileSync(OUT, JSON.stringify({ frames: data, debug: captured.debug, consoleErrors }, null, 2));

// 找出滚动事件里 scrollTop 跳变 > 20px 的事件名
const dbg = captured.debug.filter((e) => typeof e.viewportScrollTop === "number");
for (let i = 1; i < dbg.length; i++) {
  const d = dbg[i].viewportScrollTop - dbg[i - 1].viewportScrollTop;
  if (Math.abs(d) > 20) {
    console.log("SCROLL-EVENT", JSON.stringify({ event: dbg[i].event, at: dbg[i].at, dScroll: +d.toFixed(1), from: dbg[i-1].viewportScrollTop, to: dbg[i].viewportScrollTop, prevEvent: dbg[i-1].event, targetTop: dbg[i].programmaticScrollTargetTop ?? null, mode: dbg[i].programmaticScrollTargetMode ?? null }));
  }
}
console.log("debug events:", dbg.length, "| frame/at range:", data[0] ? new Date(Date.now()).toString() : "");

const f = data.filter((x) => x.flowTop != null && x.scrollTop != null);
let worst = [];
for (let i = 1; i < f.length; i++) {
  const dFlow = f[i].flowTop - f[i - 1].flowTop;
  const dScroll = f[i].scrollTop - f[i - 1].scrollTop;
  const dFlowH = (f[i].flowHeight ?? 0) - (f[i - 1].flowHeight ?? 0);
  const dScrollH = f[i].scrollHeight - f[i - 1].scrollHeight;
  worst.push({ i, dt: +(f[i].t - f[i - 1].t).toFixed(1), dFlow: +dFlow.toFixed(1), dScroll: +dScroll.toFixed(1), dFlowH: +dFlowH.toFixed(1), dScrollH: +dScrollH.toFixed(1) });
}
worst.sort((a, b) => Math.abs(b.dFlow) - Math.abs(a.dFlow));
const big = worst.filter((x) => Math.abs(x.dFlow) > 25);
console.log("frames", f.length, "| frames with |dFlow|>25:", big.length);
console.log("top 12 by |dFlow|:");
big.slice(0, 12).forEach((x) => console.log(JSON.stringify(x)));
const slotTops = f.filter((x) => x.slotTop != null).map((x) => x.slotTop);
console.log("slotTop range:", Math.max(...slotTops) - Math.min(...slotTops));
console.log("consoleErrors:", consoleErrors.length, consoleErrors.slice(0, 3));
await browser.close();
