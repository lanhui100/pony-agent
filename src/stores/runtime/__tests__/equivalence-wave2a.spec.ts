/**
 * Wave 2a · 等价测试补强（B277 / fingerprint `frontend::stores::runtime::test-coverage-gap`）
 *
 * 覆盖面：D8 健康与能力目录（6）· 用量治理（2）· 停止链（2）· 会话通知（1）· 待发送附件（3）。
 * 断言口径与既有 golden 场景不同：golden 冻结「事件走完后的全量快照」，本文件冻结
 * 「action 的宿主命令边界 + 状态转移」——两者互补，不重复覆盖同一判据。
 *
 * 确定性：假定时器 + 冻结时钟 + 假 IPC（FakeBackend）；无 sleep、无真实 IPC。
 */
import { describe, expect, it, vi } from "vitest";

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

vi.mock("@/lib/runtime/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/utils")>();
  return {
    ...actual,
    waitForNextPaint: () => Promise.resolve(),
    runLowPriorityTurnWork: (callback: () => void) => callback()
  };
});

const recorderMock = vi.hoisted(() => ({
  initFrontendFlightRecorder: vi.fn(),
  bindFrontendRecorderSession: vi.fn(),
  bindFrontendRecorderTurn: vi.fn(),
  recordFrontendInstant: vi.fn()
}));

vi.mock("@/lib/frontend-flight-recorder", () => recorderMock);

const settingsMock = vi.hoisted(() => ({
  state: {
    settings: {
      usageGovernancePolicy: { enforcement: true }
    },
    saveSettings: vi.fn(async () => undefined)
  }
}));

vi.mock("@/stores/settings", () => ({
  useSettingsStore: () => settingsMock.state
}));

// 附件字节读取走 FileReader（事件驱动），在假定时器下不会自行落定 ——
// 用例侧只替换「读字节」这一层，其余附件规则（类型/上限/去重/嗅探）保持真实实现。
vi.mock("@/lib/runtime/file-attachments", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/file-attachments")>();
  return {
    ...actual,
    readFileAsArrayBuffer: async () => new TextEncoder().encode("hello endpoint").buffer
  };
});

import { seedStore, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);

describe("D8 · fetchHealth", () => {
  it("宿主成功 → health 落值、phase=completed、step-context 置 completed", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("health_check", {
      appName: "Pony Agent",
      appVersion: "9.9.9",
      runtime: "rust",
      graphEngine: "graph",
      graphContractVersion: "v1"
    });

    await store.fetchHealth();

    expect(store.health?.appVersion).toBe("9.9.9");
    expect(store.phase).toBe("completed");
    expect(store.traceSteps.find((step) => step.id === "step-context")?.state).toBe("completed");
    expect(recorderMock.initFrontendFlightRecorder).toHaveBeenCalledTimes(1);
  });

  it("已有 health → 幂等短路，不重复打宿主", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("health_check", {
      appName: "Pony Agent",
      appVersion: "1.0.0",
      runtime: "rust",
      graphEngine: "graph",
      graphContractVersion: "v1"
    });

    await store.fetchHealth();
    await store.fetchHealth();

    expect(harness.backend().callsFor("health_check")).toHaveLength(1);
  });

  it("宿主失败 → error 文案 + phase=failed，不抛错", async () => {
    const store = harness.getStore();
    harness.backend().route("health_check", () => {
      throw new Error("boom");
    });

    await store.fetchHealth();

    expect(store.phase).toBe("failed");
    expect(store.error).toContain("Rust 后端连接失败");
  });
});

