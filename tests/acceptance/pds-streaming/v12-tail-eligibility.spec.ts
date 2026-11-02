/**
 * wave-pds-3 · **V12：尾巴资格门（tail eligibility gate）**（契约 v7）—— jsdom
 *
 * 契约 v7 裁定：**只有「纯段落」未闭合块才发尾巴**；栅栏 / 缩进代码 / 列表 / 引用 / 表格 /
 * HTML / setext 下划线 / `$$` 数学围栏一律**不发尾巴**，全额走 `stableHtml`（等同 wave-59）。
 * 同时删除「尾巴内嵌 `<pre><code>`」整条路径（N1/N3 就住在那里）。
 *
 * 本组对 8 类构造各造一条**流式输入**，每条同时断言三件事：
 *   ① `[data-stream-tail]` 数量 **= 0**（资格门生效，且 `data-tail-len` 也不得出现）；
 *   ② **无字面围栏泄漏**（`pre` 之外的可见文本不得含 ``` / ~~~）+ **无行丢失**
 *      （整体可见文本必须与 `renderMarkdown(全文)` 的渲染语义对齐）；
 *   ③ **done 帧无内容跳动**（同一输入的完成态文本 == 流式态文本）。
 *
 * 夹具有效性自检（防"造了个纯段落还断言没有尾巴"的假绿）：
 *   - 源必须以未闭合块收尾（`splitMarkdownBlocks().tail` 非空）；
 *   - 该未闭合块必须命中契约 v7 的"非纯段落"判据（8 条规则的独立实现）。
 *
 * **本组的实战战绩（重要）**：首次运行（2026-11-02 23:49）时 8 条构造里 **V12-3 红**，
 * 命中的正是审查者报的 **N3**：` ```md ` 未闭围栏被 `unwrapOuterMarkdownFence` 按 markdown 解析，
 * 代码内容 `# 标题` 变成 `<h2>` ⇒ 可见文本少了 `#`，与 `renderMarkdown(全文)` 不一致（done 帧会跳）。
 * 失败原文：`实际="已定稿段落。标题正文一行"；期望="已定稿段落。#标题正文一行"`。
 * 实现侧随后修复，现全绿 —— 这条记录用于证明本组判据**确实能抓到该缺陷**，而不是碰巧为绿。
 *
 * 环境限制：jsdom 无布局，但本组判据全部是 DOM/文本语义，jsdom 可判定；
 * 逐帧高度不跳由 V4 覆盖（本组的"done 帧无跳动"是文本语义等价，不是像素）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { renderMarkdown, splitMarkdownBlocks } from "@/lib/markdown";
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

/** 归一化可见文本（折叠全部空白），用于语义对齐与"行丢失"判定。 */
function normalize(text: string | null | undefined): string {
  return (text ?? "").replace(/\s+/g, "");
}

/** `pre` 之外的可见文本：代码块内部的 ``` / ~~~ 是代码内容，不算"字面围栏泄漏"。 */
function textOutsideCodeBlocks(root: Element): string {
  const clone = root.cloneNode(true) as Element;
  for (const pre of Array.from(clone.querySelectorAll("pre"))) pre.remove();
  return normalize(clone.textContent);
}

/**
 * 契约 v7 的「非纯段落」判据（独立实现，用于夹具有效性自检）。
 * 任意一行命中即取消尾巴资格。
 */
