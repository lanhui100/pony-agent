import { ref } from "vue";
import { marked } from "marked";
import DOMPurify from "dompurify";

export const markdownRenderEpoch = ref(0);

if (import.meta.hot) {
  import.meta.hot.accept(() => {
    markdownRenderEpoch.value += 1;
  });
}

const SAFE_TAGS = new Set([
  "a",
  "blockquote",
  "br",
  "code",
  "del",
  "div",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "img",
  "input",
  "li",
  "ol",
  "p",
  "pre",
  "strong",
  "table",
  "tbody",
  "td",
  "th",
  "thead",
  "tr",
  "ul"
]);



/**
 * String-based HTML sanitizer.
 *
 * Strips unsafe tags and attributes without building a DOM tree, avoiding the
 * expensive innerHTML-parse → walk → serialize cycle of the previous implementation.
 *
 * Security model:
 *   - <script> / <style> removed entirely (tag + content)
 *   - Tags not in SAFE_TAGS are removed (content preserved — "unwrap")
 *   - Safe tags: only SAFE_TAG_ATTRS keys + SAFE_GLOBAL_ATTRS survive
 *   - href/src verified by isSafeUrl()
 *   - <a href="…"> gets target="_blank" rel="noopener noreferrer"
 *   - <input> gets disabled="" and only "checkbox" type survives
 *
 * Non-DOM early-return 契约（显式决策：直通）：
 *   - `typeof document === "undefined"` 时直接 `return html`（不过滤）。
 *   - 该路径仅测试/SSR 环境可达；生产 Tauri WebView 必有 document，
 *     MarkdownRenderer.vue 两处 v-html（:308/:322）消费的永远是已消毒输出。
 *   - 直通意味着非 DOM 环境下调用方不得把返回值当作"已消毒"使用；
 *     红队矩阵 tests/markdown-sanitize.redteam.spec.ts 锁定该契约（含直通断言）。
 */
if (typeof window !== "undefined") {
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "A" && node.hasAttribute("href")) {
      node.setAttribute("target", "_blank");
      node.setAttribute("rel", "noopener noreferrer");
    }
    if (node.tagName === "INPUT") {
      if (node.getAttribute("type") !== "checkbox") {
        node.remove();
      } else {
        node.setAttribute("disabled", "");
      }
    }
  });
}

export function sanitizeMarkdownHtml(html: string): string {
  if (typeof document === "undefined") {
    // 非 DOM 仅测试/SSR 路径：显式直通（见上契约），生产 WebView 必有 document。
    return html;
  }

  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: Array.from(SAFE_TAGS),
    ALLOWED_ATTR: [
      "href",
      "title",
      "target",
      "rel",
      "src",
      "alt",
      "width",
      "height",
      "align",
      "colspan",
      "rowspan",
      "type",
      "checked",
      "disabled",
      "open",
      "class",
    ],
    ALLOW_DATA_ATTR: false,
    ADD_ATTR: ["target", "rel"],
  });
}