describe("D8 · 能力目录", () => {
  it("fetchAvailableTools：工具表为空时走宿主 list_available_tools 并落值", async () => {
    const store = harness.getStore();
    seedStore(store, { availableTools: [] });
    harness.backend().routeValue("list_available_tools", [{ name: "fs.read", description: "读文件" }]);

    await store.fetchAvailableTools();

    expect(store.availableTools.map((tool) => tool.name)).toEqual(["fs.read"]);
    expect(harness.backend().callsFor("list_available_tools")).toHaveLength(1);
  });

  it("fetchCapabilitySources：宿主返回落值", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("list_capability_sources", [{ sourceId: "src-1" }]);

    await store.fetchCapabilitySources();

    expect(store.capabilitySources).toEqual([{ sourceId: "src-1" }]);
  });

  it("fetchCapabilities：过滤参数去空白后透传宿主，返回落值", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("list_capabilities", [{ capabilityId: "cap-1" }]);

    await store.fetchCapabilities({ sourceId: "  src-1  ", kind: "  tool  " });

    expect(harness.backend().callsFor("list_capabilities")[0]?.args).toEqual({
      sourceId: "src-1",
      kind: "tool"
    });
    expect(store.capabilities).toEqual([{ capabilityId: "cap-1" }]);
  });

  it("inspectCapability：空 id 短路不触达宿主；有 id 时透传并返回", async () => {
    const store = harness.getStore();

    expect(await store.inspectCapability("   ")).toBeNull();
    expect(harness.backend().callsFor("inspect_capability")).toHaveLength(0);

    harness.backend().routeValue("inspect_capability", { capabilityId: "cap-1", kind: "tool" });
    expect(await store.inspectCapability("cap-1")).toEqual({ capabilityId: "cap-1", kind: "tool" });
    expect(harness.backend().callsFor("inspect_capability")[0]?.args).toEqual({ capabilityId: "cap-1" });
  });

  it("inspectCapabilitySource：空 id 短路不触达宿主；有 id 时透传并返回", async () => {
    const store = harness.getStore();

    expect(await store.inspectCapabilitySource("")).toBeNull();
    expect(harness.backend().callsFor("inspect_capability_source")).toHaveLength(0);

    harness.backend().routeValue("inspect_capability_source", { sourceId: "src-1" });
    expect(await store.inspectCapabilitySource("src-1")).toEqual({ sourceId: "src-1" });
  });
});

describe("用量治理", () => {
  it("overrideGovernanceBlock：清本地裁决并关闭持久化 enforcement", async () => {
    const store = harness.getStore();
    settingsMock.state.settings.usageGovernancePolicy = { enforcement: true };
    settingsMock.state.saveSettings.mockClear();
    (store as unknown as Record<string, unknown>).governanceDecision = { blocked: true };

    await store.overrideGovernanceBlock();

    expect((store as unknown as Record<string, unknown>).governanceDecision).toBeNull();
    expect(settingsMock.state.settings.usageGovernancePolicy.enforcement).toBe(false);
    expect(settingsMock.state.saveSettings).toHaveBeenCalledTimes(1);
  });

  it("fetchUsageGovernanceSummary：非 Tauri 零调用；Tauri 下透传 sessionId 并落值", async () => {
    const store = harness.getStore();

    tauriMocks.mockIsTauriAvailable.mockReturnValueOnce(false);
    await store.fetchUsageGovernanceSummary();
    expect(harness.backend().callsFor("usage_governance_summary")).toHaveLength(0);

    harness.backend().routeValue("usage_governance_summary", { blocked: false, totalUsd: 1.5 });
    await store.fetchUsageGovernanceSummary();

    expect(harness.backend().callsFor("usage_governance_summary")[0]?.args).toEqual({
      sessionId: store.sessionId
    });
    expect((store as unknown as Record<string, unknown>).backendGovernanceSummary).toEqual({
      blocked: false,
      totalUsd: 1.5
    });
  });
});

describe("停止链", () => {
  it("stopTurn：未在提交中 → false 且不触达宿主", async () => {
    const store = harness.getStore();

    expect(await store.stopTurn()).toBe(false);
    expect(harness.backend().callsFor("stop_turn")).toHaveLength(0);
    expect(harness.backend().callsFor("stop_graph_run")).toHaveLength(0);
  });

  it("stopTurn：无 runId 时走 stop_turn，接受后同微任务进入 stopping", async () => {
    const store = harness.getStore();
    seedStore(store, { activeTurnId: "turn-stop-1", isSubmitting: true, activeRunId: null });
    harness.backend().routeValue("stop_turn", { accepted: true });

    expect(await store.stopTurn()).toBe(true);
    expect(harness.backend().callsFor("stop_turn")[0]?.args).toEqual({ turnId: "turn-stop-1" });
    expect(store.phase).toBe("stopping");
  });

  it("stopTurn：宿主 accepted=false → 返回 false 且不切 stopping", async () => {
    const store = harness.getStore();
    seedStore(store, { activeTurnId: "turn-stop-2", isSubmitting: true, activeRunId: null });
    harness.backend().routeValue("stop_turn", { accepted: false });

    expect(await store.stopTurn()).toBe(false);
    expect(store.phase).not.toBe("stopping");
  });

  it("enterStoppingPhase：无 activeTurnId 或非提交态幂等不动；在飞时落 stopping", () => {
    const store = harness.getStore();

    store.enterStoppingPhase();
    expect(store.phase).not.toBe("stopping");

    seedStore(store, { activeTurnId: "turn-stop-3", isSubmitting: true });
    store.enterStoppingPhase();
    expect(store.phase).toBe("stopping");

    store.enterStoppingPhase();
    expect(store.phase).toBe("stopping");
  });
});

