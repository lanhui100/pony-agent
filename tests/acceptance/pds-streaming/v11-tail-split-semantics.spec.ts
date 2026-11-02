/**
 * wave-pds-3 · **V11：尾巴切分语义**（契约 v5 / R-A1、R-A2、R-A3）—— jsdom
 *
 * 为什么需要这一组（v5 双路审查发现的最大缺陷）：
 *   v1–v4 的夹具用的是**纯文本尾巴**（`MD_TAIL_TEXTS` 6–29 字），于是「按最后一个 `\n` 切分」
 *   这个错误实现在纯文本下与正确实现无法区分 —— 夹具给了绿，不是实现对。
 *   本组一律用**含 markdown 语法的尾巴**驱动**真渲染**（`renderPartialMarkdown` 路径），
 *   断言「已定稿区 + 尾巴」拼接后的**整体语义**，而不是只看尾巴节点本身。
 *
 * 判据来源（R-A1）：必须复用 `src/lib/markdown.ts` 的 `splitMarkdownBlocks`，尾巴 = **整个未闭合块**：
 *   - 表格：整体不得出现裸竖线可见文本；不得出现只有表头、`tbody` 无行的空表；
 *   - `text\n---`：必须渲染为 setext `<h2>`，不得是 `<hr>` + `<p>` 的拆分形态；
 *   - 行内加粗跨行：可见文本不得含字面 `**`；
 *   - 嵌套列表：不得塌成"并列两个 `<ul>`"；
 *   - 多行段落：不得裂成两个 `<p>`；
 *   - 栅栏闭合帧（R-A2）：不得出现第二个空 `<pre>`；尾巴必须为 0（或 `data-tail-len === 0`）。
 *     高度单调性在 jsdom 无法测（无布局）⇒ 由 V4（Chromium 逐帧 scrollHeight）覆盖，此处只锁结构等价。
 *
 * ⚠️ 契约 v7 变更（本文件的相应调整）：v7 收缩尾巴作用域 —— **非纯段落一律不发尾巴**，
 *   并删除「尾巴内嵌 `<pre><code>`」整条路径（原 V11-6a 已随之删除：代码块流式期不再有尾巴，
 *   改由 **V12** 覆盖"无尾巴 + 无字面围栏 + 无行丢失"）。
 *   本文件保留的语义断言（表格/setext/加粗/嵌套列表/多行段落/栅栏闭合）在 v7 下依然成立：
 *   非纯段落走 stableHtml 整块渲染，整体语义同样必须正确。
 *
 * **判据自检（V11-0）**：本组每个检测器都会先对「按最后一个 `\n` 切分」的缺陷实现（用同一套
 * `splitMarkdownBlocks` + `renderPartialMarkdown` 复现 v5 描述的症状）跑一遍，**必须报错**。
 * 这样"实现被改好了 ⇒ 用例变绿"与"判据本身不灵 ⇒ 假绿"就被区分开了 —— 这正是 v1–v4 的教训。
 *
 * 环境能力边界：jsdom 无布局 ⇒ `clientWidth === 0`，实现走「字数兜底」分支。因此本组只覆盖
 * 「未闭合块内多行」被错误切分的路径；「单行宽度钳制 / 按标点前推」两条分支在 jsdom 侧**零覆盖**，
 * 由 V4/V6（Chromium）间接兜底，已登记在 RECEIPT 的覆盖缺口表里。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { renderMarkdown, renderPartialMarkdown, splitMarkdownBlocks } from "@/lib/markdown";
import {
  TAIL_LEN_ATTR,
  TAIL_SELECTOR,
  contentColumn,
  installDomStubs,
  mountWorkspace,
  patchAssistant,
  setTailSwitch,
  settleRender
} from "./pds-streaming-harness";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

// ───────────────────────────── 判据（检测器） ─────────────────────────────

/** 归一化可见文本：折叠所有空白，用于判断"语义是否被切坏"。 */
function visibleText(element: Element | null): string {
  return (element?.textContent ?? "").replace(/\s+/g, "");
}

/**
 * 表格语义检测器。
 * 判据：① 可见文本不得含裸竖线；② 必须存在 <table>；③ 不得有"只有表头、tbody 无行"的空表；
 *       ④ 表格内必须包含全部单元格文本（a / 1 / 2）。
 */
