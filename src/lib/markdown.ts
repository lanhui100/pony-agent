import { marked } from "marked";

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

const SAFE_GLOBAL_ATTRS = new Set(["aria-label", "aria-hidden", "role", "title"]);
const SAFE_TAG_ATTRS: Record<string, Set<string>> = {
  a: new Set(["href", "rel", "target", "title"]),
  code: new Set(["class"]),
  div: new Set(["class"]),
  img: new Set(["alt", "height", "loading", "src", "title", "width"]),
  input: new Set(["checked", "disabled", "type"]),
  pre: new Set(["class"]),
  td: new Set(["align", "colspan", "rowspan"]),
  th: new Set(["align", "colspan", "rowspan"])
};

function isSafeUrl(value: string) {
  const trimmed = value.trim();

  if (!trimmed) {
    return false;
  }

  if (
    trimmed.startsWith("#") ||
    trimmed.startsWith("/") ||
    trimmed.startsWith("./") ||
    trimmed.startsWith("../")
  ) {
    return true;
  }

  const lower = trimmed.toLowerCase();

  if (lower.startsWith("mailto:") || lower.startsWith("tel:")) {
    return true;
  }

  try {
    const parsed = new URL(trimmed, "https://pony-agent.local");
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

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
 */
function sanitizeMarkdownHtml(html: string): string {
  if (typeof document === "undefined") {
    return html;
  }

  // ── Phase 1: Strip script/style blocks entirely (tag + content) ──
  let clean = html.replace(
    /<script\b[^<>]*>[\s\S]*?<\/script\s*>/gi,
    "",
  );
  clean = clean.replace(
    /<style\b[^<>]*>[\s\S]*?<\/style\s*>/gi,
    "",
  );

  // ── Phase 2: Walk tags with a regex, sanitize in place ──
  // Matches: leading slash? | tag-name | optional attrs   | self-close?
  // Group:        (1)        |   (2)    |      (3)         |    (4)
  const TAG_RE = /<(\/?)([a-zA-Z]\w*)((?:\s[^>]*)?)\s*(\/?)>/g;

  let result = "";
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = TAG_RE.exec(clean)) !== null) {
    // Text content before this tag
    result += clean.slice(lastIndex, match.index);
    lastIndex = match.index + match[0].length;

    const closingSlash = match[1];
    const tagName = match[2].toLowerCase();
    const attrsStr = match[3];
    const selfClose = match[4];

    if (closingSlash) {
      // Closing tag — keep only if tag is safe
      if (SAFE_TAGS.has(tagName)) {
        result += `</${tagName}>`;
      }
      continue;
    }

    if (!SAFE_TAGS.has(tagName)) {
      // Unknown tag — skip (content between tags is preserved as text)
      continue;
    }

    // Safe tag — collect and sanitize attributes
    const safe: [string, string][] = [];
    if (attrsStr.trim()) {
      const allowed = SAFE_TAG_ATTRS[tagName];
      if (allowed) {
        const ATTR_RE = /(\w[\w-]*)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|(\S+)))?\s*/g;
        let attrMatch: RegExpExecArray | null;
        while ((attrMatch = ATTR_RE.exec(attrsStr)) !== null) {
          const name = attrMatch[1].toLowerCase();
          const value = attrMatch[2] ?? attrMatch[3] ?? attrMatch[4] ?? "";

          if (!allowed.has(name) && !SAFE_GLOBAL_ATTRS.has(name)) {
            continue;
          }
          if (tagName === "a" && name === "href" && !isSafeUrl(value)) {
            continue;
          }
          if (tagName === "img" && name === "src" && !isSafeUrl(value)) {
            continue;
          }
          if (tagName === "input" && name === "type" && value !== "checkbox") {
            continue;
          }
          if (tagName === "input" && name === "disabled") {
            continue; // added unconditionally below
          }
          safe.push([name, value]);
        }
      }
    }

    // Enforce security invariants
    if (tagName === "a" && safe.some(([n]) => n === "href")) {
      safe.push(["target", "_blank"]);
      safe.push(["rel", "noopener noreferrer"]);
    }
    if (tagName === "input") {
      safe.push(["disabled", ""]);
    }

    const attrStr = safe
      .map(([n, v]) => (v ? `${n}="${v.replace(/"/g, "&quot;")}"` : n))
      .join(" ");

    result += `<${tagName}${attrStr ? " " + attrStr : ""}${selfClose ? "/" : ""}>`;
  }

  // Trailing text after the last tag
  result += clean.slice(lastIndex);
  return result;
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

function wrapTablesInScrollableContainer(html: string): string {
  return html.replace(
    /<table([^>]*)>([\s\S]*?)<\/table>/g,
    '<div class="table-scroll-wrapper"><table$1>$2</table></div>'
  );
}

export async function renderMarkdown(content: string): Promise<string> {
  const normalizedContent = normalizeMarkdownSource(content);
  const html = await marked.parse(normalizedContent, {
    breaks: true,
    gfm: true,
    async: true
  }) as string;

  return wrapTablesInScrollableContainer(sanitizeMarkdownHtml(html));
}
