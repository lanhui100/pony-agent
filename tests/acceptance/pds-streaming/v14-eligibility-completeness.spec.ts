/**
 * wave-pds-3 · **V14：资格门完备性**（契约 v9）—— jsdom
 *
 * v9 把资格判定从「8 条行正则黑名单」改为「**渲染结果自证**（render-then-verify）」：
 *   对候选尾巴源文本调 `renderPartialMarkdown`，若结果里出现**块级元素**
 *   （`h1–h6 / ul / ol / li / pre / table / blockquote / hr / div / section / figure` 等）
 *   即判为"非纯段落" ⇒ **不发尾巴**；允许至多一个顶层 `<p>` 与任意行内标签。
 *
 * 因此本组的**真判据是 ②**：`[data-stream-tail]` 内不得出现任何块级标签。
 * 只断言「tails = 0」抓不到 v9 说的漏网形态（把 `<pre>`/`<hr>`/`<h1>` 塞进尾巴 span）。
 *
 * 覆盖（全部已知块级构造）：tab 缩进 / 四空格缩进 / `***` / `___` / 单 `-` setext /
 * `#` 与 `##` ATX 标题 / 四反引号栅栏 / CRLF / `$$` / HTML 块 / HTML 注释，
 * 外加**反向自检**（纯段落必须发尾巴，且尾巴内同样不得有块级标签）。
 *
 * ⚠️ 已知契约张力（`$$`）：v9 的 render-then-verify 语义下，`$$…$$` 经 `renderPartialMarkdown`
 *   产出的是 `<p>$$<br>x = y<br>$$</p>`（**没有块级元素**）⇒ 按 v9 它是**合格**的纯段落尾巴；
 *   但 v9 的 V14 清单把 `$$` 列为"必须 tails = 0"。两者不能同时成立。
 *   本组的处置：对 `$$` **只断言 ②（尾巴内无块级标签）+ 文本无丢失**，不硬断言 tails=0，
 *   把分歧显式登记给 Lead（见 RECEIPT §7.3），而不是替契约做选择。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { renderMarkdown, renderPartialMarkdown, splitMarkdownBlocks } from "@/lib/markdown";
import {
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

/** 契约 v9：判为"非纯段落"的块级标签集合。 */
const BLOCK_LEVEL_TAGS = [
  "h1", "h2", "h3", "h4", "h5", "h6",
  "ul", "ol", "li",
  "pre", "table", "thead", "tbody", "tr", "td", "th",
  "blockquote", "hr", "div", "section", "figure", "details", "dl", "dt", "dd",
  "form", "fieldset", "address", "aside", "article", "header", "footer", "nav", "main"
] as const;

/** 归一化可见文本（折叠空白）。 */
function normalize(text: string | null | undefined): string {
  return (text ?? "").replace(/\s+/g, "");
}

/** ② 真判据：任何 `[data-stream-tail]` 内部都不得出现块级标签。 */
function blockTagsInsideTails(root: Element | null): Array<{ tag: string; html: string }> {
  if (!root) return [];
  const violations: Array<{ tag: string; html: string }> = [];
  for (const tail of Array.from(root.querySelectorAll(TAIL_SELECTOR))) {
    for (const tag of BLOCK_LEVEL_TAGS) {
      const found = Array.from(tail.querySelectorAll(tag));
      for (const element of found) {
        violations.push({ tag, html: element.outerHTML.slice(0, 160) });
      }
    }
  }
  return violations;
}

type TailExpectation = "none" | "present" | "either";

interface EligibilityFixture {
  id: string;
  title: string;
  source: string;
  /** 期望 tails 数量：none = 0；present = 1；either = 契约分歧项（只断言 ② 与文本无损）。 */
  expectTail: TailExpectation;
  /**
   * 该夹具的「资格判定依据」——即 `renderPartialMarkdown(tail)` 的产物形态：
   * - `block`：产出块级标签（⇒ v9 判为不合格）
   * - `inline`：只产出 `<p>`/行内标签（⇒ 合格；`$$` 属此类）
   * - `empty`：产出空串（HTML 注释 ⇒ 无可渲染内容 ⇒ 自然无尾巴）
   * 用于 V14-0b 的**夹具有效性/同源自检**：渲染器一旦改行为，这里会先红。
   */
  renderExpect: "block" | "inline" | "empty";
}

