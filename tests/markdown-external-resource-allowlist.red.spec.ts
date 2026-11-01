/**
 * T2-4 红相：Markdown 消毒的「外部资源白名单」契约。
 *
 * 缺陷：`src/lib/markdown.ts` 的 `sanitizeMarkdownHtml` 把 `img` 放进 SAFE_TAGS、把 `src`
 * 放进 ALLOWED_ATTR，且注释声称的 `isSafeUrl()` 在模块里**根本不存在**（全仓无定义）。
 * 结果：模型输出（可被 prompt 注入 / 远端内容污染的不可信文本）里的
 * `<img src="https://attacker.tld/...">` 会被原样保留 —— 渲染即产生一次静默外连
 * （IP/UA 泄漏、像素追踪），违反 AGENTS.md「外部安全边界 Fail-Closed」。
 *
 * 断言口径（机器可判定，只看输出 DOM，不看实现）：
 *   - 输出中不得残留任何指向远程 http(s) 的 `img[src]`（剥离 src 或移除元素都算通过）；
 *   - 输出中 `a[href]` 若带 scheme，必须是 https + 白名单 host（白名单真值源与
 *     `src-tauri/src/platform.rs::ALLOWED_OPEN_HOSTS` 同源）；
 *   - 反向钉子：白名单内的 https 链接、root-relative / 锚点链接必须仍然保留（防一刀切）。
 */
import { describe, expect, it } from "vitest";
import { sanitizeMarkdownHtml } from "@/lib/markdown";

/** 与 `src-tauri/src/platform.rs::ALLOWED_OPEN_HOSTS` 保持同源的 host 白名单。 */
const ALLOWED_HOSTS = ["github.com", "api.github.com", "exa.ai", "s3.local.ponyjob.top"];

function parseBody(html: string): HTMLElement {
  const doc = new DOMParser().parseFromString(
    `<!doctype html><html><body>${html}</body></html>`,
    "text/html",
  );
  return doc.body;
}

function isRemoteUrl(url: string): boolean {
  return /^\s*https?:\/\//i.test(url);
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return "";
  }
}

function hasScheme(url: string): boolean {
  return /^\s*[a-z][a-z0-9+.-]*:/i.test(url);
}

function isAllowedHref(url: string): boolean {
  const trimmed = url.trim();
  if (!trimmed) {
    return true;
  }
  // 相对路径 / 锚点 / 站内路径：不产生外部网络动作
  if (!hasScheme(trimmed) || trimmed.startsWith("#")) {
    return true;
  }
  if (!/^https:\/\//i.test(trimmed)) {
    return false;
  }
  return ALLOWED_HOSTS.includes(hostOf(trimmed));
}

describe("sanitizeMarkdownHtml 外部资源白名单（T2-4 红相）", () => {
  it("远程 <img src=https://…> 不得保留可加载的远程地址", () => {
    const dirty =
      '<p>look</p><img src="https://attacker.example/pixel.png" alt="pillow" width="1" height="1">';
    const clean = sanitizeMarkdownHtml(dirty);
    const body = parseBody(clean);

    const remoteImages = Array.from(body.querySelectorAll("img"))
      .map((img) => img.getAttribute("src") ?? "")
      .filter((src) => isRemoteUrl(src));

    expect(
      remoteImages,
      `远程图片必须在消毒阶段被拒绝（剥离 src 或移除元素），实际输出：${clean}`,
    ).toEqual([]);
  });

  it("协议相对 //attacker 形式的图片同样不得保留", () => {
    const clean = sanitizeMarkdownHtml('<img src="//attacker.example/x.png" alt="x">');
    const sources = Array.from(parseBody(clean).querySelectorAll("img")).map(
      (img) => img.getAttribute("src") ?? "",
    );

    expect(
      sources.filter((src) => src.trim().startsWith("//")),
      `协议相对地址（//host/…）会随页面协议发起外连，必须被拒绝，实际输出：${clean}`,
    ).toEqual([]);
  });

  it("非白名单外链必须被拒绝（href 剥离或元素移除）", () => {
    const dirty =
      '<a href="https://evil.example/steal?token=1">click</a>' +
      '<a href="http://plain.example/x">http</a>' +
      '<a href="ftp://files.example/x">ftp</a>';
    const clean = sanitizeMarkdownHtml(dirty);
    const offenders = Array.from(parseBody(clean).querySelectorAll("a"))
      .map((anchor) => anchor.getAttribute("href") ?? "")
      .filter((href) => !isAllowedHref(href));

    expect(
      offenders,
      `非白名单链接（远程 img / 非白名单链接必须 fail-closed）不得保留 href，实际输出：${clean}`,
    ).toEqual([]);
  });

  it("反向钉子：白名单内 https 链接与站内相对链接必须保留", () => {
    const clean = sanitizeMarkdownHtml(
      '<a href="https://github.com/dm/pony-agent">repo</a>' +
        '<a href="https://api.github.com/repos/x/y">api</a>' +
        '<a href="/settings">relative</a>' +
        '<a href="#anchor">anchor</a>',
    );
    const hrefs = Array.from(parseBody(clean).querySelectorAll("a")).map(
      (anchor) => anchor.getAttribute("href") ?? "",
    );

    expect(hrefs, `白名单链接不得被误杀，实际输出：${clean}`).toEqual([
      "https://github.com/dm/pony-agent",
      "https://api.github.com/repos/x/y",
      "/settings",
      "#anchor",
    ]);
  });
});
