/**
 * Wave-52 冻结契约（L2-T 组件契约）：设置页「软件自动更新」卡片文案与布局。
 *
 * 契约源（只读）：`.dev-team/contract-matrix-wave52.md` §冻结契约 C1 / C2 / C3。
 * 作者：Lead（A 级测试兼职，遵循 test-expert L2 组件契约 + AAA + 零假测试律）。
 * 红相锚定后本文件冻结只读，任何修改需 Lead 批准。
 *
 * 为什么 C2/C3 在 jsdom 只判 DOM 序、不判像素：
 *   jsdom 不执行 Tailwind，class 断言只是契约代理；真实几何由
 *   `tests/e2e/wave52-update-card-layout.spec.ts` 在真实 Chromium 上判定。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import ConfigGeneralSection from "@/components/config/ConfigGeneralSection.vue";
import { useUpdateStore } from "@/stores/update";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

function mountSection() {
  return mount(ConfigGeneralSection);
}

/** DOM 序判定：`a` 是否严格早于 `b`（同文档树）。 */
function precedes(a: Element, b: Element): boolean {
  return Boolean(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
}

describe("ConfigGeneralSection 软件自动更新卡片布局契约（Wave-52）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ tag_name: "v0.0.1", name: null }), {
          status: 200,
          headers: { "content-type": "application/json" }
        })
      )
    );
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    tauriMocks.mockSafeInvoke.mockResolvedValue("");
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  // ── C1：标题文案 ────────────────────────────────────────────────────────
  it("C1：分区标题文本严格为「软件自动更新」", () => {
    const wrapper = mountSection();

    const title = wrapper.find('[data-testid="config-update-section-title"]');
    expect(title.exists(), "必须存在契约锚点 config-update-section-title").toBe(true);
    expect(title.text()).toBe("软件自动更新");

    // 反向断言：旧标题「软件更新」不得残留（「软件自动更新」不含该子串，故该断言有证伪力）。
    expect(wrapper.get('[data-testid="config-update-section"]').text()).not.toContain("软件更新");
  });

  // ── C2：开关行最右端 ────────────────────────────────────────────────────
  it("C2：自动检查行内 Switch 为最右端元素、文案在左", () => {
    const wrapper = mountSection();

    const row = wrapper.find('[data-testid="config-update-autocheck-row"]');
    expect(row.exists(), "必须存在契约锚点 config-update-autocheck-row").toBe(true);
    const rowEl = row.element as HTMLElement;

    const switchEl = wrapper.get('[data-testid="config-update-autocheck-switch"]').element as HTMLElement;
    expect(rowEl.contains(switchEl), "Switch 必须在自动检查行内").toBe(true);

    // 最右端 = 行内不存在任何排在 Switch 之后、且不属于 Switch 自身的元素。
    const trailing = Array.from(rowEl.querySelectorAll("*")).filter(
      (el) => el !== switchEl && !switchEl.contains(el) && precedes(switchEl, el)
    );
    expect(
      trailing.map((el) => `${el.tagName.toLowerCase()}[${el.className}]`),
      "Switch 之后不得再有任何元素（最右端）"
    ).toEqual([]);

    // 文案在 Switch 左侧（DOM 序早于 Switch）。
    const labelEl = Array.from(rowEl.querySelectorAll("*")).find(
      (el) => !switchEl.contains(el) && el.children.length === 0 && (el.textContent ?? "").trim() === "自动检查更新"
    );
    expect(labelEl, "自动检查行内必须有「自动检查更新」文案").toBeTruthy();
    expect(precedes(labelEl as Element, switchEl), "文案必须排在 Switch 之前").toBe(true);
  });

  // ── C3：「上次检查」紧随当前版本 ─────────────────────────────────────────
  it("C3：「上次检查」位于版本行内且在版本 chip 之后", () => {
    const store = useUpdateStore();
    store.lastCheckedAtMs = Date.now() - 60 * 60 * 1000;
    store.status = "unpublished";

    const wrapper = mountSection();

    const row = wrapper.find('[data-testid="config-update-version-row"]');
    expect(row.exists(), "必须存在契约锚点 config-update-version-row").toBe(true);

    const chip = row.get('[data-testid="config-update-current-version"]');
    const lastChecked = wrapper.get('[data-testid="config-update-last-checked"]');

    expect(row.element.contains(lastChecked.element), "「上次检查」必须落在版本行内").toBe(true);
    expect(precedes(chip.element, lastChecked.element), "「上次检查」必须排在版本 chip 之后").toBe(true);
    expect(lastChecked.text()).toContain("上次检查：");

    // 反向断言：不得仍留在自动检查行（旧布局的位置）。
    const autoRow = wrapper.find('[data-testid="config-update-autocheck-row"]');
    expect(autoRow.exists()).toBe(true);
    expect(autoRow.element.contains(lastChecked.element), "「上次检查」不得留在自动检查行").toBe(false);
  });

  it("C3b：无上次检查时间时不渲染「上次检查」，版本行仍在", () => {
    const wrapper = mountSection();

    expect(wrapper.find('[data-testid="config-update-version-row"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="config-update-last-checked"]').exists()).toBe(false);
  });
});
