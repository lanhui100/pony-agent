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

export function renderMarkdown(content: string) {
  const html = marked.parse(content, {
    breaks: true,
    gfm: true
  }) as string;

  return sanitizeMarkdownHtml(html);
}
