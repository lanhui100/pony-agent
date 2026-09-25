/**
 * T3-F2 消毒红队矩阵（jsdom 前提）。
 *
 * 对象：src/lib/markdown.ts sanitizeMarkdownHtml（现行手写正则消毒）。
 * Oracle：浏览器解析语义——输出经 DOMParser 重解析后断言，
 *   实体编码（&#106; 等）与 "/ " 分隔畸形（<img/src=…>）等直通原文的
 *   旁路会被浏览器解码/容错解析，因此字符串级断言不足够，必须走 DOM。
 *
 * 通过标准：重解析后无危险元素、无 on*、style、ping、srcset 等危险属性、
 *   href/src 不含 javascript:/data:/blob:/vbscript:（大小写/实体解码后口径）。
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { sanitizeMarkdownHtml } from "@/lib/markdown";

const DANGEROUS_ELEMENTS = [
  "script",
  "style",
  "iframe",
  "object",
  "embed",
  "form",
  "button",
  "base",
  "link",
  "meta",
  "frame",
  "frameset",
  "svg",
  "math",
  "video",
  "audio",
  "source",
  "track",
  "textarea",
  "select",
  "option",
  "noscript",
];

const DANGEROUS_URL_SCHEMES = ["javascript:", "data:", "blob:", "vbscript:"];

function parseOutput(html: string): Document {
  return new DOMParser().parseFromString(
    `<!doctype html><html><body>${html}</body></html>`,
    "text/html",
  );
}

/** 浏览器语义级断言：输出重解析后无可执行残留。 */
function expectNeutralized(html: string) {
  const doc = parseOutput(html);

  for (const tag of DANGEROUS_ELEMENTS) {
    expect(
      doc.body.querySelectorAll(tag).length,
      `危险元素 <${tag}> 残留于输出: ${html}`,
    ).toBe(0);
  }

  const elements = doc.body.querySelectorAll("*");
  elements.forEach((el) => {
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      expect(name.startsWith("on"), `事件属性 ${attr.name} 残留: ${html}`).toBe(false);
      expect(name === "style", `style 属性残留: ${html}`).toBe(false);
      expect(name === "ping", `ping 属性残留: ${html}`).toBe(false);
      expect(name === "srcset", `srcset 属性残留: ${html}`).toBe(false);
      expect(name === "poster", `poster 属性残留: ${html}`).toBe(false);
      expect(name === "formaction", `formaction 属性残留: ${html}`).toBe(false);
      expect(name === "background", `background 属性残留: ${html}`).toBe(false);
      expect(name === "id" || name === "name", `clobbering 属性 ${attr.name} 残留: ${html}`).toBe(false);

      if (name === "href" || name === "src" || name === "xlink:href") {
        const decoded = attr.value.trim().toLowerCase();
        for (const scheme of DANGEROUS_URL_SCHEMES) {
          expect(
            decoded.startsWith(scheme),
            `危险 URL scheme ${scheme} 残留于 ${attr.name}: ${html}`,
          ).toBe(false);
        }
      }
    }
  });
}

