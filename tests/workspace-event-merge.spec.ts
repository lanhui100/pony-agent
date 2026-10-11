import { describe, expect, it } from "vitest";

function mergeContentEvents(lastContent: string, newContent: string): string {
  const sep = lastContent.endsWith("\n\n")
    ? ""
    : lastContent.endsWith("\n")
      ? "\n"
      : "\n\n";
  return lastContent + sep + newContent;
}

describe("Event content smart merging", () => {
  it("does not create redundant blank lines when merging content with trailing newline", () => {
    const part1 = "第一行回答\n";
    const part2 = "第二行回答";
    const merged = mergeContentEvents(part1, part2);
    expect(merged).toBe("第一行回答\n\n第二行回答");
    expect(merged).not.toContain("\n\n\n");
  });

  it("does not create extra newlines if already ending with double newline", () => {
    const part1 = "第一行回答\n\n";
    const part2 = "第二行回答";
    const merged = mergeContentEvents(part1, part2);
    expect(merged).toBe("第一行回答\n\n第二行回答");
  });

  it("adds double newline if ending without any newline", () => {
    const part1 = "第一行回答";
    const part2 = "第二行回答";
    const merged = mergeContentEvents(part1, part2);
    expect(merged).toBe("第一行回答\n\n第二行回答");
  });
});