function detectTableViolations(root: Element): string[] {
  const violations: string[] = [];
  const text = visibleText(root);
  if (text.includes("|")) {
    violations.push(`可见文本含裸竖线：${JSON.stringify((root.textContent ?? "").slice(0, 160))}`);
  }
  const tables = Array.from(root.querySelectorAll("table"));
  if (tables.length === 0) {
    violations.push("整体没有 <table>（表格被拆散成文本）");
    return violations;
  }
  for (const table of tables) {
    const bodyRows = table.querySelectorAll("tbody tr").length;
    const headerCells = table.querySelectorAll("thead th").length;
    if (bodyRows === 0 && headerCells > 0) {
      violations.push("存在只有表头、tbody 无行的空表（已定稿区拿到了半张表）");
    }
  }
  const tableText = tables.map((table) => visibleText(table)).join("|");
  for (const marker of ["a", "1", "2"]) {
    if (!tableText.includes(marker)) {
      violations.push(`表格缺少单元格文本 ${JSON.stringify(marker)}：${JSON.stringify(tableText)}`);
    }
  }
  return violations;
}

/** setext 标题检测器：必须 <h2>，不得 <hr>，不得把标题行当 <p>。 */
function detectSetextViolations(root: Element): string[] {
  const violations: string[] = [];
  const target = "这是一个setext标题";
  const hrs = root.querySelectorAll("hr").length;
  if (hrs > 0) violations.push(`出现 <hr> ×${hrs}（\`---\` 被当成水平线单独成块）`);
  const headings = Array.from(root.querySelectorAll("h1, h2"));
  const matched = headings.filter((heading) => visibleText(heading).includes(target));
  if (matched.length !== 1) {
    violations.push(
      `setext 标题必须恰好渲染为 1 个 h1/h2；实际=${JSON.stringify(headings.map((h) => `${h.tagName}:${visibleText(h)}`))}`
    );
  } else if (matched[0]!.tagName.toLowerCase() !== "h2") {
    violations.push(`setext 标题必须是 <h2>，实际 <${matched[0]!.tagName.toLowerCase()}>`);
  }
  const asParagraph = Array.from(root.querySelectorAll("p"))
    .filter((paragraph) => visibleText(paragraph) === target);
  if (asParagraph.length > 0) violations.push("标题行被渲染成 <p>（<hr> + <p> 拆分形态的一半）");
  return violations;
}

/** 行内加粗跨行检测器：不得有字面 `**`；必须形成内容为「加粗文字」的 <strong>。 */
function detectBoldViolations(root: Element): string[] {
  const violations: string[] = [];
  if ((root.textContent ?? "").includes("**")) {
    violations.push(`可见文本含字面 \`**\`：${JSON.stringify((root.textContent ?? "").slice(0, 160))}`);
  }
  const strongTexts = Array.from(root.querySelectorAll("strong")).map((strong) => visibleText(strong));
  if (!strongTexts.includes("加粗文字")) {
    violations.push(`必须形成跨行 <strong>加粗文字</strong>；实际 strong=${JSON.stringify(strongTexts)}`);
  }
  if (!visibleText(root).includes("继续")) violations.push("加粗后的「继续」不可见");
  return violations;
}

/** 嵌套列表检测器：顶层列表只能 1 个；必须存在嵌套 ul；两个二级项都在嵌套列表内。 */
function detectNestedListViolations(root: Element): string[] {
  const violations: string[] = [];
  const lists = Array.from(root.querySelectorAll("ul, ol"));
  const topLevel = lists.filter((list) => !list.parentElement?.closest("ul, ol"));
  if (topLevel.length !== 1) {
    violations.push(
      `顶层列表必须只有 1 个（塌缩症状=并列两个）；实际 ${topLevel.length} 个：`
        + JSON.stringify(topLevel.map((list) => list.outerHTML.slice(0, 120)))
    );
  }
  const nested = Array.from(root.querySelectorAll("ul ul"));
  if (nested.length !== 1) {
    violations.push(`必须存在恰好 1 个嵌套 <ul>；实际 ${nested.length} 个`);
  } else {
    const nestedTexts = Array.from(nested[0]!.querySelectorAll("li")).map((item) => visibleText(item));
    if (JSON.stringify(nestedTexts) !== JSON.stringify(["二级项A", "二级项B"])) {
      violations.push(`嵌套列表项必须是 [二级项A, 二级项B]，实际 ${JSON.stringify(nestedTexts)}`);
    }
  }
  if (/(^|\n)?-\s*二级项/.test(root.textContent ?? "")) {
    violations.push("出现裸列表标记文本（`- 二级项`），列表被当作文本吐出");
  }
  return violations;
}

