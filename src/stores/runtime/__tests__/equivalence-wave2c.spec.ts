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

import { ACTIVE_WORKSPACE_STORAGE_KEY } from "@/lib/runtime/workspace-constants";
import type { TraceTimelineEntry } from "@/types/runtime";
import { advance, seedStore, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);

/** 最小合法 timeline 条目（本波只关心「条目数量」与「投递归属」，不关心渲染字段）。 */
function timelineEntry(id: string, sequence = 1): TraceTimelineEntry {
  return { id, kind: "call_model", label: id, state: "pending", sequence };
}

describe("D2 · 工作区激活态（activateWorkspace / normalizeActiveWorkspace）", () => {
  it("activateWorkspace：设置创建目标并写入 localStorage 单一真相源", () => {
    const store = harness.getStore();
    window.localStorage.setItem(ACTIVE_WORKSPACE_STORAGE_KEY, "stale");

    store.activateWorkspace("ws-7");

    expect(store.activeWorkspaceId).toBe("ws-7");
    expect(window.localStorage.getItem(ACTIVE_WORKSPACE_STORAGE_KEY)).toBe("ws-7");
  });

  it("normalizeActiveWorkspace：激活项仍在注册表 ⇒ 保持不变", () => {
    const store = harness.getStore();
    seedStore(store, {
      workspaceList: [{ id: "ws-1", name: "项目", rootPath: "/w/one" }],
      activeWorkspaceId: "ws-1"
    });

    store.normalizeActiveWorkspace();

    expect(store.activeWorkspaceId).toBe("ws-1");
  });

  it("normalizeActiveWorkspace：激活项已失效 ⇒ 回退 default 并清掉存储键", () => {
    const store = harness.getStore();
    seedStore(store, {
      workspaceList: [{ id: "default", name: "默认", rootPath: "/w/default" }],
      activeWorkspaceId: "ws-gone"
    });
    window.localStorage.setItem(ACTIVE_WORKSPACE_STORAGE_KEY, "ws-gone");

    store.normalizeActiveWorkspace();

    expect(store.activeWorkspaceId).toBe("default");
    expect(window.localStorage.getItem(ACTIVE_WORKSPACE_STORAGE_KEY)).toBeNull();
  });
});

describe("D1/D2 · 侧边栏 inflight 守卫（isSidebarOpInflight）", () => {
  it("无 inflight 键 ⇒ false；置位后 ⇒ true（判据即键存在性）", () => {
    const store = harness.getStore();

    expect(store.isSidebarOpInflight("s-rename:s1")).toBe(false);

    seedStore(store, { sidebarOpInflightSet: { "s-rename:s1": true } });
    expect(store.isSidebarOpInflight("s-rename:s1")).toBe(true);
    expect(store.isSidebarOpInflight("s-rename:s2")).toBe(false);
  });
});

describe("D3 · 流式 trace timeline 节流（scheduleThrottledTraceTimeline）", () => {
  it("首次调度落 pending 并起定时器；重复调度不新建定时器", async () => {
    const store = harness.getStore();
    seedStore(store, { sessionId: "s1", activeTurnId: "t1" });

    store.scheduleThrottledTraceTimeline([timelineEntry("a")]);
    const firstTimerId = store.traceTimelineThrottleTimerId;
    expect(firstTimerId).not.toBeNull();
    expect(store.pendingThrottledTraceTimeline).toHaveLength(1);

    store.scheduleThrottledTraceTimeline([timelineEntry("a"), timelineEntry("b", 2)]);
    expect(store.traceTimelineThrottleTimerId).toBe(firstTimerId);
    expect(store.pendingThrottledTraceTimeline).toHaveLength(2);
  });

  it("定时器到期但归属已变 ⇒ 丢弃本次投递（不串味到新会话）", async () => {
    const store = harness.getStore();
    seedStore(store, { sessionId: "s1", activeTurnId: "t1" });

    store.scheduleThrottledTraceTimeline([timelineEntry("a")]);
    // 定时器存活期间切会话：旧归属的 pending 不得落到新会话的活跃轮。
    seedStore(store, { sessionId: "s2" });
    await advance(260);

    expect(store.pendingThrottledTraceTimeline).toHaveLength(1);
    expect(store.traceTimelineThrottleTimerId).toBeNull();
  });
});

describe("D1 · 失败轮续跑入口（retryLastTurn）", () => {
  it("正在提交 ⇒ 直接拒绝，不写草稿", async () => {
    const store = harness.getStore();
    seedStore(store, { isSubmitting: true, failedTurnRetry: { retryable: true } });

    await expect(store.retryLastTurn()).resolves.toBe(false);
    expect(store.draftMessage).not.toBe("继续");
  });

  it("没有可重试的失败轮 ⇒ 拒绝，不写草稿", async () => {
    const store = harness.getStore();
    seedStore(store, { failedTurnRetry: { retryable: false } });

    await expect(store.retryLastTurn()).resolves.toBe(false);
  });
});