function normalizeMarkdownLine(line: string) {
  if (/^\s{0,3}(#{1,6})([^\s#].*)$/.test(line)) {
    return line.replace(/^(\s{0,3}#{1,6})([^\s#].*)$/, "$1 $2");
  }

  if (/^\s{0,3}(>+)([^\s>].*)$/.test(line)) {
    return line.replace(/^(\s{0,3}>+)([^\s>].*)$/, "$1 $2");
  }

  if (/^\s{0,3}\d+\.([^\s].*)$/.test(line)) {
    return line.replace(/^(\s{0,3}\d+\.)([^\s].*)$/, "$1 $2");
  }

  if (/^\s{0,3}[-+*]\s*\[[ xX]\]([^\s].*)$/.test(line)) {
    return line.replace(/^(\s{0,3}[-+*]\s*\[[ xX]\])([^\s].*)$/, "$1 $2");
  }

  if (/^\s{0,3}[-+*]([^\s*+-].*)$/.test(line) && !/^\s{0,3}([-+*])\1{2,}\s*$/.test(line)) {
    return line.replace(/^(\s{0,3}[-+*])([^\s*+-].*)$/, "$1 $2");
  }

  return line;
}

function escapeForRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function markdownSignalScore(content: string) {
  const signals = [
    /^\s{0,3}#{1,6}\s+\S/m,
    /^\s{0,3}#{1,6}\S/m,
    /^\s{0,3}>+\s+\S/m,
    /^\s{0,3}>+\S/m,
    /^\s{0,3}[-+*]\s+\S/m,
    /^\s{0,3}[-+*]\S/m,
    /^\s{0,3}\d+\.\s+\S/m,
    /^\s{0,3}\d+\.\S/m,
    /\*\*[^*\n]+\*\*/,
    /(?:^|\n)\|.+\|(?:\n|$)/,
    /`[^`\n]+`/,
    /^\s{0,3}[-+*]\s*\[[ xX]\]\s+\S/m
  ];

  return signals.reduce((score, pattern) => score + Number(pattern.test(content)), 0);
}

function looksLikeMarkdownDocument(content: string) {
  return markdownSignalScore(content) >= 2;
}

function maybeParseSerializedMarkdown(content: string) {
  const trimmed = content.trim();

  if (!trimmed) {
    return content;
  }

  const tryParse = (candidate: string) => {
    try {
      const parsed = JSON.parse(candidate);
      return typeof parsed === "string" ? parsed : null;
    } catch {
      return null;
    }
  };

  const directlyParsed = tryParse(trimmed);
  if (directlyParsed && looksLikeMarkdownDocument(directlyParsed)) {
    return directlyParsed;
  }

  if (!trimmed.startsWith("\"") && (trimmed.includes("\\n") || trimmed.includes("\\u") || trimmed.includes("\\\""))) {
    const escaped = trimmed
      .replace(/\\/g, "\\\\")
      .replace(/"/g, "\\\"");
    const reparsed = tryParse(`"${escaped}"`);

    if (reparsed && looksLikeMarkdownDocument(reparsed)) {
      return reparsed;
    }
  }

  return content;
}

function maybeDecodeEscapedMarkdown(content: string) {
  if (content.includes("\n") || !content.includes("\\n")) {
    return content;
  }

  const decoded = content
    .replace(/\\r\\n/g, "\n")
    .replace(/\\n/g, "\n")
    .replace(/\\t/g, "\t");

  return looksLikeMarkdownDocument(decoded) ? decoded : content;
}

function unwrapOuterMarkdownFence(content: string) {
  const decodedContent = maybeDecodeEscapedMarkdown(maybeParseSerializedMarkdown(content));
  const lines = decodedContent.split(/\r?\n/);
  const openIndex = lines.findIndex((line) => /^\s*([`~]{3,})(?:\s*([A-Za-z0-9_-]+))?\s*$/.test(line));

  if (openIndex === -1) {
    return decodedContent;
  }

  const openMatch = lines[openIndex].match(/^\s*([`~]{3,})(?:\s*([A-Za-z0-9_-]+))?\s*$/);

  if (!openMatch) {
    return decodedContent;
  }

  let lastNonEmptyIndex = lines.length - 1;

  while (lastNonEmptyIndex >= 0 && lines[lastNonEmptyIndex].trim().length === 0) {
    lastNonEmptyIndex -= 1;
  }

  if (lastNonEmptyIndex <= openIndex) {
    return decodedContent;
  }

  const fenceToken = openMatch[1];
  const fenceLanguage = openMatch[2]?.trim().toLowerCase() ?? "";
  const closingPattern = new RegExp(`^\\s*${escapeForRegExp(fenceToken[0])}{${fenceToken.length},}\\s*$`);

  if (!closingPattern.test(lines[lastNonEmptyIndex])) {
    return decodedContent;
  }

  const nestedMarkdownFenceExists = lines
    .slice(openIndex + 1, lastNonEmptyIndex)
    .some((line) => /^\s*([`~]{3,})\s*(md|markdown)\s*$/i.test(line));

  if (nestedMarkdownFenceExists) {
    return decodedContent;
  }

  const innerContent = lines.slice(openIndex + 1, lastNonEmptyIndex).join("\n");
  const canUnwrap =
    fenceLanguage === "md" ||
    fenceLanguage === "markdown" ||
    (!fenceLanguage && looksLikeMarkdownDocument(innerContent));

  if (!canUnwrap) {
    return decodedContent;
  }

  return [...lines.slice(0, openIndex), ...lines.slice(openIndex + 1, lastNonEmptyIndex), ...lines.slice(lastNonEmptyIndex + 1)].join("\n");
}

function unwrapInlineMarkdownFences(content: string) {
  return content.replace(
    /(^|\n)\s*```(?:md|markdown)\s*\r?\n([\s\S]*?)\r?\n\s*```(?=\s*(?:\n|$))/gi,
    (_, prefix: string, inner: string) => `${prefix}${inner.trim()}`
  );
}

