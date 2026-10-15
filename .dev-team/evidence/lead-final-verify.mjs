// Lead 终验：流式期状态条真实隐藏的视觉 + 计算样式取证（临时脚本）
import pw from "/home/dm/pony-agent/node_modules/@playwright/test/index.js";
const { chromium } = pw;

const URL = "http://127.0.0.1:4176/";
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const errors = [];
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));

await page.goto(URL, { waitUntil: "domcontentloaded" });
await page.evaluate(() => { window.localStorage.clear(); window.sessionStorage.clear(); });
await page.goto(URL, { waitUntil: "networkidle" });
await page.waitForSelector('[data-testid="workspace-composer-input"]');
await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, { timeout: 20000 });

// 合成流式：calling_model + pending assistant 持续增长正文（无工具活动）
await page.evaluate(() => {
  const appEl = document.querySelector("#app");
  const store = appEl.__vue_app__.config.globalProperties.$pinia._s.get("runtime");
  const turnId = "lead-verify-turn";
  store.$patch({
    sessionId: "lead-verify", sessionOperation: null, sessionError: null, error: null,
    phase: "calling_model", isSubmitting: true, activeTurnId: turnId,
    activeTurnStartedAtMs: Date.now(), toolActivities: [], streamBufferText: "", streamBufferReasoning: "",
    messages: [
      { id: "lead-user", turnId, role: "user", content: "请解释一下这个优化" },
      { id: "lead-assistant", turnId, role: "assistant", content: "", status: "pending" }
    ]
  });
  const line = "这是一行正在流式输出的回答正文，用于验证状态条在最终回答 streaming 期间已隐藏。\n";
  let n = 0;
  window.__grow = setInterval(() => {
    n += 1;
    const target = store.messages.find((m) => m.role === "assistant");
    if (target) target.content += line;
    if (n >= 30) clearInterval(window.__grow);
  }, 80);
});

// 等待正文真正在流 + 显隐过渡（200ms）完成
await page.waitForTimeout(1400);
const probe = await page.evaluate(() => {
  const slot = document.querySelector('[data-testid="workspace-runtime-status-slot"]');
  const flow = document.querySelector('[data-testid="assistant-streaming-flow"]');
  const style = slot ? getComputedStyle(slot) : null;
  return {
    dataVisible: slot ? slot.getAttribute("data-visible") : null,
    ariaHidden: slot ? slot.getAttribute("aria-hidden") : null,
    opacity: style ? style.opacity : null,
    visibility: style ? style.visibility : null,
    pointerEvents: style ? style.pointerEvents : null,
    slotTop: slot ? Number(slot.getBoundingClientRect().top.toFixed(1)) : null,
    slotHeight: slot ? Number(slot.getBoundingClientRect().height.toFixed(1)) : null,
    flowVisible: Boolean(flow && flow.getBoundingClientRect().height > 100),
    labelText: document.querySelector('[data-testid="agent-runtime-status-label"]')?.textContent ?? null
  };
});
await page.screenshot({ path: "/home/dm/pony-agent/docs/release/2026-10-15-pa-status-flow/after-streaming-status-hidden.png" });

// 反向对照：停止流式（isSubmitting=false）后槽位应随 run 结束卸载
console.log(JSON.stringify({ probe, consoleErrors: errors }, null, 2));
await browser.close();
