/**
 * Wave-17 冻结契约红相验收（L2-T）— C3 浏览器真实链路门禁 + C2-2 物理宽度测量
 *
 * 契约源（只读）：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 C2-2 / C3-1 / C3-2 / C3-3。
 *
 * 独立性声明（BASE-E2E-1）：
 *   基线 `npm run test:e2e` 即 8 failed / 12 passed —— 存量 `tests/e2e/browser-preview.spec.ts`
 *   断言 `session-sidebar-session-list`，而侧栏结构已漂移。本文件**不依赖**该存量 spec，
 *   也不去修它，可独立运行：
 *     npx playwright test tests/e2e/wave17-browser-gate.spec.ts
 *
 * 为什么 C2-2 必须放在真实浏览器：
 *   jsdom 不执行 Tailwind CSS，class 断言只是契约代理（见 tabular-nums-contract.spec.ts）。
 *   只有真实 Chromium 才能给出 `getComputedStyle(el).fontVariantNumeric` 的计算值与
 *   `getBoundingClientRect().width` 的物理像素宽度。
 *
 * 零假测试律：所有用例均有硬断言；无 skip、无吞异常、无硬编码 sleep 等待。
 *
 * wave-59 契约演进（Lead 授权迁移，原意图不变、仅迁移定位锚点）：
 *   C2-2 的 counts 行锚点由 `status-panel-toggle` 头部行迁移至
 *   `[data-testid="status-metrics-counts"]`（wave-59 F2-1/F2-2 计数位下移）；
 *   C2-2a/b/c/d 四段物理口径与阈值（0.5px / 单调不减）保持原样。
 *
 * ── wave-61 契约 §5 授权迁移（Lead 一票，原意图保真）────────────────────────────
 * 迁移点：C2-2 位点集合与 token 行锚点。
 *   原意图：对 HomeStatusPanel 的**全部指标位**逐位做 C2-2a/b/c/d 四段物理测量
 *           （等位数宽度漂移 ≤0.5px / 相邻位左边缘位移 ≤0.5px / 宽度单调不减），
 *           并保证「直接渲染数字文本的位点 ≥ 6」，防止探针降级掩盖定位失效。
 *   迁移理由：wave-61 §1 L-1/L-2/L-3 把「上下文圆环」并入计数行成为第 4 个位点
 *           （位点总数 3+4+1 = 8），并把 token 行从 `div.flex.items-center.justify-around`
 *           改为 `grid grid-cols-4` + 新增锚点 `[data-testid="status-metrics-tokens"]`；
 *           旧 class 锚点与 `measurements.length === 7` 必然失效。
 *   等价断言：0.5px 阈值（Assert 4/5）、宽度单调不减（Assert 6）、
 *           tabular-nums 计算值（Assert 3）、真实数字文本覆盖率 ≥6（Assert 2）**全部原样不变**；
 *           仅「位点总数 7 ⇒ 8」与「token 行定位方式」随契约迁移。圆环位无数字文本，
 *           走既有 `[RED DEGRADED: PROBE-NODE]` 探针路径（该位点仍参与四段测量）。
 * ── wave-61 §5.1 追加授权（Lead 二轮裁决 R2，2026-11-03）──────────────────────
 * 夹具 `seedTokenMetricFixture` 补一条 `call_tool` 工具活动（traceTimeline + toolActivities 各一条）。
 *   原意图不变：位点总数 8 与「3 个计数位全可见」必须同时成立。
 *   迁移理由：§2 E-1 使 0 值计数位整个不渲染 ⇒ `toolCallCount=0` 时 8 位点算术上不可达。
 *   等价性：只加数据，不改任何断言、阈值与测量口径。
 *
 * 作者：test-agent（L2-T）。红相锚定后本文件冻结只读，任何修改需 Lead 批准。
 */
import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

/** C3-3 证据落盘目录（契约 §2 C3-3）。 */
const EVIDENCE_DIR = path.resolve(process.cwd(), ".dev-team", "evidence", "wave-17");

interface BrowserSignals {
  consoleErrors: string[];
  pageErrors: string[];
}

/**
 * 在页面导航**之前**挂上 console / pageerror 收集器，否则会漏掉启动期错误。
 * 契约 C3-2：Console Error = 0 且未捕获异常 = 0。
 */
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

