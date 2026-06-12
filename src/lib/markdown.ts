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

function shouldKeepAttribute(tagName: string, attrName: string) {
  if (SAFE_GLOBAL_ATTRS.has(attrName)) {
    return true;
  }

  const tagAttrs = SAFE_TAG_ATTRS[tagName];
  return tagAttrs?.has(attrName) ?? false;
}

function sanitizeElementAttributes(element: Element) {
  const tagName = element.tagName.toLowerCase();

  for (const attr of Array.from(element.attributes)) {
    const attrName = attr.name.toLowerCase();

    if (!shouldKeepAttribute(tagName, attrName)) {
      element.removeAttribute(attr.name);
      continue;
    }

    if (tagName === "a" && attrName === "href" && !isSafeUrl(attr.value)) {
      element.removeAttribute(attr.name);
      continue;
    }

    if (tagName === "img" && attrName === "src" && !isSafeUrl(attr.value)) {
      element.removeAttribute(attr.name);
      continue;
    }

    if (tagName === "input" && attrName === "type" && attr.value !== "checkbox") {
      element.removeAttribute(attr.name);
    }
  }

  if (tagName === "a" && element.hasAttribute("href")) {
    element.setAttribute("target", "_blank");
    element.setAttribute("rel", "noopener noreferrer");
  }

  if (tagName === "input") {
    element.setAttribute("disabled", "");
  }
}

function sanitizeMarkdownHtml(html: string) {
  if (typeof document === "undefined") {
    return html;
  }

  const template = document.createElement("template");
  template.innerHTML = html;

  const visit = (node: ParentNode) => {
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType !== Node.ELEMENT_NODE) {
        continue;
      }

      const element = child as Element;
      const tagName = element.tagName.toLowerCase();

      if (!SAFE_TAGS.has(tagName)) {
        if (tagName === "script" || tagName === "style") {
          element.remove();
          continue;
        }

        while (element.firstChild) {
          element.parentNode?.insertBefore(element.firstChild, element);
        }

        element.remove();
        continue;
      }

      sanitizeElementAttributes(element);
      visit(element);
    }
  };

  visit(template.content);
  return template.innerHTML;
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

export function renderMarkdown(content: string) {
  const normalizedContent = normalizeMarkdownSource(content);
  const html = marked.parse(normalizedContent, {
    breaks: true,
    gfm: true
  }) as string;

  return wrapTablesInScrollableContainer(sanitizeMarkdownHtml(html));
}
