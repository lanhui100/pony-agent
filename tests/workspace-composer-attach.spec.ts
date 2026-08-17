import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import WorkspaceComposer from "@/components/chat/WorkspaceComposer.vue";
import { useProviderStore } from "@/stores/providers";
import type { ProviderRegistry } from "@/types/provider";

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

function mountComposer(registry?: ProviderRegistry) {
  const pinia = createPinia();
  if (registry) {
    useProviderStore(pinia).$patch({ registry });
  }

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

function createInternalModelNameRegistry(): ProviderRegistry {
  return {
    selectedProviderId: "provider-acme",
    providers: [
      {
        id: "provider-acme",
        name: "Acme",
        protocol: "openai",
        baseUrl: "https://api.example.com/v1",
        authType: "auto",
        supportedProtocols: ["openai"],
        endpoints: [],
        apiKeyEnvVar: "ACME_API_KEY",
        apiKeyValue: "",
        apiKeyPresent: false,
        selectedModelId: "model-2d8a65a0-09f9-43fb-90a9-6f2f89de1f29",
        models: [
          {
            id: "model-2d8a65a0-09f9-43fb-90a9-6f2f89de1f29",
            name: "model-2d8a65a0-09f9-43fb-90a9-6f2f89de1f29",
            model: "gpt-5.4",
            protocol: "openai",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: null,
            reasoningBudgetTokens: null,
            capabilityPreset: "open-ai-reasoning",
            capabilities: {
              contextWindowTokens: 128000,
              supportsTools: true,
              supportsStreaming: true,
              supportsReasoning: true,
              supportsImageInput: false,
              supportsVideoInput: false,
              supportsAudioInput: false,
              supportsTextOutput: true,
              supportsImageOutput: false,
              supportsVideoOutput: false,
              supportsAudioOutput: false
            }
          },
          {
            id: "model-acme-default",
            name: "Claude Sonnet",
            model: "claude-sonnet-4-20250514",
            protocol: "openai",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: null,
            reasoningBudgetTokens: null,
            capabilityPreset: "open-ai-chat",
            capabilities: {
              contextWindowTokens: 200000,
              supportsTools: true,
              supportsStreaming: true,
              supportsReasoning: false,
              supportsImageInput: true,
              supportsVideoInput: false,
              supportsAudioInput: false,
              supportsTextOutput: true,
              supportsImageOutput: false,
              supportsVideoOutput: false,
              supportsAudioOutput: false
            }
          },
          {
            id: "model-legacy-default",
            name: "model-legacy-default",
            model: "deepseek-v4-flash",
            protocol: "openai",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: null,
            reasoningBudgetTokens: null,
            capabilityPreset: "deepseek-chat",
            capabilities: {
              contextWindowTokens: 128000,
              supportsTools: true,
              supportsStreaming: true,
              supportsReasoning: false,
              supportsImageInput: false,
              supportsVideoInput: false,
              supportsAudioInput: false,
              supportsTextOutput: true,
              supportsImageOutput: false,
              supportsVideoOutput: false,
              supportsAudioOutput: false
            }
          }
        ]
      }
    ]
  };
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

  it("only renders visible model names or actual model IDs in the model menu", async () => {
    const wrapper = mountComposer(createInternalModelNameRegistry());

    await wrapper.get('[data-testid="workspace-model-menu-trigger"]').trigger("click");
    await nextTick();
    await nextTick();

    const floatingMenu = document.body.textContent ?? "";
    expect(floatingMenu).toContain("gpt-5.4");
    expect(floatingMenu).toContain("Claude Sonnet");
    expect(floatingMenu).toContain("deepseek-v4-flash");
    expect(floatingMenu).not.toContain("model-2d8a65a0-09f9-43fb-90a9-6f2f89de1f29");
    expect(floatingMenu).not.toContain("model-acme-default");
    expect(floatingMenu).not.toContain("model-legacy-default");

    wrapper.unmount();
  });
});