const NON_PARAGRAPH_RULES: Array<{ id: string; test: (lines: string[], index: number) => boolean }> = [
  { id: "fence", test: (lines, index) => /^\s*(```|~~~)/.test(lines[index]!) },
  { id: "indented-code", test: (lines, index) => /^ {4,}\S/.test(lines[index]!) },
  { id: "list-item", test: (lines, index) => /^\s*([-*+]|\d+[.)])\s/.test(lines[index]!) },
  { id: "blockquote", test: (lines, index) => /^\s*>/.test(lines[index]!) },
  {
    id: "table",
    test: (lines, index) =>
      /^\s*\|/.test(lines[index]!)
      || (/[|]/.test(lines[index]!) && /^\s*\|?[\s:-]*\|/.test(lines[index + 1] ?? ""))
  },
  { id: "html", test: (lines, index) => /^\s*<[A-Za-z!/]/.test(lines[index]!) },
  { id: "setext", test: (lines, index) => /^\s*(=+|-{3,})\s*$/.test(lines[index]!) },
  { id: "math", test: (lines, index) => /^\s*\$\$/.test(lines[index]!) }
];

function nonParagraphReasons(tail: string): string[] {
  const lines = tail.split(/\r?\n/);
  const reasons: string[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    for (const rule of NON_PARAGRAPH_RULES) {
      if (rule.test(lines, index)) {
        reasons.push(`${rule.id}@line${index}`);
      }
    }
  }
  return Array.from(new Set(reasons));
}

interface ConstructFixture {
  id: string;
  title: string;
  /** N 编号（对应 v7/v9 审查里暴露的缺陷原型）。 */
  prototype: string;
  source: string;
  /** 期望"代码内容里可能出现围栏字面量"时，允许 pre 内出现（②的判据只看 pre 之外）。 */
  extraChecks?: (root: Element) => string[];
  /** 跳过"非纯段落"夹具自检（仅用于契约分歧项，如 `$$`）。 */
  skipNonParagraphGuard?: boolean;
  /** v9 之后个别构造（如 `$$`）的尾巴数量待裁定：`either` = 0 或 1 都接受。 */
  expectTailCount?: "zero" | "either";
}

const FIXTURES: ConstructFixture[] = [
  {
    id: "V12-1",
    title: "未闭 ```ts 围栏，且围栏行不在尾巴首行（N1 原型）",
    prototype: "N1",
    source: "已定稿段落。\n\n第一行正文\n```ts\nconst x = 1;",
    // N1 的关键：尾巴首行是普通文本，围栏在中间 —— 这正是"整行丢失 + 屏上字面 ```"的地方
    extraChecks: (root) => {
      const violations: string[] = [];
      if (!normalize(root.textContent).includes("第一行正文")) violations.push("「第一行正文」丢失（N1 行丢失）");
      if (!normalize(root.textContent).includes("constx=1;")) violations.push("代码内容「const x = 1;」丢失");
      return violations;
    }
  },
  {
    id: "V12-2",
    title: "未闭 ~~~ 围栏",
    prototype: "fence-~~~",
    source: "已定稿段落。\n\n~~~\nconst y = 2;",
    extraChecks: (root) => {
      const violations: string[] = [];
      if (!normalize(root.textContent).includes("consty=2;")) violations.push("代码内容「const y = 2;」丢失");
      return violations;
    }
  },
  {
    id: "V12-3",
    title: "未闭 ```md 围栏（N3 原型：opener 被 unwrap 吃掉）",
    prototype: "N3",
    source: "已定稿段落。\n\n```md\n# 标题\n正文一行",
    extraChecks: (root) => {
      const violations: string[] = [];
      if (!normalize(root.textContent).includes("#标题")) violations.push("代码内容「# 标题」丢失（N3 opener 被吃）");
      if (!normalize(root.textContent).includes("正文一行")) violations.push("代码内容「正文一行」丢失");
      return violations;
    }
  },
  {
    id: "V12-4",
    title: "含 info string 的 ```js 出现在未闭围栏内部（N4 原型）",
    prototype: "N4",
    source: "已定稿段落。\n\n```ts\nconst a = 1;\n```js\nconst b = 2;",
    extraChecks: (root) => {
      const violations: string[] = [];
      const text = normalize(root.textContent);
      if (!text.includes("consta=1;")) violations.push("代码内容「const a = 1;」丢失");
      // N4 的核心：```js 必须被当作**代码内容**保留（而不是被当作闭合栅栏把后面截断）
      if (!text.includes("```js")) violations.push("「```js」这一行丢失（N4：info string 被当闭合栅栏吃掉）");
      if (!text.includes("constb=2;")) violations.push("代码内容「const b = 2;」丢失");
      return violations;
    }
  },
  {
    id: "V12-5",
    title: "缩进代码块",
    prototype: "indented-code",
    // ⚠️ v9 修正：原夹具 `已定稿段落。\n\n    缩进代码行…` 的 `isSimpleTextContent === true`
    // ⇒ 走纯文本快路径、**根本不进 MarkdownRenderer** ⇒ 是"路径未到"的假绿。
    // 现加 `**加粗**` 前缀强制走 markdown 路径。
    source: "已定稿 **加粗** 段落。\n\n    缩进代码行\n    第二行缩进",
    extraChecks: (root) => {
      const violations: string[] = [];
      const text = normalize(root.textContent);
      if (!text.includes("缩进代码行")) violations.push("缩进代码「缩进代码行」丢失");
      if (!text.includes("第二行缩进")) violations.push("缩进代码「第二行缩进」丢失");
      return violations;
    }
  },
  {
    id: "V12-6",
    title: "列表项（loose list 的 boundary 形态，N5 原型）",
    prototype: "N5",
    // N5 的形态是"松散列表被切成两块"。注意：`splitMarkdownBlocks` 遇空行必切块 ⇒
    // **一个 loose list 不可能整体落在一个未闭合块里**（见 RECEIPT §7.4），
    // 因此这里覆盖的是 N5 的**边界形态**：已定稿区一个 `<ul>`、尾巴是另一个列表项。
    source: "已定稿 **加粗** 段落。\n\n- 项目 A\n\n- 项目 B",
    extraChecks: (root) => {
      const violations: string[] = [];
      const text = normalize(root.textContent);
      if (!text.includes("项目A")) violations.push("列表项「项目 A」丢失");
      if (!text.includes("项目B")) violations.push("列表项「项目 B」丢失（N5：loose list 被切两块）");
      return violations;
    }
  },
  {
    id: "V12-7",
    title: "引用块",
    prototype: "blockquote",
    source: "已定稿段落。\n\n> 引用第一行\n> 引用第二行",
    extraChecks: (root) => {
      const violations: string[] = [];
      const text = normalize(root.textContent);
      if (!text.includes("引用第一行")) violations.push("引用「引用第一行」丢失");
      if (!text.includes("引用第二行")) violations.push("引用「引用第二行」丢失");
      return violations;
    }
  },
  {
    id: "V12-9",
    title: "rule 5b：段落中带 `|` + 紧随分隔行（渲染为表格）",
    prototype: "rule-5b",
    source: "已定稿 **加粗** 段落。\n\n正文 a | b\n| --- | --- |",
    extraChecks: (root) => {
      const violations: string[] = [];
      const text = normalize(root.textContent);
      if (!text.includes("正文a")) violations.push("首行文本「正文 a」丢失");
      if (!text.includes("b")) violations.push("表头「b」丢失");
      return violations;
    }
  },
  {
    id: "V12-10",
    title: "rule 6：HTML 块",
    prototype: "rule-6",
    source: "已定稿 **加粗** 段落。\n\n<div>html 块内容</div>",
    extraChecks: (root) => {
      const violations: string[] = [];
      if (!normalize(root.textContent).includes("html块内容")) violations.push("HTML 块文本丢失");
      return violations;
    }
  },
  {
    id: "V12-11",
    title: "rule 7：setext 下划线（`===` ⇒ h1）",
    prototype: "rule-7",
    source: "已定稿 **加粗** 段落。\n\n标题一行\n===",
    extraChecks: (root) => {
      const violations: string[] = [];
      if (!normalize(root.textContent).includes("标题一行")) violations.push("setext 标题文本丢失");
      return violations;
    }
  },
  {
    id: "V12-12",
    title: "rule 8：`$$` 数学围栏（⚠️ 契约分歧项，见 RECEIPT §7.3）",
    prototype: "rule-8",
    source: "已定稿 **加粗** 段落。\n\n$$\nx = y\n$$",
    // v9 的 render-then-verify 下 `$$` 渲染为 `<p>$$<br>x = y<br>$$</p>`（无块级）⇒ 属"合格尾巴"；
    // 但 v9 的 V14 清单要求 tails=0。两者冲突，故此处**不断言 tails 数量**，
    // 只断言"尾巴内无块级标签 + 文本不丢"（真正的判据），分歧交由 Lead 裁定。
    skipNonParagraphGuard: true,
    expectTailCount: "either" as const,
    extraChecks: (root) => {
      const violations: string[] = [];
      const text = normalize(root.textContent);
      if (!text.includes("$$")) violations.push("`$$` 首行丢失");
      if (!text.includes("x=y")) violations.push("`$$` 内容「x = y」丢失");
      return violations;
    }
  },
  {
    id: "V12-8",
    title: "表格行",
    prototype: "table",
    source: "已定稿段落。\n\n| a | b |\n| --- | --- |\n| 1 | 2",
    extraChecks: (root) => {
      const violations: string[] = [];
      const text = normalize(root.textContent);
      for (const marker of ["a", "b", "1", "2"]) {
        if (!text.includes(marker)) violations.push(`表格单元格「${marker}」丢失`);
      }
      return violations;
    }
  }
];

/** 渲染一个**流式**正文源（生产路径），返回内容区承载节点。 */
async function renderStreaming(source: string, status: "pending" | "done" = "pending") {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  await patchAssistant(store, source, status);
  const wrapper = mountWorkspace(pinia);
  await settleRender();
  const body = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
  return { wrapper, body, store };
}

/** 期望语义：把同一份源按**完整** markdown 渲染一遍（与流式路径无关的真值）。 */
async function expectedHtml(source: string): Promise<string> {
  return renderMarkdown(source);
}

describe("wave-pds-3 V12：尾巴资格门（契约 v7）", () => {
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

  it.each(FIXTURES.map((fixture) => [fixture.id, fixture.title, fixture] as const))(
    "%s: %s",
    async (id, _title, fixture) => {
      // ── Arrange：夹具有效性自检（防"造了纯段落还断言没尾巴"的假绿）──
      const { committed, tail } = splitMarkdownBlocks(fixture.source);
      expect(
        tail.length,
        `${id} 夹具自检：源必须以未闭合块收尾（splitMarkdownBlocks 的 tail 非空）`
      ).toBeGreaterThan(0);
      expect(
        committed.length,
        `${id} 夹具自检：必须存在已定稿块（用于区分"整块未定稿"与"尾巴"）`
      ).toBeGreaterThan(0);
      if (!fixture.skipNonParagraphGuard) {
        const reasons = nonParagraphReasons(tail);
        expect(
          reasons,
          `${id} 夹具自检：未闭合块必须命中"非纯段落"判据（否则本用例前提不成立）；tail=${JSON.stringify(tail)}`
        ).not.toHaveLength(0);
      }

      // ── Act：流式帧 ──
      const { wrapper, body, store } = await renderStreaming(fixture.source);

      // Assert 自检：内容确实渲染到了 DOM
      expect(body, `${id} 自检：必须存在 .markdown-body 承载节点`).not.toBeNull();
      expect(
        normalize(body?.textContent).length,
        `${id} 自检：正文必须非空（body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 200))}）`
      ).toBeGreaterThan(0);

      // ── Assert ①：非纯段落未闭合块 ⇒ 不发尾巴（`$$` 等分歧项允许 0 或 1，见接口注释）──
      const tails = Array.from(body?.querySelectorAll(TAIL_SELECTOR) ?? []);
      if (fixture.expectTailCount === "either") {
        expect(
          tails.length,
          `${id}（原型 ${fixture.prototype}）：尾巴数量只能是 0 或 1；实际 ${tails.length}`
        ).toBeLessThanOrEqual(1);
      } else {
        expect(
          tails.length,
          `${id}（原型 ${fixture.prototype}）：非纯段落未闭合块不得发尾巴（数量必须为 0）；`
            + `实际 ${tails.length} 个；tail 源=${JSON.stringify(tail)}；`
            + `body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 600))}`
        ).toBe(0);
      }
      if (fixture.expectTailCount !== "either") {
        expect(
          body?.querySelectorAll(`[${TAIL_LEN_ATTR}]`).length ?? 0,
          `${id}：不得出现任何 ${TAIL_LEN_ATTR} 属性节点（那是尾巴的第二判据）`
        ).toBe(0);
      }

      // ── Assert ①b（v9 真判据）：尾巴 span 内不得出现块级标签 ──
      const BLOCK_TAGS = ["h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "pre", "table", "blockquote", "hr", "div", "section", "figure"];
      const blockInside = tails.flatMap((tail) =>
        BLOCK_TAGS.flatMap((tag) =>
          Array.from(tail.querySelectorAll(tag)).map((element) => ({
            tag,
            html: element.outerHTML.slice(0, 120)
          }))
        )
      );
      expect(
        blockInside,
        `${id}：尾巴 span 内不得出现块级标签（v9 核心判据）；实际：${JSON.stringify(blockInside)}`
      ).toHaveLength(0);

      // ── Assert ②-a：`pre` 之外不得出现字面围栏标记 ──
      const outside = textOutsideCodeBlocks(body as Element);
      expect(
        outside,
        `${id}：pre 之外的可见文本不得含字面围栏标记（三个反引号 / ~~~）；实际=${JSON.stringify(outside.slice(0, 200))}`
      ).not.toMatch(/```|~~~/);

      // ── Assert ②-b：无行丢失 —— 与完整 markdown 渲染的语义对齐 ──
      const expected = document.createElement("div");
      expected.innerHTML = await expectedHtml(fixture.source);
      const actualText = normalize(body?.textContent);
      const expectedText = normalize(expected.textContent);
      expect(
        actualText,
        `${id}：流式渲染的可见文本必须与 renderMarkdown(全文) 对齐（行丢失/多吐字都会被这里抓到）；`
          + `实际=${JSON.stringify(actualText.slice(0, 300))}；期望=${JSON.stringify(expectedText.slice(0, 300))}`
      ).toBe(expectedText);

      // ── Assert ②-c：夹具自带的针对性检查（N1/N3/N4/N5 等原型）──
      if (fixture.extraChecks) {
        expect(
          fixture.extraChecks(body as Element),
          `${id}（原型 ${fixture.prototype}）：针对性检查未通过`
        ).toHaveLength(0);
      }

      // ── Assert ③：done 帧无内容跳动 ──
      await patchAssistant(store, fixture.source, "done");
      await settleRender();
      const doneBody = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
      expect(doneBody, `${id} 自检：done 帧必须存在 .markdown-body`).not.toBeNull();
      expect(
        normalize(doneBody?.textContent),
        `${id}：streaming→done 不得产生内容跳动（完成态文本必须与流式态一致）；`
          + `流式=${JSON.stringify(actualText.slice(0, 200))}；完成=${JSON.stringify(normalize(doneBody?.textContent).slice(0, 200))}`
      ).toBe(actualText);
      expect(
        doneBody?.querySelectorAll(TAIL_SELECTOR).length ?? 0,
        `${id}：done 帧同样不得残留尾巴节点`
      ).toBe(0);

      wrapper.unmount();
    }
  );

  it("V12-0（资格门边界自检）: 纯段落未闭合块必须【发】尾巴 —— 门不能变成「永不发尾巴」", async () => {
    // Arrange：一个纯段落尾巴（不含任何 v7 非纯段落特征）。
    // 注：这里刻意保留行内标记（`**加粗**` / 行内代码）。纯**无 markup** 的正文会走
    // WorkspaceTurnItem 的「纯文本快路径」，根本不进 MarkdownRenderer —— 那是另一条路径，
    // 已在 RECEIPT §7.2 作为待定问题登记（是否也该发尾巴，需契约明确）。
    const source = "已定稿段落。\n\n这是一段**加粗**与`代码`的普通正文，正在逐字输出";
    const { tail } = splitMarkdownBlocks(source);
    expect(tail.length, "V12-0 夹具自检：纯段落尾必须非空").toBeGreaterThan(0);
    expect(
      nonParagraphReasons(tail),
      `V12-0 夹具自检：纯段落尾不得命中任何"非纯段落"判据；tail=${JSON.stringify(tail)}`
    ).toHaveLength(0);

    // Act
    const { wrapper, body } = await renderStreaming(source);

    // Assert：纯段落必须发尾巴（否则 V12 的"全 0"可以靠"永不发尾巴"作弊通过）
    expect(
      body?.querySelectorAll(TAIL_SELECTOR).length ?? 0,
      "V12-0：纯段落未闭合块必须发且仅发 1 个尾巴；若这里也是 0，说明资格门被实现成"
        + "「一律不发尾巴」，整个尾巴特性就没了"
    ).toBe(1);
    expect(
      normalize(body?.textContent),
      "V12-0 自检：纯段落尾巴的文本必须可见"
    ).toContain("这是一段加粗与代码的普通正文，正在逐字输出".replace(/\s+/g, ""));

    wrapper.unmount();
  });
});
