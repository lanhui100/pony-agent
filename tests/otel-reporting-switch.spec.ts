import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent } from "vue";
import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import ConfigGeneralSection from "@/components/config/ConfigGeneralSection.vue";
import TelemetryPage from "@/components/telemetry/TelemetryPage.vue";
import { useSettingsStore } from "@/stores/settings";
import { useRuntimeStore } from "@/stores/runtime";

/**
 * Wave43 UI-1 / UI-2 验收：全局轨迹上报开关 + 观测页文案去 PonySentry 化。
 *
 * 契约来源：
 * - `.dev-team/wave43-adjudication.md` A5（testid 冻结）：`config-agent-trace-reporting-switch` /
 *   `telemetry-upload-independent-note`；`config-agent-trace-privacy-notice` 已由 ADR 0050 裁决移除。
 * - `docs/decisions/0050-trace-reporting-switch-ui-simplification.md`（用户裁决 2026-10-27）：
 *   选项收敛为单行「全局轨迹上报」+ 右端 switch，去除注释（隐私告示）与子项（OTel/用量治理/成本预算入口）。
 * - ADR 0047 plan D7（缺省 `false`、落盘键 `agentTraceReporting`）；U3（手动上传不受全局开关约束）。
 *
 * mock 范式沿用 `tests/TelemetryPage.spec.ts` + `tests/ConfigGeneralSectionUpdate.spec.ts`
 * （`vi.hoisted` mock `@/lib/tauri` + `setActivePinia(createPinia())` + data-testid 断言），不自创基础设施。
 */

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

const TraceInspectorStub = defineComponent({
  template: '<div data-testid="trace-inspector-stub">trace-inspector</div>'
});

const ModelMonitorPageStub = defineComponent({
  template: '<div data-testid="model-monitor-stub">model-monitor</div>'
});

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" }
  });
}

/** 以对象形式读取 settings 上的新字段（实现落地前该字段尚不在 AppSettings 类型里）。 */
function settingsField(name: string): unknown {
  return (useSettingsStore().settings as unknown as Record<string, unknown>)[name];
}

describe("Wave43 Agent Trace 上报开关与观测页文案", () => {
  enableAutoUnmount(afterEach);

  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse({ tag_name: "v0.0.1", name: null }))
    );
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue("");
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  // ------------------------------------------------------------------ UI-1

  it("UI-1：开关渲染且默认关闭（aria-checked=false），仅单行「全局轨迹上报」标签，无注释与子项", () => {
    const wrapper = mount(ConfigGeneralSection);

    const toggle = wrapper.get('[data-testid="config-agent-trace-reporting-switch"]');
    expect(toggle.attributes("aria-checked")).toBe("false");
    expect(toggle.attributes("aria-label")).toBe("全局轨迹上报");
    // store 缺省值同源冻结（D7：DEFAULT_SETTINGS.agentTraceReporting === false）
    expect(settingsField("agentTraceReporting")).toBe(false);

    // 用户裁决（2026-10-27）：选项收敛为「全局轨迹上报」+ 右端 switch，旧称「全局 Agent Trace 上报」不再出现
    const section = wrapper.get('[data-testid="config-diagnostics-section"]');
    expect(section.text()).toContain("全局轨迹上报");
    expect(section.text()).not.toContain("Agent Trace");

    // 用户裁决（2026-10-27 二次）：不再有「诊断与遥测」分区标题，行本身无边框、无背景色
    expect(section.text()).not.toContain("诊断与遥测");
    const sectionClasses = section.classes();
    expect(sectionClasses.some((name) => name === "border" || name.startsWith("border-"))).toBe(false);
    expect(sectionClasses.some((name) => name.startsWith("bg-"))).toBe(false);
    // 标签与 switch 处于同一 justify-between 行（开关贴最右端）
    const row = section.get("div.flex");
    expect(row.classes()).toContain("justify-between");

    // 去除注释与子项：隐私告示、OTel 子配置、用量治理、成本预算入口一律不得残留
    const removedTestIds = [
      "config-agent-trace-privacy-notice",
      "config-otel-endpoint-input",
      "config-otel-service-name-input",
      "config-otel-sampling-ratio-input",
      "config-otel-token-input",
      "config-usage-policy-window",
      "config-usage-policy-hard-limit",
      "config-usage-policy-soft-ratio",
      "config-usage-policy-enforcement",
      "config-cost-budget-input",
    ];
    for (const testId of removedTestIds) {
      expect(wrapper.find(`[data-testid="${testId}"]`).exists()).toBe(false);
    }
  });

  it("UI-1：开启开关后调用 save_app_settings 且入参 agentTraceReporting 为 true", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "save_app_settings") {
        const payload = args as { settings?: unknown } | undefined;
        return payload?.settings ?? "";
      }
      return "";
    });

    const wrapper = mount(ConfigGeneralSection);
    const toggle = wrapper.get('[data-testid="config-agent-trace-reporting-switch"]');

    await toggle.trigger("click");
    await flushPromises();

    // 落盘断言：必须真的 invoke（只改本地 state 不落盘视为契约未实现）
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "save_app_settings",
      expect.objectContaining({
        settings: expect.objectContaining({ agentTraceReporting: true })
      })
    );
    // 开关状态随之上翻（末态回显）
    expect(
      wrapper.get('[data-testid="config-agent-trace-reporting-switch"]').attributes("aria-checked")
    ).toBe("true");
  });

  it("UI-1：loadSettings 必须透传 agentTraceReporting（防 normalizeAppSettings 丢字段导致开关回弹）", async () => {
    tauriMocks.mockSafeInvoke.mockResolvedValueOnce({
      workspaceMode: "coding",
      agentTraceReporting: true
    });

    const store = useSettingsStore();
    await store.loadSettings();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("load_app_settings");
    expect(store.settings).toMatchObject({ agentTraceReporting: true });
  });

  // ------------------------------------------------------------------ UI-2

  it("UI-2：全局开关关闭时手动上传仍可用，文案不含 PonySentry 且存在独立说明", async () => {
    // Arrange：全局开关关闭（U3：手动上传为逐次授权，不受全局开关约束）
    const settingsStore = useSettingsStore();
    Object.assign(settingsStore.settings, { agentTraceReporting: false });

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: "sess-x" });

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "upload_session_trace") {
        return 3;
      }
      return null;
    });

    const wrapper = mount(TelemetryPage, {
      attachTo: document.body,
      global: {
        stubs: {
          TraceInspector: TraceInspectorStub,
          ModelMonitorPage: ModelMonitorPageStub
        }
      }
    });
    await flushPromises();

    // Act
    await wrapper.get('[data-testid="telemetry-upload-trace"]').trigger("click");
    await flushPromises();

    // Assert 1：上传走通（若上传入口被包进全局开关，这里会是 0 次调用）
    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("upload_session_trace", {
      sessionId: "sess-x"
    });
    expect(wrapper.get('[data-testid="telemetry-upload-feedback"]').text()).toContain("3 轮");

    // Assert 2：全页（含属性，覆盖 aria-label/tooltip）不得残留 PonySentry，必须迁移到 Langfuse
    const rendered = wrapper.html();
    expect(rendered).not.toContain("PonySentry");
    expect(rendered).toContain("Langfuse");

    // Assert 3：存在「与全局开关互不影响」的独立说明容器
    const note = wrapper.get('[data-testid="telemetry-upload-independent-note"]');
    expect(note.text()).toContain("全局开关");
  });
});
