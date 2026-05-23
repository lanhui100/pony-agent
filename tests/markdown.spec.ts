import { describe, expect, it } from "vitest";
import { normalizeMarkdownSource, renderMarkdown } from "@/lib/markdown";

describe("markdown rendering", () => {
  it("renders headings, blockquotes and strong text", () => {
    const html = renderMarkdown(["#标题", "", ">引用", "", "**加粗**"].join("\n"));

    expect(html).toContain("<h1>标题</h1>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain("<strong>加粗</strong>");
  });

  it("unwraps an outer md fence without breaking inner code fences", () => {
    const source = [
      "下面是可直接保存为 `README.md` 的内容：",
      "",
      "```md",
      "# Pony Agent",
      "",
      "> 简介",
      "",
      "## 启动",
      "",
      "```bash",
      "npm run tauri dev",
      "```",
      "",
      "**完成**",
      "```"
    ].join("\n");

    const normalized = normalizeMarkdownSource(source);
    const html = renderMarkdown(source);

    expect(normalized).not.toContain("```md");
    expect(normalized).toContain("```bash");
    expect(html).toContain("<h1>Pony Agent</h1>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain("<strong>完成</strong>");
    expect(html).toContain("<pre><code class=\"language-bash\">npm run tauri dev");
  });

  it("unwraps a plain outer fence when the body is markdown", () => {
    const source = [
      "下面是整理后的内容：",
      "",
      "```",
      "# 标题",
      "",
      "> 引用",
      "",
      "- 列表项",
      "",
      "**重点**",
      "```"
    ].join("\n");

    const normalized = normalizeMarkdownSource(source);
    const html = renderMarkdown(source);

    expect(normalized).not.toContain("```");
    expect(html).toContain("<h1>标题</h1>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain("<strong>重点</strong>");
  });

  it("decodes escaped newlines before rendering markdown", () => {
    const source = "#标题\\n\\n>引用\\n\\n**加粗**";
    const normalized = normalizeMarkdownSource(source);
    const html = renderMarkdown(source);

    expect(normalized).toContain("# 标题");
    expect(html).toContain("<h1>标题</h1>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain("<strong>加粗</strong>");
  });

  it("renders json-stringified markdown content", () => {
    const source = JSON.stringify("```md\\n# 标题\\n\\n> 引用\\n\\n**加粗**\\n```");
    const normalized = normalizeMarkdownSource(source);
    const html = renderMarkdown(source);

    expect(normalized).not.toContain("```md");
    expect(html).toContain("<h1>标题</h1>");
    expect(html).toContain("<blockquote>");
    expect(html).toContain("<strong>加粗</strong>");
  });
});