/** 所有夹具的共同前缀：带 markdown 标记，确保真的进入 MarkdownRenderer（而不是纯文本快路径）。 */
const PREFIX = "已定稿 **加粗** 段落。\n\n";

const FIXTURES: EligibilityFixture[] = [
  {
    id: "V14-1",
    title: "tab 缩进代码（v9 实测漏网：尾巴内出现 <pre>）",
    source: `${PREFIX}\tconst z = 1;`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-2",
    title: "四空格缩进代码",
    source: `${PREFIX}    缩进代码行\n    第二行`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-3",
    title: "水平线 ***（v9 实测漏网：尾巴内出现 <hr>）",
    source: `${PREFIX}正文一行\n***`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-4",
    title: "水平线 ___（v9 实测漏网：尾巴内出现 <hr>）",
    source: `${PREFIX}正文一行\n___`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-5",
    title: "单 `-` setext 标题（v9 实测漏网：尾巴内出现 <h2>）",
    source: `${PREFIX}标题一行\n-`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-6",
    title: "ATX `# 标题`（v9 实测漏网：尾巴内出现 <h1>）",
    source: `${PREFIX}# 标题一行`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-7",
    title: "ATX `## 标题`",
    source: `${PREFIX}## 标题一行`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-8",
    title: "四反引号栅栏",
    source: `${PREFIX}\`\`\`\`\n代码一行`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-9",
    title: "CRLF 换行的列表",
    source: `${PREFIX}- 项目 A\r\n- 项目 B`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-10",
    title: "`$$` 数学围栏（契约分歧项：v9 渲染为 <p> ⇒ 按 render-verify 属合格）",
    source: `${PREFIX}$$\nx = y\n$$`,
    expectTail: "either",
    renderExpect: "inline",
  },
  {
    id: "V14-11",
    title: "HTML 块",
    source: `${PREFIX}<div>html 块</div>`,
    expectTail: "none",
    renderExpect: "block",
  },
  {
    id: "V14-12",
    title: "HTML 注释",
    source: `${PREFIX}<!-- 注释 -->`,
    expectTail: "none",
    renderExpect: "empty",
  }
];

async function mountStreaming(source: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useRuntimeStore();
  await patchAssistant(store, source, "pending");
  const wrapper = mountWorkspace(pinia);
  await settleRender();
  const body = contentColumn(wrapper)?.querySelector(".markdown-body") ?? null;
  return { wrapper, body };
}