async function openHome(page: Page): Promise<void> {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("app-layout-shell")).toBeVisible();
  await expect(page.getByTestId("home-layout-shell")).toBeVisible();
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
}

// ── C3-1：核心旅程可达 ─────────────────────────────────────────────────────

test("C3-1: 真实 Chromium 加载构建产物，核心旅程可达（app-layout-shell + home-layout-shell + workspace-composer-input）", async ({ page }) => {
  // Act
  await openHome(page);

  // Assert：三个 shell 均可见（openHome 已断言），再确认 ASK/输入框可交互。
  await expect(page.getByTestId("workspace-composer-input")).toBeEnabled();
  await expect(page.getByTestId("workspace-submit-action")).toBeVisible();
  await expect(page.getByTestId("home-right-sidebar-shell")).toHaveAttribute("data-open", "true");
});

// ── C3-2：Console Error = 0 ────────────────────────────────────────────────

test("C3-2: 首页加载 + 一次真实提交旅程全程 Console Error = 0 且未捕获异常 = 0", async ({ page }) => {
  // Arrange：收集器必须在导航前挂载。
  const signals = attachSignalCollectors(page);

  // Act：完整走一遍真实旅程 —— 加载首页 → 输入 → 提交。
  await openHome(page);
  const prompt = "wave-17 browser gate probe";
  await page.getByTestId("workspace-composer-input").fill(prompt);
  await page.getByTestId("workspace-submit-action").click();

  // 显式条件等待（禁止硬编码 sleep）：用户消息必须落进会话正文。
  await expect(page.getByTestId("workspace-content-column")).toContainText(prompt);

  // Assert：零 console.error、零未捕获异常。
  expect(
    signals.consoleErrors,
    `浏览器 Console Error 必须为 0，实际 ${signals.consoleErrors.length} 条：\n${signals.consoleErrors.join("\n")}`
  ).toHaveLength(0);
  expect(
    signals.pageErrors,
    `浏览器未捕获异常必须为 0，实际 ${signals.pageErrors.length} 条：\n${signals.pageErrors.join("\n")}`
  ).toHaveLength(0);
});

// ── C3-3：关键旅程截图落盘 ─────────────────────────────────────────────────