export function normalizeMarkdownSource(content: string) {
  const unwrappedContent = unwrapInlineMarkdownFences(
    unwrapOuterMarkdownFence(maybeDecodeEscapedMarkdown(maybeParseSerializedMarkdown(content)))
  );
  const lines = unwrappedContent.split(/\r?\n/);
  let inFence = false;

  return lines
    .map((line) => {
      if (/^\s*(```|~~~)/.test(line)) {
        inFence = !inFence;
        return line;
      }

      if (inFence) {
        return line;
      }

      return normalizeMarkdownLine(line);
    })
    .join("\n");
}

export function endsWithNaturalBoundary(content: string): boolean {
  const trimmed = content.trimEnd();
  if (!trimmed) {
    return false;
  }

  const lastLine = trimmed.split(/\r?\n/).pop() ?? "";

  // Paragraph break: trailing blank line(s)
  if (trimmed.endsWith("\n\n")) {
    return true;
  }

  // Closing code fence (``` or ~~~)
  if (/^[`~]{3,}\s*$/.test(lastLine)) {
    return true;
  }

  // Closing of a blockquote section (blank line after blockquote)
  if (trimmed.endsWith("\n>") && lastLine.startsWith(">")) {
    return true;
  }

  // End of a table row followed by a blank line
  if (/^\|.+\|\s*$/.test(lastLine) && trimmed.endsWith("\n\n")) {
    return true;
  }

  // End of a horizontal rule
  if (/^\s{0,3}([-*_])\1{2,}\s*$/.test(lastLine)) {
    return true;
  }

  return false;
}

function wrapCodeBlocks(html: string): string {
  return html.replace(/<pre(\s[^>]*)?>/g, '<pre class="code-block-cream"$1>');
}

function wrapTablesInScrollableContainer(html: string): string {
  return html.replace(
    /<table([^>]*)>([\s\S]*?)<\/table>/g,
    '<div class="table-scroll-wrapper"><table$1>$2</table></div>'
  );
}

export async function renderMarkdown(content: string): Promise<string> {
  try {
    const normalizedContent = normalizeMarkdownSource(content);
    const html = await marked.parse(normalizedContent, {
      breaks: true,
      gfm: true,
      async: true
    }) as string;

    return wrapTablesInScrollableContainer(sanitizeMarkdownHtml(wrapCodeBlocks(html)));
  } catch (err) {
    console.error("[markdown] renderMarkdown failed:", err);
    return escapeHtml(content);
  }
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

const FENCE_LINE_RE = /^\s{0,3}(`{3,}|~{3,})(.*)$/;

function getUnclosedCodeFence(content: string): string | null {
  const lines = content.split(/\r?\n/);
  let openFence: string | null = null;

  for (const line of lines) {
    const match = line.match(FENCE_LINE_RE);
    if (!match) {
      continue;
    }

    const fence = match[1];
    const rest = match[2]?.trim() ?? "";

    if (openFence) {
      const sameMarker = fence[0] === openFence[0];
      if (sameMarker && fence.length >= openFence.length && !rest) {
        openFence = null;
      }
      continue;
    }

    openFence = fence;
  }

  return openFence;
}

export function isSimpleTextContent(content: string): boolean {
  if (!content) return true;
  if (content.includes("```") || content.includes("~~~")) return false;
  if (/\*\*|__/.test(content)) return false;
  if (/^#{1,6}(?:\s|\S)/m.test(content)) return false;
  if (/\[.+?\]\(.+?\)/.test(content)) return false;
  if (/^>\s/m.test(content)) return false;
  if (/^[-*+]\s/m.test(content)) return false;
  if (/^\d+\.\s/m.test(content)) return false;
  if (/\|.+\|/.test(content)) return false;
  return true;
}

export function countUnclosedCodeFences(content: string): number {
  return getUnclosedCodeFence(content) ? 1 : 0;
}

function collectTextOutsideFences(content: string): string {
  const lines = content.split(/\r?\n/);
  const outsideLines: string[] = [];
  let openFence: string | null = null;

  for (const line of lines) {
    const match = line.match(FENCE_LINE_RE);

    if (match) {
      const fence = match[1];
      const rest = match[2]?.trim() ?? "";

      if (openFence) {
        const sameMarker = fence[0] === openFence[0];
        if (sameMarker && fence.length >= openFence.length && !rest) {
          openFence = null;
        }
        continue;
      }

      openFence = fence;
      continue;
    }

    if (!openFence) {
      outsideLines.push(line);
    }
  }

  return outsideLines.join("\n");
}

function inlineMarkdownClosers(content: string): string {
  const outsideFences = collectTextOutsideFences(content);
  const stack: string[] = [];

  for (let i = 0; i < outsideFences.length; i++) {
    const char = outsideFences[i];
    if (char === "\\") {
      i++;
      continue;
    }

    const top = stack[stack.length - 1];
    if (char === "`") {
      if (outsideFences[i + 1] === "`") {
        continue;
      }
      if (top === "`") {
        stack.pop();
      } else {
        stack.push("`");
      }
      continue;
    }

    if (top === "`") {
      continue;
    }

    const marker = outsideFences.slice(i, i + 2);
    if (marker === "**" || marker === "__") {
      if (top === marker) {
        stack.pop();
      } else {
        stack.push(marker);
      }
      i++;
    }
  }

  return [...stack].reverse().join("");
}

export function autoCloseBoundaries(content: string): string {
  let completed = content;
  const unclosedFence = getUnclosedCodeFence(content);

  if (unclosedFence) {
    completed = completed.endsWith("\n") ? completed + unclosedFence : completed + "\n" + unclosedFence;
  }

  const inlineClosers = inlineMarkdownClosers(content);
  if (inlineClosers) {
    completed += inlineClosers;
  }

  return completed;
}

export async function renderPartialMarkdown(partialContent: string): Promise<string> {
  return renderMarkdown(autoCloseBoundaries(partialContent));
}