/** 多行段落检测器：两行必须在同一个 <p>，且段落总数必须为 2。 */
function detectParagraphViolations(root: Element): string[] {
  const violations: string[] = [];
  const paragraphs = Array.from(root.querySelectorAll("p"));
  const merged = paragraphs.filter(
    (paragraph) =>
      visibleText(paragraph).includes("同一段的第一行")
      && visibleText(paragraph).includes("第二行仍在同一段")
  );
  if (merged.length !== 1) {
    violations.push(
      `两行必须落在同一个 <p> 内；实际 <p> 文本=${JSON.stringify(paragraphs.map((p) => visibleText(p)))}`
    );
  }
  if (paragraphs.length !== 2) {
    violations.push(
      `段落数必须为 2（已定稿段落 + 含两行的同一段），实际 ${paragraphs.length}：`
        + JSON.stringify(paragraphs.map((p) => visibleText(p)))
    );
  }
  return violations;
}

/** 栅栏闭合检测器：只允许 1 个 <pre>；代码文本不得丢失；尾巴必须归零。 */
function detectFenceClosedViolations(root: Element): string[] {
  const violations: string[] = [];
  const pres = Array.from(root.querySelectorAll("pre"));
  if (pres.length !== 1) {
    violations.push(
      `<pre> 必须恰好 1 个（"空 <pre> 二次块"= 2 个）；实际 ${pres.length} 个：`
        + JSON.stringify(pres.map((pre) => pre.outerHTML.slice(0, 140)))
    );
  }
  if (pres.length >= 1 && !visibleText(pres[0] ?? null).includes("constx=1;")) {
    violations.push(`代码文本丢失：${JSON.stringify(visibleText(pres[0] ?? null))}`);
  }
  const tails = Array.from(root.querySelectorAll(TAIL_SELECTOR));
  const tailLens = tails.map((tail) => tail.getAttribute(TAIL_LEN_ATTR));
  if (!(tails.length === 0 || (tails.length === 1 && String(tailLens[0]) === "0"))) {
    violations.push(
      `栅栏闭合后尾巴必须归零（0 个或 ${TAIL_LEN_ATTR}="0"）；实际数量=${tails.length}、len=${JSON.stringify(tailLens)}`
    );
  }
  return violations;
}

// ───────────────────────────── 夹具 ─────────────────────────────

interface TailFixture {
  id: string;
  title: string;
  source: string;
  detect: (root: Element) => string[];
}

/**
 * 夹具共同前提：已定稿区必须含 markdown 语法（这里统一用 `**加粗**`），
 * 否则 `isSimpleTextContent` 会把整段判为纯文本、走 escape 快路径 —— 那样**根本测不到尾巴切分**。
 * 这是本组第一版踩过的坑：夹具给了绿，与实现无关。
 */
const FIXTURES: TailFixture[] = [
  {
    id: "V11-1",
    title: "表格未闭合尾 —— 整体必须是表格，不得裸竖线/空 tbody",
    source: "已定稿 **加粗** 段落。\n\n| a | b |\n| --- | --- |\n| 1 | 2",
    detect: detectTableViolations
  },
  {
    id: "V11-2",
    title: "setext 标题未闭合尾 —— 必须是 <h2>，不得 <hr> + <p> 拆分",
    source: "已定稿 **加粗** 段落。\n\n这是一个 setext 标题\n---",
    detect: detectSetextViolations
  },
  {
    id: "V11-3",
    title: "行内加粗跨行 —— 不得字面 `**`，必须形成 <strong>",
    source: "已定稿 **加粗** 段落。\n\n这是**加粗\n文字**继续",
    detect: detectBoldViolations
  },
  {
    id: "V11-4",
    title: "嵌套列表未闭合尾 —— 不得塌成并列两个 <ul>",
    source: "已定稿 **加粗** 段落。\n\n- 一级项\n  - 二级项 A\n  - 二级项 B",
    detect: detectNestedListViolations
  },
  {
    id: "V11-5",
    title: "多行段落未闭合尾 —— 不得裂成两个 <p>",
    source: "已定稿 **加粗** 段落。\n\n同一段的第一行\n第二行仍在同一段",
    detect: detectParagraphViolations
  },
  {
    id: "V11-6",
    title: "栅栏闭合帧 —— 不得第二个空 <pre>，尾巴必须归零",
    // 闭合行已到达、但块尚未被空行提交（R-A2 指的正是这一帧）
    source: "已定稿 **加粗** 段落。\n\n```ts\nconst x = 1;\n```",
    detect: detectFenceClosedViolations
  }
];