test("C3-3: 关键旅程截图落盘 .dev-team/evidence/wave-17/（须拍到工作区，非启动闪屏）", async ({ page }) => {
  // Arrange
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  const splash = page.locator("#app-loading-screen");

  // Act 1：导航后**立即**确认启动闪屏存在。
  //   闪屏由 `index.html` 静态注入（`position:fixed; inset:0; z-index:999999` 全屏遮罩），
  //   故在 DOMContentLoaded 时刻必然在 DOM 中。此断言不可省：
  //   若闪屏元素从不出现，下方 `toHaveCount(0)` 会恒真 —— 那就是空等待式的假测试。
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(
    splash,
    "启动闪屏必须先存在于 DOM，否则下方『等待闪屏移除』退化为恒真的空等待（假测试）"
  ).toHaveCount(1);

  // Act 2：核心旅程可达（与 openHome 一致；此处内联以保证闪屏前置断言紧贴导航、无时序竞争）。
  await expect(page.getByTestId("app-layout-shell")).toBeVisible();
  await expect(page.getByTestId("home-layout-shell")).toBeVisible();
  await expect(page.getByTestId("workspace-composer-input")).toBeVisible();
  await page.getByTestId("workspace-composer-input").fill("wave-17 evidence screenshot");

  // Act 3（REV-3 核心修订）：显式等待闪屏从 DOM 移除。
  //   `src/main.ts` 在挂载后 `setTimeout(3000)` 加 `fade-out`，再 350ms 后 `splash.remove()`。
  //   移除前截图只会拍到闪屏 —— Playwright 的 `toBeVisible()` 对**被覆盖但未隐藏**的元素仍判为
  //   可见，故 C3-1/C3-2 的断言成立并不蕴含"画面是工作区"。
  //   此处用显式条件轮询（无硬编码 sleep）；若闪屏因任何原因不被移除，本用例超时失败，
  //   而不是拍下一张假凭据。
  await expect(
    splash,
    "启动闪屏必须在截图前从 DOM 移除（REV-3：全屏 fixed 遮罩会被 toBeVisible 判为『未隐藏』，导致截图拍到闪屏）"
  ).toHaveCount(0);

  // Act 4（证伪组合 · 为何能证伪"拍到的是闪屏"）：
  //   ① `#app-loading-screen` count = 0 → 全屏遮罩已不在 DOM，物理上不可能被绘制到截图里；
  //   ② composer 同时满足 可见 + 可交互（`toBeEnabled` 走 actionability 检查）+ 处于视口内
  //      + 持有本次填入文本 → 证明工作区已真实布局进视口且输入链路可用，
  //      而不是"被遮罩盖住但 DOM 仍可见"这一 Playwright 可见性盲区；
  //   ③ `app-layout-shell` 处于视口内 + 右侧栏 `data-open=true` → 证明三栏布局已展开
  //      （截图中应出现左侧栏与右侧状态栏区域）。
  const composer = page.getByTestId("workspace-composer-input");
  await expect(composer).toBeVisible();
  await expect(composer).toBeEnabled();
  await expect(composer).toBeInViewport();
  await expect(composer).toHaveValue("wave-17 evidence screenshot");
  await expect(page.getByTestId("app-layout-shell")).toBeInViewport();
  await expect(page.getByTestId("home-right-sidebar-shell")).toHaveAttribute("data-open", "true");

  // Act 4b（遮挡命中测试 · 直接证伪"画面被全屏遮罩覆盖"）：
  //   `document.elementFromPoint(x, y)` 返回该坐标处**最上层**的可命中元素。
  //   启动闪屏是 `position:fixed; inset:0; z-index:999999` 的全屏遮罩，且**不是** `#app` /
  //   `app-layout-shell` 的后代；若它仍覆盖画面，命中结果必然是该遮罩本身，
  //   于是 `el.contains(top)` / `top.contains(el)` 均为 false → 本断言失败。
  //   这正是 Playwright `toBeVisible()` 的盲区（元素被覆盖但仍判"可见"），
  //   也正是旧版 C3-3 拍下闪屏却全绿的原因 —— 故此条命中测试具备**真实证伪能力**。
  const occlusionProbe = await page.evaluate(() => {
    const hitTest = (selector: string) => {
      const element = document.querySelector(selector);
      if (!element) {
        return { ok: false, hit: "(目标元素不存在)" };
      }
      const rect = element.getBoundingClientRect();
      const top = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
      if (!top) {
        return { ok: false, hit: "(该坐标无命中元素)" };
      }
      return {
        ok: top === element || element.contains(top) || top.contains(element),
        hit: `${top.tagName.toLowerCase()}${top.id ? `#${top.id}` : ""}`
      };
    };
    return {
      composer: hitTest('[data-testid="workspace-composer-input"]'),
      shell: hitTest('[data-testid="app-layout-shell"]')
    };
  });
  expect(
    occlusionProbe.composer.ok,
    `composer 中心点最上层可命中元素必须属于 composer 自身（证明无全屏遮罩覆盖）；实际命中：${occlusionProbe.composer.hit}`
  ).toBe(true);
  expect(
    occlusionProbe.shell.ok,
    `app-layout-shell 中心点最上层可命中元素必须属于工作区外壳（证明无全屏遮罩覆盖）；实际命中：${occlusionProbe.shell.hit}`
  ).toBe(true);

  // Act 5：截图
  const screenshotPath = path.join(EVIDENCE_DIR, "wave17-browser-gate-home.png");
  await page.screenshot({ path: screenshotPath });

  // Assert 1：物理文件必须存在且非空（不能只断言"截图函数没抛错"）。
  expect(fs.existsSync(screenshotPath), `截图必须落盘：${screenshotPath}`).toBe(true);
  const sizeBytes = fs.statSync(screenshotPath).size;
  expect(sizeBytes, `截图必须非空（> 10KB），实际 ${sizeBytes} 字节`).toBeGreaterThan(10 * 1024);

  // Assert 2（截图后复核）：截图期间遮罩不得重新出现 / 仍然存在。
  await expect(splash, "截图后启动闪屏仍不得存在于 DOM").toHaveCount(0);
});

// ── C2-2：真实浏览器物理宽度稳定性测量 ─────────────────────────────────────