describe("会话通知", () => {
  it("setSessionNotice：写值 / 置空", () => {
    const store = harness.getStore();

    store.setSessionNotice("已完成");
    expect(store.sessionNotice).toBe("已完成");

    store.setSessionNotice(null);
    expect(store.sessionNotice).toBeNull();
  });
});

describe("待发送附件", () => {
  it("addPendingAttachments：不支持的类型 → 记 errors 且不入列表", async () => {
    const store = harness.getStore();
    const before = store.pendingAttachments.length;

    const result = await store.addPendingAttachments([
      new File(["x"], "a.xyz", { type: "application/octet-stream" })
    ]);

    expect(result.added).toBe(0);
    expect(result.errors[0]).toContain("暂不支持该文件类型");
    expect(store.pendingAttachments).toHaveLength(before);
  });

  it("addPendingAttachments：文本文件经宿主导入后入列表", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("import_attachment", {
      path: "/tmp/notes.txt",
      relativePath: "notes.txt"
    });

    const result = await store.addPendingAttachments([
      new File(["hello"], "notes.txt", { type: "text/plain" })
    ]);

    expect(result.added).toBe(1);
    expect(result.errors).toEqual([]);
    expect(store.pendingAttachments[0]?.route).toBe("text");
    expect(store.pendingAttachments[0]?.path).toBe("/tmp/notes.txt");
    expect(harness.backend().callsFor("import_attachment")).toHaveLength(1);
  });

  it("addPendingAttachments：图片已达 3 张上限 → 拒绝并记 errors", async () => {
    const store = harness.getStore();
    seedStore(store, {
      pendingAttachments: Array.from({ length: 3 }, (_, index) => ({
        id: `pending-image-${index}`,
        name: `image-${index}.png`,
        sizeBytes: 8,
        mimeType: "image/png",
        route: "image",
        status: "ok"
      }))
    });

    const result = await store.addPendingAttachments([
      new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], "extra.png", { type: "image/png" })
    ]);

    expect(result.added).toBe(0);
    expect(result.errors.some((error) => error.includes("最多添加 3 张图片"))).toBe(true);
    expect(store.pendingAttachments).toHaveLength(3);
  });

  it("removePendingAttachment：按 id 移除，其余保持", () => {
    const store = harness.getStore();
    seedStore(store, {
      pendingAttachments: [
        { id: "keep", name: "keep.txt", route: "text", status: "ok", sizeBytes: 1, mimeType: "text/plain" },
        { id: "drop", name: "drop.txt", route: "text", status: "ok", sizeBytes: 1, mimeType: "text/plain" }
      ]
    });

    store.removePendingAttachment("drop");

    expect(store.pendingAttachments.map((attachment) => attachment.id)).toEqual(["keep"]);
  });

  it("getAttachmentAssets：无过滤返回全量，statuses 过滤生效", () => {
    const store = harness.getStore();
    seedStore(store, {
      attachmentAssets: [
        { id: "a1", sessionId: "s1", name: "a.md", mimeType: "text/markdown", sizeBytes: 1, status: "active", kind: "text" },
        { id: "a2", sessionId: "s1", name: "b.md", mimeType: "text/markdown", sizeBytes: 1, status: "expired", kind: "text" }
      ]
    });

    expect(store.getAttachmentAssets()).toHaveLength(2);
    expect(
      store.getAttachmentAssets({ statuses: ["active"] }).map((asset) => asset.id)
    ).toEqual(["a1"]);
  });
});
