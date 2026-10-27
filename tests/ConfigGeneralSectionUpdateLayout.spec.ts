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

  // ── C2：开关在 section 标题行最右端 ─────────────────────────────────────
  it("C2：section 标题行内 Switch 为最右端元素、标题在左", () => {
    const wrapper = mountSection();

    const row = wrapper.find('[data-testid="config-update-header-row"]');
    expect(row.exists(), "必须存在契约锚点 config-update-header-row").toBe(true);
    const rowEl = row.element as HTMLElement;

    const switchEl = wrapper.get('[data-testid="config-update-autocheck-switch"]').element as HTMLElement;
    expect(rowEl.contains(switchEl), "Switch 必须在 section 标题行内").toBe(true);

    // 最右端 = 行内不存在任何排在 Switch 之后、且不属于 Switch 自身的元素。
    const trailing = Array.from(rowEl.querySelectorAll("*")).filter(
      (el) => el !== switchEl && !switchEl.contains(el) && precedes(switchEl, el)
    );
    expect(
      trailing.map((el) => `${el.tagName.toLowerCase()}[${el.className}]`),
      "Switch 之后不得再有任何元素（最右端）"
    ).toEqual([]);

    // 标题文案在 Switch 左侧（DOM 序早于 Switch），且标题元素就是契约锚点。
    const titleEl = row.get('[data-testid="config-update-section-title"]').element as HTMLElement;
    expect(precedes(titleEl, switchEl), "标题必须排在 Switch 之前").toBe(true);
    expect(titleEl.textContent?.trim()).toBe("软件自动更新");
  });

  // ── C2b：卡片方框内不再有「自动检查更新」行 ──────────────────────────────
  it("C2b：方框内无「自动检查更新」文案、无自动检查行，section 内 Switch 恰为 1 个", () => {
    const wrapper = mountSection();

    const section = wrapper.get('[data-testid="config-update-section"]');
    expect(section.text(), "方框内不得再有「自动检查更新」文案").not.toContain("自动检查更新");
    expect(
      wrapper.find('[data-testid="config-update-autocheck-row"]').exists(),
      "旧的行级锚点 config-update-autocheck-row 必须消失"
    ).toBe(false);
    expect(
      section.findAll('[data-testid="config-update-autocheck-switch"]').length,
      "section 内 Switch 必须恰为 1 个（标题行那个）"
    ).toBe(1);
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

    // 反向断言：不得跑到 section 标题行（那里只有标题 + 开关）。
    const headerRow = wrapper.find('[data-testid="config-update-header-row"]');
    expect(headerRow.exists()).toBe(true);
    expect(headerRow.element.contains(lastChecked.element), "「上次检查」不得出现在 section 标题行").toBe(false);
  });

  // ── C9：安装入口收敛为版本行主按钮 ──────────────────────────────────────
  it("C9：方框内不存在独立「立即升级」按钮，版本行只有主按钮 1 个", () => {
    const wrapper = mountSection();

    expect(
      wrapper.find('[data-testid="config-update-install-button"]').exists(),
      "下方独立「立即升级」按钮必须被移除（契约 v3 C9）"
    ).toBe(false);

    const versionRow = wrapper.get('[data-testid="config-update-version-row"]');
    const buttons = versionRow.findAll("button");
    expect(
      buttons.map((b) => b.attributes("data-testid")),
      "版本行只允许存在 1 个主操作按钮"
    ).toEqual(["config-update-check-button"]);
  });

  it("C3b：无上次检查时间时不渲染「上次检查」，版本行仍在", () => {
    const wrapper = mountSection();

    expect(wrapper.find('[data-testid="config-update-version-row"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="config-update-last-checked"]').exists()).toBe(false);
  });
});