/**
 * [C2-2 口径 — 依契约 §6 REV-1（2026-10-16 Lead 一票裁决）]
 *
 * 原 C2-2 字面口径（"`9` → `12` → `1234` 自身宽度变化 ≤ 0.5px"）经本 Agent 真实浏览器实测
 * 证明**物理不可满足**，已由 Lead 裁决修订为 C2-2a/b/c/d（契约 §2 + §6 REV-1）。
 *
 * 关键实测证据（Chromium，dist 构建产物，viewport 1440×900）：
 * - 自身宽度随位数变化：`9` = 23px → `12` = 26px → `1234` = 40px（Δ=17px）
 * - 注入 `font-variant-numeric: tabular-nums` 后：`9` = 23px → `12` = 30px → `1234` = 44px（Δ=21px）
 *   —— `tabular-nums` 只统一**等位数**数字的 advance width，无法让 1 位与 4 位等宽。
 * - 等位数异字形（tabular-nums 的真实语义）：`1/9/0` 基线 Δ=3px、`1111/9999/0000` 基线 Δ=4px；
 *   注入 tabular-nums 后两者 Δ=0。
 *
 * REV-1 拆解与本用例的对应断言：
 * - **C2-2a**（红相驱动）：`getComputedStyle(el).fontVariantNumeric === "tabular-nums"` → Assert 3
 * - **C2-2b**：等位数异字形宽度漂移 ≤ 0.5px（`1/9/0` 与 `1111/9999/0000`）→ Assert 4
 * - **C2-2c**（用户可见缺陷本体）：位数变化（`9`→`12`→`1234`）时**相邻指标位包围盒左边缘位移 ≤ 0.5px**
 *   → Assert 5；由 task-4 已授权的容器 `min-w` 保证（C2-5 从未禁止 `min-width`）
 * - **C2-2d**（合理性哨兵）：三档自身宽度 `9` ≤ `12` ≤ `1234` 单调不减，**不**断言相等 → Assert 6
 */

interface NeighborEvidence {
  label: string;
  /** 该相邻位在 3 个样本下的包围盒左边缘。 */
  lefts: number[];
  delta: number;
}

interface SlotMeasurement {
  label: string;
  /** 同一布局行分组：counts（3 个计数位）/ tokens（4 个 token 指标位）。C2-2c 的"相邻"以此为准。 */
  rowId: "counts" | "tokens";
  source: "real-text" | "probe-node";
  sampleText: string;
  fontVariantNumeric: string;
  /** C2-2b：等位数异字形样本（1 位 / 4 位）的宽度序列与漂移。 */
  oneDigitWidths: number[];
  fourDigitWidths: number[];
  equalLengthMaxDelta: number;
  /** C2-2d：契约原文的位数递增样本宽度（仅做单调性断言）。 */
  literalWidths: number[];
  /** C2-2c：位数变化时，同行**相邻**位点包围盒左边缘的最大位移。 */
  neighborLeftMaxDelta: number;
  neighborEvidence: NeighborEvidence[];
  /** 自身左边缘在 3 个样本下的位移（物理证据，随失败信息一并输出）。 */
  ownLeftDelta: number;
}

/**
 * C2-2 夹具数据注入（Lead 追加裁定 B，2026-11-01）：
 *
 * 背景：C2-2 的 Assert 2 要求「至少 6 个指标位直接渲染数字文本」，用于防止探针降级掩盖定位失效；
 * 但 fresh page 没有任何会话/调用数据，4 个 token 指标位一律渲染 `—`（走 `[RED DEGRADED: PROBE-NODE]`），
 * 使该前提在环境上不可满足（**基线原版 C2-2 同样红**，见 `.dev-team/wave59-green-receipts.md` §6.2）。
 *
 * 本夹具只**注入数据**让四个 token 位渲染真实数字，不改任何断言与阈值（0.5px / 单调不减 全部原样）。
 * 注入源 = pinia runtime store 的一条已完成 turn（与 wave-17 C2-1 单测夹具同构）：
 * - `contextDisplayTokens` ← providerCallRecords[].contextInputTokens ?? inputTokens（缺省回落 timeline call_model.inputTokens）
 * - `sessionOutputTokensTotal` ← turn.outputTokens
 * - `sessionCacheHitTokensTotal` ← Σ providerCallRecords[].cacheHitInputTokens
 * - `sessionTokenGenerationSpeed` ← outputTokens / (durationMs − firstTokenLatencyMs)
 */
