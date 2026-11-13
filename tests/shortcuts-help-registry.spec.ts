/**
 * 快捷键清单真值源（`@/lib/chat/keyboard-shortcuts`）纯数据门禁。
 *
 * 覆盖口径：
 * - 清单结构完整性：每个 section 有非空 title 与非空 items，条目 keys/description 均非空。
 * - 无重复组合键（keys.join("+") 唯一），避免帮助面板出现两条相同快捷键。
 * - 必须覆盖 handler 真实绑定的全部组合键（Mod+G/H/Z/T/B、Alt+S、Alt+↑/↓、Esc+Esc、Enter、Shift+Enter）。
 * - 平台解析：Mod → ⌘ / Ctrl，其余 token 原样；isMacPlatform 判定。
 * - 「帮助面板列出的 Mod 组合键 ⊆ handler 实际处理的键」这一机器约束。
 */
import { describe, expect, it } from "vitest";
import {
  SHORTCUT_HANDLED_MODIFIER_KEYS,
  SHORTCUT_SECTIONS,
  isMacPlatform,
  resolveShortcutKey
} from "@/lib/chat/keyboard-shortcuts";

const REQUIRED_COMBOS = [
  "Mod+G",
  "Mod+H",
  "Mod+Z",
  "Mod+T",
  "Mod+B",
  "Alt+S",
  "Alt+↑",
  "Alt+↓",
  "Esc+Esc",
  "Enter",
  "Shift+Enter"
] as const;

function allItems() {
  return SHORTCUT_SECTIONS.flatMap((section) => section.items);
}

function allCombos(): string[] {
  return allItems().map((item) => item.keys.join("+"));
}

describe("SHORTCUT_SECTIONS：清单结构", () => {
  it("每个 section 都有非空 title 与非空 items", () => {
    expect(SHORTCUT_SECTIONS.length).toBeGreaterThan(0);
    for (const section of SHORTCUT_SECTIONS) {
      expect(section.title.trim().length).toBeGreaterThan(0);
      expect(section.items.length).toBeGreaterThan(0);
      for (const item of section.items) {
        expect(item.description.trim().length).toBeGreaterThan(0);
        expect(item.keys.length).toBeGreaterThan(0);
        for (const key of item.keys) {
          expect(key.trim().length).toBeGreaterThan(0);
        }
      }
    }
  });

  it("全部条目 keys 序列化后无重复", () => {
    const combos = allCombos();
    expect(new Set(combos).size).toBe(combos.length);
  });

  it("覆盖 handler 实际绑定的全部组合键", () => {
    const combos = new Set(allCombos());
    for (const required of REQUIRED_COMBOS) {
      expect(combos.has(required)).toBe(true);
    }
  });

  it("清单中的 Mod 组合键 ⊆ handler 实际处理的单字母键集合", () => {
    const modCombos = allItems()
      .filter((item) => item.keys.length === 2 && item.keys[0] === "Mod")
      .map((item) => item.keys[1].toLowerCase());
    expect(modCombos.length).toBeGreaterThan(0);
    for (const letter of modCombos) {
      expect(SHORTCUT_HANDLED_MODIFIER_KEYS.has(letter)).toBe(true);
    }
    expect(SHORTCUT_HANDLED_MODIFIER_KEYS.has("h")).toBe(true);
    expect(SHORTCUT_HANDLED_MODIFIER_KEYS.has("g")).toBe(true);
  });

  it("清单中不含平台硬编码修饰键（macOS 的 ⌘ 只能由 resolveShortcutKey 产生）", () => {
    const flatKeys = allItems().flatMap((item) => item.keys);
    expect(flatKeys).not.toContain("⌘");
    expect(flatKeys).not.toContain("Ctrl");
  });
});

describe("resolveShortcutKey：平台 token 解析", () => {
  it("Mod 在 macOS 解析为 ⌘，在其它平台解析为 Ctrl", () => {
    expect(resolveShortcutKey("Mod", true)).toBe("⌘");
    expect(resolveShortcutKey("Mod", false)).toBe("Ctrl");
  });

  it("非 Mod token 原样返回（与平台无关）", () => {
    expect(resolveShortcutKey("Alt", true)).toBe("Alt");
    expect(resolveShortcutKey("Alt", false)).toBe("Alt");
    expect(resolveShortcutKey("Shift", true)).toBe("Shift");
    expect(resolveShortcutKey("Esc", true)).toBe("Esc");
    expect(resolveShortcutKey("Enter", false)).toBe("Enter");
    expect(resolveShortcutKey("↑", false)).toBe("↑");
    expect(resolveShortcutKey("/goal", true)).toBe("/goal");
  });
});

describe("isMacPlatform：navigator.platform 判定", () => {
  it("Mac 系平台返回 true", () => {
    expect(isMacPlatform("MacIntel")).toBe(true);
    expect(isMacPlatform("iPad")).toBe(true);
  });

  it("非 Mac 平台与缺省值返回 false", () => {
    expect(isMacPlatform("Win32")).toBe(false);
    expect(isMacPlatform("Linux x86_64")).toBe(false);
    expect(isMacPlatform(undefined)).toBe(false);
    expect(isMacPlatform("")).toBe(false);
  });
});