/** 归一化可见文本（折叠空白） */
function normalizeText(text: string | null | undefined): string {
  return (text ?? "").replace(/\s+/g, "");
}

/** 渲染一个**流式**正文源（生产路径），返回 `.markdown-body` 承载节点。 */
async function renderStreamingSource(source: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  await patchAssistant(store, source, "pending");
  const wrapper = mountWorkspace(pinia);
  await settleRender();
  const body = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
  return {
    wrapper,
    store,
    body,
    readBody: () => contentColumn(wrapper)?.querySelector(".markdown-body") ?? null
  };
}

/** 夹具前提自检：源必须真的以未闭合块收尾（否则测的不是尾巴）。 */
function assertUnclosedTail(source: string) {
  const { tail } = splitMarkdownBlocks(source);
  expect(
    tail.length,
    `V11 夹具自检：源必须以未闭合块收尾（splitMarkdownBlocks 的 tail 非空），实际 tail=${JSON.stringify(tail)}`
  ).toBeGreaterThan(0);
}

/**
 * 复现 v5 描述的**缺陷实现**：`tail` 按最后一个 `\n` 切成「已定稿前缀 + 尾行」，两段分别渲染。
 * 用于 V11-0 的判据自检 —— 这是"检测器是否真的能抓到已知缺陷"的唯一证据。
 */
async function renderBuggySplit(source: string): Promise<string> {
  const { committed, tail } = splitMarkdownBlocks(source);
  const lastBreak = tail.lastIndexOf("\n");
  const head = lastBreak >= 0 ? tail.slice(0, lastBreak + 1) : "";
  const lastLine = lastBreak >= 0 ? tail.slice(lastBreak + 1) : tail;
  const committedHtml = await Promise.all(committed.map((block) => renderMarkdown(block)));
  const headHtml = head ? await renderPartialMarkdown(head) : "";
  const tailHtml = lastLine ? await renderPartialMarkdown(lastLine) : "";
  return [
    ...committedHtml,
    headHtml,
    `<span data-stream-tail data-tail-len="${lastLine.length}">${tailHtml}</span>`
  ].filter(Boolean).join("\n");
}