async function seedTokenMetricFixture(page: Page) {
  await page.evaluate(() => {
    const appEl = document.querySelector("#app") as unknown as Record<string, unknown>;
    const globalProperties = (appEl.__vue_app__ as Record<string, unknown>).config as Record<string, unknown>;
    const pinia = globalProperties.globalProperties as Record<string, unknown>;
    const stores = (pinia.$pinia as Record<string, unknown>)._s as Map<string, Record<string, unknown>>;
    const runtime = stores.get("runtime") as { $patch: (value: unknown) => void };

    runtime.$patch({
      sessionId: "w59-c22-fixture-session",
      messages: [],
      turnTraceHistory: [
        {
          turnId: "w59-c22-fixture-turn",
          sessionId: "w59-c22-fixture-session",
          title: "wave-59 C2-2 物理测量夹具",
          phase: "completed",
          traceSteps: [],
          traceTimeline: [
            {
              id: "w59-c22-call-model",
              kind: "call_model",
              label: "调用模型",
              state: "completed",
              sequence: 1,
              inputTokens: 1234,
              outputTokens: 567,
              cacheHitInputTokens: 89,
              firstTokenLatencyMs: 321,
              turnDurationMs: 4200,
              durationMs: 4200
            },
            // wave-61 §5.1 追加授权（R2）：本夹具必须补一条 call_tool 工具活动。
            // 理由：契约 §2 E-1 让「0 值计数位」整个不渲染 ⇒ 不补该活动时 toolCallCount=0 被隐藏，
            // 「位点总数 8」在算术上不可达（3 计数 + 4 token + 1 圆环 需要三个计数位全可见）。
            // 只加数据，不改任何断言与阈值。
            {
              id: "w59-c22-call-tool",
              kind: "call_tool",
              label: "调用工具",
              state: "completed",
              sequence: 2,
              durationMs: 120
            }
          ],
          toolActivities: [
            {
              id: "w59-c22-tool-activity",
              name: "read_file",
              status: "done",
              description: "wave-61 R2 夹具：保证 toolCallCount 有值"
            }
          ],
          providerCallRecords: [
            {
              requestKind: "initial_request",
              providerSource: "provider_decision",
              providerMode: "live",
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

  // 条件等待（禁止硬编码等待）：等到四个 token 位确实渲染出数字文本再开始测量，
  // 避免在 Vue 重渲染落地前读到 `—`；超时即说明注入未生效（属夹具失效，必须显式暴露而非静默降级）。
  await page.waitForFunction(
    () => {
      const tokenSlots = [
        "status-metric-input-tokens",
        "status-metric-output-tokens",
        "status-metric-cache-tokens",
        "status-metric-token-speed"
      ];
      return tokenSlots.every((testid) => {
        const element = document.querySelector(`[data-testid="${testid}"]`);
        return Boolean(element) && /\d/.test(element?.textContent ?? "");
      });
    },
    undefined,
    { timeout: 10_000 }
  );
}

test("C2-2: HomeStatusPanel 指标位 tabular-nums 计算样式 + 等位数宽度稳定 + 相邻位左边缘不位移", async ({ page }) => {
  // Arrange
  await openHome(page);
  const statusPanel = page.getByTestId("status-panel-toggle");
  await expect(statusPanel).toBeVisible();
  // 夹具注入（只注入数据；断言与阈值未改动）
  await seedTokenMetricFixture(page);

  // Act：在真实页面的真实元素上做物理测量（全部读数在同一次 evaluate 内同步完成，
  //      避免 Vue 重渲染回写导致读数漂移）。
  const measurements: SlotMeasurement[] = await page.evaluate(() => {
    interface RawSlot {
      label: string;
      rowId: "counts" | "tokens";
      element: Element;
    }

    const slots: RawSlot[] = [];

    // wave-59 F2 契约演进（Lead 授权迁移，仅迁移定位锚点）：
    // 三个计数位由 status-panel-toggle 头部行下移至 token 卡内的 status-metrics-counts 行。
    // wave-61 §5 迁移：改用既有 testid 前缀识别 3 个计数位（排除同行的圆环位），
    // 并把圆环位作为第 8 个位点纳入测量（位点总数 3 + 4 + 1 = 8）。
    const countsRow = document.querySelector('[data-testid="status-metrics-counts"]');
    const countLabels = ["轮次计数位", "模型调用计数位", "工具调用计数位"];
    const countCarriers = countsRow ? Array.from(countsRow.querySelectorAll('[data-testid^="status-metric-"]')) : [];
    countCarriers.forEach((element, index) => {
      slots.push({ label: countLabels[index] ?? `计数位#${index}`, rowId: "counts", element });
    });
    const ringSlotElement = countsRow ? countsRow.querySelector('[data-testid="status-context-usage"]') : null;
    if (ringSlotElement) {
      slots.push({ label: "上下文圆环位", rowId: "counts", element: ringSlotElement });
    }

    const tokenLabels = ["输入 token 位", "输出 token 位", "缓存 token 位", "生成速度位"];
    // wave-61 §5 迁移：token 行锚点由 `div.justify-around` 改为契约 testid 锚点。
    const tokenRow = document.querySelector('[data-testid="status-metrics-tokens"]');
    if (tokenRow) {
      Array.from(tokenRow.children)
        .filter((element) => element.matches("span.inline-flex"))
        .forEach((element, index) => {
          slots.push({ label: tokenLabels[index] ?? `token 位#${index}`, rowId: "tokens", element });
        });
    }

    const lastNumericTextNode = (root: Element): Text | null => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      let last: Text | null = null;
      while (walker.nextNode()) {
        const node = walker.currentNode as Text;
        if (/\d/.test(node.data)) {
          last = node;
        }
      }
      return last;
    };

    /** 契约原文的位数递增样本（C2-2c / C2-2d 共用）。 */
    const LITERAL_SAMPLES = ["9", "12", "1234"];

    return slots.map((slot) => {
      const existing = lastNumericTextNode(slot.element);
      let textNode: Text;
      let source: "real-text" | "probe-node";
      let probe: Text | null = null;
      let sampleText = "";
      if (existing) {
        textNode = existing;
        source = "real-text";
        sampleText = existing.data.trim();
      } else {
        // [RED DEGRADED: PROBE-NODE]
        // 「生成速度」位在无速度数据时渲染 "—"（无数字文本节点）。此时在该**真实位点内部**
        // 注入一个临时文本节点：它完整复用该位点真实的 font-family / font-size /
        // font-variant-numeric 继承上下文，测完立即移除，不改变被测 CSS 上下文。
        probe = document.createTextNode("");
        slot.element.appendChild(probe);
        textNode = probe;
        source = "probe-node";
        sampleText = "—(无数据，注入探针文本)";
      }

      const measureOwnWidth = (value: string): number => {
        textNode.data = value;
        return slot.element.getBoundingClientRect().width;
      };

      // C2-2b：等位数异字形（同位数、不同数字）—— tabular-nums 的真实物理语义。
      const oneDigitWidths = ["1", "9", "0"].map(measureOwnWidth);
      const fourDigitWidths = ["1111", "9999", "0000"].map(measureOwnWidth);

      // C2-2c：位数变化时，**同行相邻位**的包围盒左边缘位移。
      //   每个样本下同步读取整行所有位点的 left，才能捕捉"这一位变宽把旁边推走"的真实联动。
      const rowMates = slots.filter((mate) => mate.rowId === slot.rowId);
      const leftsBySample: number[][] = [];
      const literalWidths: number[] = [];
      for (const value of LITERAL_SAMPLES) {
        textNode.data = value;
        literalWidths.push(slot.element.getBoundingClientRect().width);
        leftsBySample.push(rowMates.map((mate) => mate.element.getBoundingClientRect().left));
      }

      const neighborEvidence: NeighborEvidence[] = rowMates.map((mate, mateIndex) => {
        const lefts = leftsBySample.map((sample) => sample[mateIndex]!);
        return {
          label: mate.label,
          lefts,
          delta: Math.max(...lefts) - Math.min(...lefts)
        };
      });
      // "相邻指标位"= 同行其他位点；自身位移单独作为证据记录。
      const neighborLeftMaxDelta = Math.max(
        0,
        ...neighborEvidence.filter((entry) => entry.label !== slot.label).map((entry) => entry.delta)
      );
      const ownLeftDelta =
        neighborEvidence.find((entry) => entry.label === slot.label)?.delta ?? 0;

      const equalLengthMaxDelta = Math.max(
        Math.max(...oneDigitWidths) - Math.min(...oneDigitWidths),
        Math.max(...fourDigitWidths) - Math.min(...fourDigitWidths)
      );

      if (probe) {
        probe.remove();
      } else {
        textNode.data = sampleText;
      }

      return {
        label: slot.label,
        rowId: slot.rowId,
        source,
        sampleText,
        fontVariantNumeric: getComputedStyle(slot.element).fontVariantNumeric,
        oneDigitWidths,
        fourDigitWidths,
        equalLengthMaxDelta,
        literalWidths,
        neighborLeftMaxDelta,
        neighborEvidence,
        ownLeftDelta
      };
    });
  });

  // Assert 1（位点完整性）：契约 C2-1 的 4 个 token 位 + 3 个计数位，
  // 加上 wave-61 §1 L-1/L-2 并入计数行的上下文圆环位 = 8 个位点。
  expect(
    measurements.length,
    `HomeStatusPanel 必须渲染 8 个指标位（3 计数 + 4 token + 1 圆环），实际 ${measurements.length} 个；测得：${JSON.stringify(measurements.map((slot) => slot.label))}`
  ).toBe(8);

  // Assert 2（真实数字文本覆盖率）：防止探针降级路径掩盖选择器落空。
  //   轮次/模型调用/工具调用 + 输入/输出/缓存 这 6 个位点在首页必然渲染数字（夹具注入后为真值），
  //   只有「生成速度」与「上下文圆环」可能走探针路径。因此真实数字文本位点数必须 ≥ 6（阈值未改）。
  const realTextSlots = measurements.filter((slot) => slot.source === "real-text");
  expect(
    realTextSlots.length,
    `至少 6 个指标位必须直接渲染数字文本（防探针降级掩盖定位失效）；实际 ${realTextSlots.length} 个：${JSON.stringify(measurements.map((slot) => `${slot.label}=${slot.source}`))}`
  ).toBeGreaterThanOrEqual(6);

  for (const slot of measurements) {
    const evidence = JSON.stringify(
      slot.neighborEvidence.map((entry) => ({ 相邻位: entry.label, lefts: entry.lefts, delta: entry.delta }))
    );

    // Assert 3（C2-2a · 红相驱动断言）：真实 Chromium 计算值必须为 tabular-nums。
    expect(
      slot.fontVariantNumeric,
      `${slot.label}: getComputedStyle(...).fontVariantNumeric 必须为 "tabular-nums"，实际 "${slot.fontVariantNumeric}"`
    ).toBe("tabular-nums");

    // Assert 4（C2-2b）：等位数异字形宽度漂移 ≤ 0.5px。
    expect(
      slot.equalLengthMaxDelta,
      `${slot.label}: 等位数不同数字导致自身宽度漂移 ${slot.equalLengthMaxDelta}px（> 0.5px）；1 位档 ${JSON.stringify(slot.oneDigitWidths)}，4 位档 ${JSON.stringify(slot.fourDigitWidths)}`
    ).toBeLessThanOrEqual(0.5);

    // Assert 5（C2-2c · 用户可见缺陷本体）：位数变化时相邻指标位包围盒左边缘位移 ≤ 0.5px。
    expect(
      slot.neighborLeftMaxDelta,
      `${slot.label}: 位数变化（9 → 12 → 1234）推移相邻指标位左边缘 ${slot.neighborLeftMaxDelta}px（> 0.5px）；自身左边缘位移 ${slot.ownLeftDelta}px；相邻位左边缘证据 ${evidence}`
    ).toBeLessThanOrEqual(0.5);

    // Assert 6（C2-2d · 合理性哨兵）：位数递增时自身宽度必须单调不减（不断言相等）。
    expect(
      slot.literalWidths[0]! <= slot.literalWidths[1]! && slot.literalWidths[1]! <= slot.literalWidths[2]!,
      `${slot.label}: 9 → 12 → 1234 宽度必须单调不减，实际 ${JSON.stringify(slot.literalWidths)}`
    ).toBe(true);
  }
});
