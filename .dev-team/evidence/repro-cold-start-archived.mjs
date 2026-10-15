// 复现：归档当前会话后冷启动是否落在已归档会话（wave-12 同族残留）
import pw from "/home/dm/pony-agent/node_modules/@playwright/test/index.js";
const { chromium } = pw;

const URL = "http://127.0.0.1:4176/";
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
const errors = [];
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));

const readStore = () =>
  page.evaluate(() => {
    const store = document.querySelector("#app").__vue_app__.config.globalProperties.$pinia._s.get("runtime");
    return {
      sessionId: store.sessionId,
      phase: store.phase,
      messageCount: store.messages.length,
      firstUserText: store.messages.find((m) => m.role === "user")?.content ?? null,
      list: store.sessionList.map((s) => ({ id: s.conversationId, archived: Boolean(s.archived), title: s.title, updatedAtMs: s.updatedAtMs })),
      workspaceEmpty: Boolean(document.querySelector('[data-testid="workspace-empty-state"]'))
    };
  });

async function boot() {
  await page.goto(URL, { waitUntil: "domcontentloaded" });
  await page.waitForSelector('[data-testid="workspace-composer-input"]');
  await page.waitForFunction(() => !document.getElementById("app-loading-screen"), undefined, { timeout: 20000 });
}

// 1) 干净启动 → 发一条消息（生成持久化会话 A）
await page.goto(URL, { waitUntil: "domcontentloaded" });
await page.evaluate(() => { window.localStorage.clear(); window.sessionStorage.clear(); });
await boot();
await page.getByTestId("workspace-composer-input").fill("归档前的一条消息 ALPHA");
await page.getByTestId("workspace-submit-action").click();
await page.waitForTimeout(1500);
const afterSend = await readStore();

// 2) 归档当前会话
const archivedId = afterSend.sessionId;
await page.evaluate(async (id) => {
  const store = document.querySelector("#app").__vue_app__.config.globalProperties.$pinia._s.get("runtime");
  await store.archiveSession(id);
}, archivedId);
await page.waitForTimeout(400);
const afterArchive = await readStore();

// 3) 冷启动（整页重载）
await boot();
await page.waitForTimeout(800);
const afterReload = await readStore();

console.log(JSON.stringify({
  afterSend: { sessionId: afterSend.sessionId, messages: afterSend.messageCount, list: afterSend.list },
  afterArchive: { sessionId: afterArchive.sessionId, messages: afterArchive.messageCount, workspaceEmpty: afterArchive.workspaceEmpty, list: afterArchive.list },
  afterReload: { sessionId: afterReload.sessionId, phase: afterReload.phase, messages: afterReload.messageCount, firstUserText: afterReload.firstUserText, workspaceEmpty: afterReload.workspaceEmpty, list: afterReload.list },
  verdict: afterReload.firstUserText ? "COLD-START 落在已归档会话（复现成功）" : "冷启动未落在已归档会话",
  consoleErrors: errors
}, null, 2));
await browser.close();