describe("wave-pds-3 V11：尾巴切分语义（契约 v5 R-A1 / R-A2 / R-A3）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    window.localStorage.setItem("pony-agent.stream-render.release-chars", "10000");
    setTailSwitch(null);
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
    installDomStubs();
  });

  afterEach(() => {
    window.localStorage.clear();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("V11-0（判据自检）: 对「按最后一个 \\n 切分」的缺陷实现，本组全部检测器必须报错", async () => {
    // Arrange + Act：用同一套仓内渲染器复现 v5 描述的缺陷形态
    const missed: Array<{ fixture: string; html: string }> = [];

    for (const fixture of FIXTURES) {
      const buggyHtml = await renderBuggySplit(fixture.source);
      const host = document.createElement("div");
      host.innerHTML = buggyHtml;
      const violations = fixture.detect(host);
      if (violations.length === 0) {
        missed.push({ fixture: fixture.id, html: buggyHtml.slice(0, 300) });
      }
    }

    // Assert：任何一条检测器抓不到已知缺陷 ⇒ 该条判据是假绿，必须修判据
    expect(
      missed,
      `V11-0：以下检测器抓不到「按最后一个 \\n 切分」的缺陷（说明判据本身不灵，不是实现问题）：`
        + JSON.stringify(missed, null, 1)
    ).toHaveLength(0);
  });

  it.each(FIXTURES.map((fixture) => [fixture.id, fixture.title, fixture] as const))(
    "%s: %s",
    async (id, _title, fixture) => {
      // Arrange
      assertUnclosedTail(fixture.source);
      const { wrapper, body } = await renderStreamingSource(fixture.source);

      // Assert 自检：内容确实渲染到了 DOM（防"什么都没渲染"的假绿）
      expect(body, `${id} 自检：必须存在 .markdown-body 承载节点`).not.toBeNull();
      expect(
        visibleText(body).length,
        `${id} 自检：正文必须非空（body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 200))}）`
      ).toBeGreaterThan(0);

      // Assert（契约主体）：整体语义正确（已定稿区 + 尾巴拼接后不得出现 v5 列的任一症状）
      const violations = fixture.detect(body as Element);
      expect(
        violations,
        `${id}：尾巴切分破坏了整体 markdown 语义（契约 v5 R-A1/R-A2）；`
          + `违规：${JSON.stringify(violations, null, 1)}；源=${JSON.stringify(fixture.source)}；`
          + `body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 600))}`
      ).toHaveLength(0);

      wrapper.unmount();
    }
  );

  it("V11-7（v7 新增）: 纯段落尾巴（含行内加粗/行内代码/链接）必须发尾巴且语义正确", async () => {
    // 目的：防止 v7 之后把资格门实现成"一律不发尾巴"也能绿 —— 纯段落是本波**唯一**允许发尾巴的形态。
    // Arrange
    const source = "已定稿 **加粗** 段落。\n\n这是**加粗尾巴**与`行内代码`以及[链接](https://github.com/)的普通段落";
    const tailSource = splitMarkdownBlocks(source).tail;
    assertUnclosedTail(source);
    expect(
      tailSource,
      "V11-7 夹具自检：尾巴必须是纯段落（本用例只测纯段落尾巴）"
    ).toBe("这是**加粗尾巴**与`行内代码`以及[链接](https://github.com/)的普通段落");

    const { wrapper, body } = await renderStreamingSource(source);

    // Assert ①：必须发且仅发 1 个尾巴
    const tails = Array.from(body?.querySelectorAll(TAIL_SELECTOR) ?? []);
    expect(
      tails.length,
      `V11-7：纯段落尾巴必须发且仅发 1 个 [data-stream-tail]；实际 ${tails.length} 个；`
        + `body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 400))}`
    ).toBe(1);

    // Assert ②：`data-tail-len` == 源尾巴字符数（v5 R-A3 仍适用）
    expect(
      tails[0]!.getAttribute(TAIL_LEN_ATTR),
      `V11-7：data-tail-len 必须等于源尾巴字符数 ${tailSource.length}`
    ).toBe(String(tailSource.length));

    // Assert ③：行内标记必须被真正解析（不得字面外露）
    expect(normalizeText(body?.textContent), "V11-7：可见文本不得含字面 `**`").not.toContain("**");
    expect(normalizeText(body?.textContent), "V11-7：可见文本不得含行内代码反引号").not.toContain("`");
    expect(normalizeText(body?.textContent), "V11-7：可见文本不得含链接语法残片 `](`").not.toContain("](");
    expect(
      Array.from(body?.querySelectorAll("strong") ?? []).map((node) => normalizeText(node.textContent)),
      "V11-7：行内加粗必须渲染为 <strong>"
    ).toContain("加粗尾巴");
    expect(
      Array.from(body?.querySelectorAll("code") ?? []).map((node) => normalizeText(node.textContent)),
      "V11-7：行内代码必须渲染为 <code>"
    ).toContain("行内代码");
    expect(
      Array.from(body?.querySelectorAll("a") ?? []).map((node) => normalizeText(node.textContent)),
      "V11-7：链接必须渲染为 <a>（href 可能被安全策略剥离，但链接节点与文本必须保留）"
    ).toContain("链接");

    wrapper.unmount();
  });

  it("V11-6b: 栅栏闭合且块已提交（尾随空行）后 —— 结构等价且代码文本不回撤", async () => {
    // Arrange：先开栅栏，再提交（尾随空行使 splitMarkdownBlocks 把代码块并入已定稿区）
    const openSource = "已定稿 **加粗** 段落。\n\n```ts\nconst x = 1;";
    const committedSource = "已定稿 **加粗** 段落。\n\n```ts\nconst x = 1;\n```\n\n";
    expect(
      splitMarkdownBlocks(committedSource).tail.length,
      "V11-6b 夹具自检：提交帧的 tail 必须为空"
    ).toBe(0);

    const { wrapper, store, readBody } = await renderStreamingSource(openSource);
    const openCodeText = visibleText(readBody()?.querySelector("pre") ?? null);

    // Act
    await patchAssistant(store, committedSource, "pending");
    await settleRender();
    const closedBody = readBody();

    // Assert
    expect(closedBody, "V11-6b 自检：提交帧必须存在 .markdown-body").not.toBeNull();
    expect(
      detectFenceClosedViolations(closedBody as Element),
      "V11-6b：提交帧结构必须与闭合帧等价（不得残留空 <pre>/尾巴）"
    ).toHaveLength(0);
    expect(
      visibleText(closedBody?.querySelector("pre") ?? null),
      `V11-6b：代码块可见文本不得回撤（开=${JSON.stringify(openCodeText)}）`
    ).toBe(openCodeText);

    wrapper.unmount();
  });
});