describe("wave-pds-3 V14：资格门完备性（契约 v9 render-then-verify）", () => {
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
      // Arrange
      const { wrapper, body } = await mountStreaming(fixture.source);

      // Assert 自检：内容确实渲染到了 DOM，且确实走了 markdown 路径（前缀含 `**加粗**`）
      expect(body, `${id} 自检：必须存在 .markdown-body 承载节点`).not.toBeNull();
      expect(
        normalize(body?.textContent).length,
        `${id} 自检：正文必须非空（body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 200))}）`
      ).toBeGreaterThan(0);
      expect(
        body?.querySelector("strong"),
        `${id} 前置：前缀含 \`**加粗**\` ⇒ 必须渲染出 <strong>（证明走的是 markdown 路径而不是纯文本快路径）`
      ).not.toBeNull();

      const tails = Array.from(body?.querySelectorAll(TAIL_SELECTOR) ?? []);

      // Assert ①：按 v9 期望的尾巴数量
      if (fixture.expectTail === "none") {
        expect(
          tails.length,
          `${id}：块级构造不得发尾巴（v9 render-then-verify）；实际 ${tails.length} 个；`
            + `body.innerHTML=${JSON.stringify((body?.innerHTML ?? "").slice(0, 500))}`
        ).toBe(0);
      } else if (fixture.expectTail === "present") {
        expect(tails.length, `${id}：纯段落必须发且仅发 1 个尾巴`).toBe(1);
      } else {
        // either：契约分歧项。只要求"若发尾巴，尾巴内不得有块级标签"
        expect(
          tails.length,
          `${id}：尾巴数量只能是 0 或 1（不得多个）`
        ).toBeLessThanOrEqual(1);
      }

      // Assert ②（**真判据**）：尾巴 span 内不得出现任何块级标签
      expect(
        blockTagsInsideTails(body),
        `${id}：尾巴 span 内不得出现块级标签（这是 v9 的核心判据；只断言 tails=0 抓不到"span 里塞 pre"）`
      ).toHaveLength(0);

      // Assert ③：无内容丢失 —— 可见文本与完整 markdown 渲染对齐
      const expected = document.createElement("div");
      expected.innerHTML = await renderMarkdown(fixture.source);
      expect(
        normalize(body?.textContent),
        `${id}：可见文本必须与 renderMarkdown(全文) 对齐（防吞字/丢行）；`
          + `实际=${JSON.stringify(normalize(body?.textContent).slice(0, 200))}；`
          + `期望=${JSON.stringify(normalize(expected.textContent).slice(0, 200))}`
      ).toBe(normalize(expected.textContent));

      wrapper.unmount();
    }
  );

  it("V14-0b（判据同源/夹具有效性自检）: 每个夹具的资格依据必须与 renderPartialMarkdown 的产出一致", async () => {
    // v9 把资格判定改成「渲染结果自证」⇒ 判据与渲染器**同源**。
    // 这条自检的作用：一旦 renderPartialMarkdown 的行为变化（例如某个构造不再产出块级标签），
    // 对应夹具就失效了（它会变成"断言一个已经不存在的形态"），必须立刻被发现并重写夹具。
    const mismatched: Array<{ id: string; expect: string; html: string; hasBlock: boolean }> = [];

    for (const fixture of FIXTURES) {
      const { tail } = splitMarkdownBlocks(fixture.source);
      const html = await renderPartialMarkdown(tail);
      const host = document.createElement("div");
      host.innerHTML = html;
      const hasBlock = BLOCK_LEVEL_TAGS.some((tag) => host.querySelector(tag) !== null);
      const empty = normalize(html).length === 0;

      const actual = empty ? "empty" : hasBlock ? "block" : "inline";
      if (actual !== fixture.renderExpect) {
        mismatched.push({ id: fixture.id, expect: fixture.renderExpect, html: html.slice(0, 120), hasBlock });
      }
    }

    expect(
      mismatched,
      `V14-0b：以下夹具的"资格依据"与 renderPartialMarkdown 的实际产出不一致 —— 渲染器行为已变，`
        + `对应夹具必须重写（否则它们退化为"断言不存在的形态"）：${JSON.stringify(mismatched, null, 1)}`
    ).toHaveLength(0);
  });

  it("V14-0（反向自检）: 纯段落必须发尾巴，且尾巴内同样不得有块级标签", async () => {
    // 目的：防止"资格门实现成永不发尾巴"也能让上面 12 条全绿
    const source = `${PREFIX}这是一段**加粗**与\`代码\`的普通正文，正在逐字输出`;
    const { wrapper, body } = await mountStreaming(source);

    const tails = Array.from(body?.querySelectorAll(TAIL_SELECTOR) ?? []);
    expect(
      tails.length,
      "V14-0：纯段落必须发且仅发 1 个尾巴（否则 V14 的『全 0』可以靠永不发尾巴作弊通过）"
    ).toBe(1);

    // 尾巴内允许 `<p>` 与行内标签，但不得有块级标签
    expect(
      blockTagsInsideTails(body),
      "V14-0：纯段落尾巴内不得出现块级标签（允许 <p>/<strong>/<code> 等）"
    ).toHaveLength(0);

    // 尾巴文本必须可见且完整
    expect(
      normalize(tails[0]?.textContent).length,
      "V14-0：尾巴必须有可见文本"
    ).toBeGreaterThan(0);

    wrapper.unmount();
  });
});
