// F1 冒烟断言（design 硬约束 6）：dialog 插件 capability 最小权限面钉板。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("tauri capabilities（dialog 最小权限）", () => {
  const raw = readFileSync("src-tauri/capabilities/default.json", "utf-8");
  const parsed = JSON.parse(raw) as { permissions: string[] };

  it("授予 dialog:allow-open（目录选择器所需）", () => {
    expect(parsed.permissions).toContain("dialog:allow-open");
  });

  it("不授 dialog:default / allow-save / allow-message / allow-ask / allow-confirm", () => {
    for (const forbidden of [
      "dialog:default",
      "dialog:allow-save",
      "dialog:allow-message",
      "dialog:allow-ask",
      "dialog:allow-confirm"
    ]) {
      expect(parsed.permissions, `不应包含 ${forbidden}`).not.toContain(forbidden);
    }
  });

  it("gen/schemas/capabilities.json 已随 capability 变更再生且含 dialog 条目", () => {
    const gen = JSON.parse(
      readFileSync("src-tauri/gen/schemas/capabilities.json", "utf-8")
    ) as Record<string, { permissions?: string[] }>;
    const main = gen["main-capability"];
    expect(main?.permissions).toBeDefined();
    expect(
      (main?.permissions ?? []).some((p) => p.startsWith("dialog:")),
      "gen/schemas 应同步 dialog 权限"
    ).toBe(true);
  });
});