describe("markdown sanitize redteam", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  // ── 1. javascript:（含大小写/实体/空白混淆） ──
  it("strips plain javascript: href", () => {
    expectNeutralized(sanitizeMarkdownHtml('<a href="javascript:alert(1)">x</a>'));
  });

  it("strips mixed-case JaVaScRiPt: href", () => {
    expectNeutralized(sanitizeMarkdownHtml('<a href="JaVaScRiPt:alert(1)">x</a>'));
  });

  it("strips decimal-entity obfuscated href (&#106;)", () => {
    expectNeutralized(sanitizeMarkdownHtml('<a href="&#106;avascript:alert(1)">x</a>'));
  });

  it("strips hex-entity obfuscated href (&#x6A;)", () => {
    expectNeutralized(sanitizeMarkdownHtml('<a href="&#x6A;avascript:alert(1)">x</a>'));
  });

  it("strips tab/newline-obfuscated javascript: href", () => {
    expectNeutralized(sanitizeMarkdownHtml('<a href="java\tscript:alert(1)">x</a>'));
    expectNeutralized(sanitizeMarkdownHtml('<a href="java\nscript:alert(1)">x</a>'));
  });

  it("strips unquoted and single-quoted javascript: href", () => {
    expectNeutralized(sanitizeMarkdownHtml("<a href=javascript:alert(1)>x</a>"));
    expectNeutralized(sanitizeMarkdownHtml("<a href='javascript:alert(1)'>x</a>"));
  });

  // ── 2. onerror / onload（img / svg） ──
  it("strips onerror from img", () => {
    expectNeutralized(sanitizeMarkdownHtml('<img src="x" onerror="alert(1)">'));
  });

  it("strips unquoted onerror from img", () => {
    expectNeutralized(sanitizeMarkdownHtml("<img src=x onerror=alert(1)>"));
  });

  it("strips onload from svg-nested content", () => {
    expectNeutralized(sanitizeMarkdownHtml('<svg><g onload="alert(1)"><circle r="5"/></g></svg>'));
    expectNeutralized(sanitizeMarkdownHtml('<svg onload="alert(1)"></svg>'));
  });

  it("neutralizes slash-separated attrs (<img/src=x/onerror=…>)", () => {
    expectNeutralized(sanitizeMarkdownHtml("<img/src=x/onerror=alert(1)>"));
    expectNeutralized(sanitizeMarkdownHtml("<a/href=javascript:alert(1)>x</a>"));
  });

  // ── 3. style 属性与 expression ──
  it("strips style attribute and expression()", () => {
    expectNeutralized(
      sanitizeMarkdownHtml('<div style="x:expression(alert(1))">x</div>'),
    );
    expectNeutralized(
      sanitizeMarkdownHtml('<p style="background:url(javascript:alert(1))">x</p>'),
    );
  });

  it("removes style blocks entirely (tag + content)", () => {
    const out = sanitizeMarkdownHtml("<style>body{background:url(javascript:alert(1))}</style><p>x</p>");
    expect(out).not.toContain("expression");
    expectNeutralized(out);
  });

  // ── 4. form / button ──
  it("unwraps form and button without leaving attributes", () => {
    const out = sanitizeMarkdownHtml(
      '<form action="https://evil.test"><button formaction="javascript:alert(1)">go</button></form>',
    );
    expect(out).not.toContain("<form");
    expect(out).not.toContain("<button");
    expect(out).not.toContain("formaction");
    expectNeutralized(out);
  });

  // ── 5. iframe / embed / object ──
  it("removes iframe/embed/object", () => {
    const out = sanitizeMarkdownHtml(
      '<iframe src="javascript:alert(1)"></iframe><embed src="x"><object data="x"></object><p>ok</p>',
    );
    expect(out).not.toContain("<iframe");
    expect(out).not.toContain("<embed");
    expect(out).not.toContain("<object");
    expectNeutralized(out);
  });

  it("removes case-mixed iframe", () => {
    expectNeutralized(sanitizeMarkdownHtml("<IFRAME SRC=\"javascript:alert(1)\"></IFRAME>"));
  });

  // ── 6. a ping ──
  it("strips ping from anchor", () => {
    const out = sanitizeMarkdownHtml('<a href="https://example.com" ping="https://evil.test">x</a>');
    expect(out).not.toContain("ping");
    expectNeutralized(out);
  });

  // ── 7. base 劫持 ──
  it("removes base tag", () => {
    const out = sanitizeMarkdownHtml('<base href="https://evil.test/"><p>x</p>');
    expect(out).not.toContain("<base");
    expectNeutralized(out);
  });

  // ── 8. srcset / poster ──
  it("strips srcset and poster", () => {
    const out = sanitizeMarkdownHtml(
      '<img src="https://example.com/a.png" srcset="https://evil.test/b.png 2x"><video poster="https://evil.test/p.png"></video>',
    );
    expect(out).not.toContain("srcset");
    expect(out).not.toContain("poster");
    expect(out).not.toContain("<video");
    expectNeutralized(out);
  });

  // ── 9. data: / blob: src 变体 ──
  it("strips data: and blob: src", () => {
    expectNeutralized(sanitizeMarkdownHtml('<img src="data:text/html,<script>alert(1)</script>">'));
    expectNeutralized(sanitizeMarkdownHtml('<img src="DATA:text/html;base64,PHNjcmlwdD4=">'));
    expectNeutralized(sanitizeMarkdownHtml('<img src="blob:https://evil.test/uuid">'));
    expectNeutralized(sanitizeMarkdownHtml('<a href="blob:https://evil.test/uuid">x</a>'));
  });

  // ── 10. mXSS </noscript> 差分 ──
  it("leaves no noscript differential", () => {
    const out = sanitizeMarkdownHtml("</noscript><img src=x onerror=alert(1)><noscript>");
    expect(out).not.toContain("<noscript");
    expectNeutralized(out);
  });

  // ── 11. DOM clobbering（id / name） ──
  it("strips id and name attributes", () => {
    const out = sanitizeMarkdownHtml(
      '<div id="x" name="y"><a id="z" href="https://example.com">t</a><img id="w" src="https://example.com/a.png"></div>',
    );
    expect(out).not.toContain("id=");
    expect(out).not.toContain("name=");
    expectNeutralized(out);
  });

  // ── 12. svg / math 畸形嵌套 ──
  it("unwraps svg/math nesting without executable residue", () => {
    expectNeutralized(sanitizeMarkdownHtml("<svg><math><mi href=\"javascript:alert(1)\">x</mi></math></svg>"));
    expectNeutralized(sanitizeMarkdownHtml("<math><mtext href=\"javascript:alert(1)\">x</mtext></math>"));
    expectNeutralized(sanitizeMarkdownHtml("<svg><animate onbegin=\"alert(1)\" attributeName=\"x\"/>"));
    expectNeutralized(sanitizeMarkdownHtml("<svg><a href=\"javascript:alert(1)\">x</a></svg>"));
  });

  // ── 13. 嵌套畸形 / 大小写混淆标签 ──
  it("removes case-mixed script blocks", () => {
    const out = sanitizeMarkdownHtml("<ScRiPt>alert(1)</ScRiPt><p>x</p>");
    expect(out.toLowerCase()).not.toContain("<script");
    expectNeutralized(out);
  });

  it("neutralizes nested-malformed tags", () => {
    const out = sanitizeMarkdownHtml("<<script>script>alert(1)<</script>/script><p>x</p>");
    expect(out.toLowerCase()).not.toContain("<script");
    expectNeutralized(out);
    const out2 = sanitizeMarkdownHtml("<scr<script>ipt>alert(1)</scr</script>ipt>");
    expectNeutralized(out2);
  });

  it("strips event handlers on upper-case safe tags", () => {
    expectNeutralized(sanitizeMarkdownHtml('<IMG SRC="x" ONERROR="alert(1)">'));
    expectNeutralized(sanitizeMarkdownHtml('<DIV ONCLICK="alert(1)">x</DIV>'));
  });

  // ── 14. 不明标签 unwrap 不留属性 ──
  it("unwraps unknown tags without leaving attributes", () => {
    const out = sanitizeMarkdownHtml('<foo bar="baz" onclick="alert(1)">text</foo><marquee behavior="slide">m</marquee>');
    expect(out).not.toContain("<foo");
    expect(out).not.toContain("<marquee");
    expect(out).not.toContain("onclick");
    expect(out).toContain("text");
    expectNeutralized(out);
  });

  // ── 正常功能回归：安全内容不受损 ──
  it("keeps safe links, images and formatting", () => {
    const out = sanitizeMarkdownHtml(
      '<h1>t</h1><p><a href="https://example.com">link</a><img src="https://example.com/a.png" alt="a"><strong>b</strong><code>c</code></p>',
    );
    expect(out).toContain('<a href="https://example.com"');
    expect(out).toContain('target="_blank"');
    expect(out).toContain('<img src="https://example.com/a.png"');
    expect(out).toContain("<strong>b</strong>");
  });

  it("keeps checkbox input disabled", () => {
    const out = sanitizeMarkdownHtml('<input type="checkbox" checked>');
    expect(out).toContain('type="checkbox"');
    expect(out).toContain("disabled");
    expectNeutralized(out);
  });

  // ── 非 DOM 早返契约（显式直通）：仅测试/SSR 路径 ──
  it("passes through untouched when document is absent (non-DOM contract)", () => {
    vi.stubGlobal("document", undefined);
    const raw = '<img src="x" onerror="alert(1)">';
    expect(sanitizeMarkdownHtml(raw)).toBe(raw);
  });
});
