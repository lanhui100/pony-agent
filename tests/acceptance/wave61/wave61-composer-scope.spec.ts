/**
 * Wave-61 冻结验收（L2-T）— §1 L-1 的「离开方」半边：composer 内不得再存在上下文圆环
 *
 * 契约源（只读）：`.dev-team/contracts/wave-61-status-panel.md` §1 L-1：
 *   「`WorkspaceComposer.vue` 内**不得**再存在该圆环（DOM、`data-testid`、专属 computed 一并移除）」。
 * 对照半边（圆环必须出现在 `status-metrics-counts` 行内）见 `wave61-layout.spec.ts` L-1。
 *
 * 为什么需要源码层断言：
 *   「专属 computed 一并移除」是**死代码**约束，DOM 无法观测（未被模板引用的 computed 仍可存在）。
 *   故本文件额外对 `WorkspaceComposer.vue` 做符号级断言（`contextUsagePercentage` /
 *   `contextDebugString` / `workspace-context-usage-circle` / `stroke-dashoffset` 均已删除）。
 *   这不是「断言实现细节」，而是契约原文点名的**必须消失的符号集合**。
 *
 * 作者：test-agent（L2-T）。冻结后 executor 只读。
 */
import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn(),
  mockPickFiles: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

// 只 mock pickFiles（文件选择接缝），保留模块其余真实实现（store 依赖它们）。
vi.mock("@/lib/runtime/file-attachments", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/file-attachments")>();
  return { ...actual, pickFiles: tauriMocks.mockPickFiles };
});

const COMPOSER_RELATIVE_PATH = path.join("src", "components", "chat", "WorkspaceComposer.vue");

/** 契约 L-1 明列必须消失的符号（DOM testid 1 个 + 专属 computed 2 个 + 圆环专有属性 1 个）。 */
const REMOVED_SYMBOLS = [
  "workspace-context-usage-circle",
  "contextUsagePercentage",
  "contextDebugString",
  "stroke-dashoffset"
] as const;

/** 挂载真实 WorkspaceComposer（与 `tests/workspace-composer-attach.spec.ts` 的挂载口径同构）。 */
function mountComposer() {
  const pinia = createPinia();
  setActivePinia(pinia);
  return mount(WorkspaceComposer, {
    props: {
      showReasoningContent: false,
      canUndoLastTurn: false,
      undoShortcutLabel: "Ctrl+Z",
      handleComposerKeydown: () => {},
      handlePrimaryAction: () => {},
      handleUndoLastTurn: () => {},
      setComposerShellRef: () => {}
    },
    global: {
      plugins: [pinia],
      stubs: {
        TooltipRoot: { template: "<div><slot /></div>" },
        TooltipTrigger: { template: "<div><slot /></div>" },
        TooltipPortal: { template: "<div><slot /></div>" },
        TooltipContent: { template: "<div><slot /></div>" }
      }
    }
  });
}

describe("wave-61 §1 L-1：composer 圆环移除（迁移的离开方）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const pinia = createPinia();
    setActivePinia(pinia);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("L-1(离开方): WorkspaceComposer 内不存在圆环 DOM（testid / aria-label / dashoffset 三路取证）", () => {
    // Arrange
    const wrapper = mountComposer();

    // Assert（锚点自检：composer 真的挂出来了，否则「找不到圆环」是恒真的空断言）
    expect(
      wrapper.find('[data-testid="workspace-composer-shell"]').exists(),
      "L-1：composer 外壳必须已渲染（反向锚点，防止空树上的恒真断言）"
    ).toBe(true);
    expect(
      wrapper.find('[data-testid="workspace-composer-input"]').exists(),
      "L-1：composer 输入框必须已渲染（反向锚点）"
    ).toBe(true);

    // Assert（契约主体 1：旧 testid 彻底消失）
    expect(
      wrapper.findAll('[data-testid="workspace-context-usage-circle"]'),
      "L-1：composer 内不得再存在 [data-testid=\"workspace-context-usage-circle\"]"
    ).toHaveLength(0);

    // Assert（契约主体 2：圆环可及名消失）
    const ariaCarriers = Array.from(wrapper.element.querySelectorAll("[aria-label]")).filter(
      (element) => element.getAttribute("aria-label") === "上下文用量"
    );
    expect(
      ariaCarriers.map((element) => element.outerHTML.slice(0, 120)),
      "L-1：composer 内不得再存在 aria-label=\"上下文用量\" 的圆环载体"
    ).toEqual([]);

    // Assert（契约主体 3：圆环专有 SVG 属性消失）
    const dashOffsetCarriers = Array.from(wrapper.element.querySelectorAll("[stroke-dashoffset]"));
    expect(
      dashOffsetCarriers.map((element) => element.outerHTML.slice(0, 120)),
      "L-1：composer 内不得再存在 stroke-dashoffset 圆环（圆环已迁移至状态面板）"
    ).toEqual([]);

    // Assert（迁移去向的对照：圆环位 testid 同样不得出现在 composer 内）
    expect(
      wrapper.findAll('[data-testid="status-context-usage"]'),
      "L-1：圆环新位点 status-context-usage 不得出现在 composer 内（唯一位点在 HomeStatusPanel）"
    ).toHaveLength(0);

    wrapper.unmount();
  });

  it("L-1(离开方): WorkspaceComposer.vue 源码不再包含圆环 testid 与专属 computed", () => {
    // Arrange
    const composerPath = path.join(process.cwd(), COMPOSER_RELATIVE_PATH);
    expect(fs.existsSync(composerPath), `L-1：composer 源码必须存在 → ${composerPath}`).toBe(true);
    const source = fs.readFileSync(composerPath, "utf-8");

    // Assert（自检：文件确实是 composer 本体，防止读错文件后「找不到符号」恒真）
    expect(source, "L-1：源码自检 —— 文件必须包含 composer 输入框锚点").toContain(
      'data-testid="workspace-composer-input"'
    );

    // Assert（契约主体：契约 L-1 点名的符号集合必须全部消失）
    for (const symbol of REMOVED_SYMBOLS) {
      expect(
        source.includes(symbol),
        `L-1：WorkspaceComposer.vue 不得再包含符号 "${symbol}"（契约 L-1：DOM / data-testid / 专属 computed 一并移除）`
      ).toBe(false);
    }
  });
});
