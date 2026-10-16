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

test("C3-3: 关键旅程截图落盘 .dev-team/evidence/wave-17/", async ({ page }) => {
  // Arrange
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });

  // Act
  await openHome(page);
  await page.getByTestId("workspace-composer-input").fill("wave-17 evidence screenshot");
  await expect(page.getByTestId("workspace-composer-input")).toHaveValue("wave-17 evidence screenshot");

  const screenshotPath = path.join(EVIDENCE_DIR, "wave17-browser-gate-home.png");
  await page.screenshot({ path: screenshotPath });

  // Assert：物理文件必须存在且非空（不能只断言"截图函数没抛错"）。
  expect(fs.existsSync(screenshotPath), `截图必须落盘：${screenshotPath}`).toBe(true);
  const sizeBytes = fs.statSync(screenshotPath).size;
  expect(sizeBytes, `截图必须非空（> 10KB），实际 ${sizeBytes} 字节`).toBeGreaterThan(10 * 1024);

  // Assert：截图内容对得上关键旅程 —— 输入框仍持有刚填入的文本。
  await expect(page.getByTestId("workspace-composer-input")).toHaveValue("wave-17 evidence screenshot");
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

test("C2-2: HomeStatusPanel 指标位 tabular-nums 计算样式 + 等位数宽度稳定 + 相邻位左边缘不位移", async ({ page }) => {
  // Arrange
  await openHome(page);
  const statusPanel = page.getByTestId("status-panel-toggle");
  await expect(statusPanel).toBeVisible();

  // Act：在真实页面的真实元素上做物理测量（全部读数在同一次 evaluate 内同步完成，
  //      避免 Vue 重渲染回写导致读数漂移）。
  const measurements: SlotMeasurement[] = await page.evaluate(() => {
    interface RawSlot {
      label: string;
      rowId: "counts" | "tokens";
      element: Element;
    }

    const slots: RawSlot[] = [];

    const header = document.querySelector('[data-testid="status-panel-toggle"]');
    const countLabels = ["轮次计数位", "模型调用计数位", "工具调用计数位"];
    const countCarriers = header ? Array.from(header.querySelectorAll("span.inline-flex")) : [];
    countCarriers.forEach((element, index) => {
      slots.push({ label: countLabels[index] ?? `计数位#${index}`, rowId: "counts", element });
    });

    const tokenLabels = ["输入 token 位", "输出 token 位", "缓存 token 位", "生成速度位"];
    const tokenRow = document.querySelector("div.justify-around");
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

  // Assert 1（位点完整性）：契约 C2-1 明列 4 个 token 指标位 + 3 个计数位。
  expect(
    measurements.length,
    `HomeStatusPanel 必须渲染 7 个指标位，实际 ${measurements.length} 个；测得：${JSON.stringify(measurements.map((slot) => slot.label))}`
  ).toBe(7);

  // Assert 2（真实数字文本覆盖率）：防止探针降级路径掩盖选择器落空。
  //   轮次/模型调用/工具调用 + 输入/输出/缓存 这 6 个位点在首页必然渲染数字（无数据显示 "0"），
  //   只有「生成速度」可能为 "—"。因此真实数字文本位点数必须 ≥ 6。
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
