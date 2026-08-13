import { beforeEach, describe, expect, it, vi } from "vitest";
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

function mountComposer() {
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
      plugins: [createPinia()],
      stubs: {
        TooltipRoot: { template: "<div><slot /></div>" },
        TooltipTrigger: { template: "<div><slot /></div>" },
        TooltipPortal: { template: "<div><slot /></div>" },
        TooltipContent: { template: "<div><slot /></div>" }
      }
    }
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
  localStorage.clear();
  vi.clearAllMocks();
  tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
});

describe("WorkspaceComposer attachments", () => {
  it("renders the attach (paperclip) button", () => {
    const wrapper = mountComposer();
    expect(wrapper.find('[data-testid="workspace-attach-button"]').exists()).toBe(true);
  });

  it("opens the picker on click and renders a chip for a picked file", async () => {
    const file = new File(["# note"], "note.md", { type: "text/markdown" });
    tauriMocks.mockPickFiles.mockResolvedValue([file]);
    const wrapper = mountComposer();

    await wrapper.find('[data-testid="workspace-attach-button"]').trigger("click");

    expect(tauriMocks.mockPickFiles).toHaveBeenCalled();
    await vi.waitFor(() => {
      expect(wrapper.find('[data-testid="workspace-attachment-chip-0"]').exists()).toBe(true);
    });
    expect(wrapper.text()).toContain("note.md");
  });

  it("removes a pending attachment chip", async () => {
    const file = new File(["# note"], "note.md", { type: "text/markdown" });
    tauriMocks.mockPickFiles.mockResolvedValue([file]);
    const wrapper = mountComposer();

    await wrapper.find('[data-testid="workspace-attach-button"]').trigger("click");
    await vi.waitFor(() => {
      expect(wrapper.find('[data-testid="workspace-attachment-chip-0"]').exists()).toBe(true);
    });

    await wrapper.find('[data-testid="workspace-attachment-remove-0"]').trigger("click");
    expect(wrapper.find('[data-testid="workspace-attachment-chip-0"]').exists()).toBe(false);
  });

  it("shows a notice for unsupported files instead of a chip", async () => {
    const file = new File(["MZ"], "evil.exe", { type: "application/x-msdownload" });
    tauriMocks.mockPickFiles.mockResolvedValue([file]);
    const wrapper = mountComposer();

    await wrapper.find('[data-testid="workspace-attach-button"]').trigger("click");

    await vi.waitFor(() => {
      expect(wrapper.find('[data-testid="workspace-attach-notice"]').exists()).toBe(true);
    });
    expect(wrapper.find('[data-testid="workspace-attach-notice"]').text()).toContain("暂不支持");
  });

  it("enables the submit action when only attachments are pending", async () => {
    const file = new File(["# note"], "note.md", { type: "text/markdown" });
    tauriMocks.mockPickFiles.mockResolvedValue([file]);
    const wrapper = mountComposer();

    expect(
      wrapper.find('[data-testid="workspace-submit-action"]').attributes("disabled")
    ).toBeDefined();

    await wrapper.find('[data-testid="workspace-attach-button"]').trigger("click");
    await vi.waitFor(() => {
      expect(wrapper.find('[data-testid="workspace-attachment-chip-0"]').exists()).toBe(true);
    });

    expect(
      wrapper.find('[data-testid="workspace-submit-action"]').attributes("disabled")
    ).toBeUndefined();
  });
});
