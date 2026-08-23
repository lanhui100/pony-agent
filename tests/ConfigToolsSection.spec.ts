import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent } from "vue";
import { mount } from "@vue/test-utils";
import ConfigToolsSection from "@/components/config/ConfigToolsSection.vue";
import { useRuntimeStore } from "@/stores/runtime";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
});

async function mountTools() {
  const wrapper = mount(ConfigToolsSection, {
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub
      }
    }
  });
  await Promise.resolve();
  return wrapper;
}

describe("ConfigToolsSection", () => {
  // ── PA-096：自 HomeSidebar.spec 迁移的工具目录用例（挂载目标改为配置页工具 tab）──

  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("工具目录优先展示中文短名并附带权限摘要", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      availableTools: [
        {
          name: "workspace_path_info",
          canonicalToolName: "路径信息",
          displayMetadata: {
            displayNameZh: "路径信息"
          },
          permissionFacts: {
            permissionScope: "workspace.read",
            approvalMode: "none",
            decisionSource: "runtime"
          },
          description: "读取当前路径的基础信息",
          inputSchema: {
            type: "object",
            properties: {
              path: { type: "string" }
            },
            required: ["path"]
          }
        }
      ]
    });

    const wrapper = await mountTools();

    const text = wrapper.text();
    expect(text).toContain("路径信息");
    expect(text).toContain("workspace.read");
    expect(text).toContain("none");
    expect(text).toContain("runtime");
    expect(text).not.toContain("workspace_path_info");
  });

  it("工具目录权限摘要在 approvalMode 缺失时回退到 requiresApproval，并展示来源", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      availableTools: [
        {
          name: "workspace_write_file",
          canonicalToolName: "Write",
          executionPrimitive: "workspace_write_file",
          kind: "write",
          exposure: "model-visible",
          displayMetadata: {
            displayNameZh: "写入"
          },
          permissionFacts: {
            requiresApproval: true,
            permissionScope: "workspace.write",
            decisionSource: "policy_engine"
          },
          description: "写入文件",
          inputSchema: {
            type: "object",
            properties: {
              path: { type: "string" }
            },
            required: ["path"]
          }
        }
      ]
    });

    const wrapper = await mountTools();

    const text = wrapper.text();
    expect(text).toContain("写入");
    expect(text).toContain("workspace.write");
    expect(text).toContain("required");
    expect(text).toContain("policy_engine");
  });

  it("无可用工具时展示空态文案", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ availableTools: [] });

    const wrapper = await mountTools();

    expect(wrapper.get('[data-testid="config-tools-empty"]').text()).toContain("暂无可用工具");
  });
});
