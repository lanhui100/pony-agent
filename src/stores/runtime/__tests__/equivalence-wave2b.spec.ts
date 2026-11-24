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

describe("D2 · 工作区注册表（loadWorkspaces / createNewWorkspace / renameWorkspace / deleteWorkspace）", () => {
  it("loadWorkspaces：宿主返回落值并标记 loaded", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("workspace_list", [
      { id: "default", name: "默认", rootPath: "/w/default" },
      { id: "ws-1", name: "项目", rootPath: "/w/one" }
    ]);

    await store.loadWorkspaces();

    expect(store.workspaceList.map((w) => w.id)).toEqual(["default", "ws-1"]);
    expect(store.workspaceListLoaded).toBe(true);
  });

  it("loadWorkspaces：非数组返回 ⇒ 空列表（防御非契约通道）", async () => {
    const store = harness.getStore();
    harness.backend().routeValue("workspace_list", { oops: true });

    await store.loadWorkspaces();

    expect(store.workspaceList).toEqual([]);
    expect(store.workspaceListLoaded).toBe(true);
  });

  it("loadWorkspaces：宿主失败 ⇒ 空列表且 loaded 保持 false（允许重试）", async () => {
    const store = harness.getStore();
    harness.backend().route("workspace_list", () => {
      throw new Error("boom");
    });

    await store.loadWorkspaces();

    expect(store.workspaceList).toEqual([]);
    expect(store.workspaceListLoaded).toBe(false);
  });

  it("createNewWorkspace：成功按 id 去重后追加", async () => {
    const store = harness.getStore();
    seedStore(store, { workspaceList: [{ id: "ws-1", name: "旧名", rootPath: "/w/one" }] });
    harness.backend().routeValue("workspace_create", {
      id: "ws-1",
      name: "新名",
      rootPath: "/w/one"
    });

    const result = await store.createNewWorkspace("  新名  ", "  /w/one  ");

    expect(result.ok).toBe(true);
    expect(result.record?.name).toBe("新名");
    expect(store.workspaceList).toHaveLength(1);
    expect(store.workspaceList[0].name).toBe("新名");
  });

  it("createNewWorkspace：名称或根路径为空 ⇒ 拒绝且不调宿主", async () => {
    const store = harness.getStore();

    const result = await store.createNewWorkspace("   ", "/w/one");

    expect(result.ok).toBe(false);
    expect(result.error).toContain("不能为空");
    expect(harness.backend().callsFor("workspace_create")).toHaveLength(0);
  });

  it("createNewWorkspace：宿主报重复 root ⇒ 重拉注册表并命中既有记录", async () => {
    const store = harness.getStore();
    harness.backend().route("workspace_create", () => {
      throw new Error("重复 root：/w/one");
    });
    harness.backend().routeValue("workspace_list", [
      { id: "ws-9", name: "既有", rootPath: "/w/one" }
    ]);

    const result = await store.createNewWorkspace("既有", "/w/one");

    expect(result.ok).toBe(true);
    expect(result.record?.id).toBe("ws-9");
    expect(harness.backend().callsFor("workspace_list")).toHaveLength(1);
  });

  it("renameWorkspace：成功替换列表中的同名 id 记录", async () => {
    const store = harness.getStore();
    seedStore(store, { workspaceList: [{ id: "ws-1", name: "旧名", rootPath: "/w/one" }] });
    harness.backend().routeValue("workspace_rename", {
      id: "ws-1",
      name: "改后",
      rootPath: "/w/one"
    });

    const result = await store.renameWorkspace("ws-1", "  改后  ");

    expect(result.ok).toBe(true);
    expect(store.workspaceList[0].name).toBe("改后");
    expect(harness.backend().callsFor("workspace_rename")).toHaveLength(1);
  });

  it("renameWorkspace：空白名与超 64 字符都在本地拒绝，不触达宿主", async () => {
    const store = harness.getStore();

    const blank = await store.renameWorkspace("ws-1", "   ");
    const tooLong = await store.renameWorkspace("ws-1", "x".repeat(65));

    expect(blank.error).toContain("不能为空");
    expect(tooLong.error).toContain("64");
    expect(harness.backend().callsFor("workspace_rename")).toHaveLength(0);
  });

  it("deleteWorkspace：默认工作区不可删除", async () => {
    const store = harness.getStore();

    const result = await store.deleteWorkspace("default");

    expect(result.ok).toBe(false);
    expect(result.error).toContain("默认工作区");
    expect(harness.backend().callsFor("workspace_delete")).toHaveLength(0);
  });

  it("deleteWorkspace：成功移除记录并把创建目标归一为默认工作区", async () => {
    const store = harness.getStore();
    seedStore(store, {
      workspaceList: [
        { id: "default", name: "默认", rootPath: "/w/default" },
        { id: "ws-1", name: "项目", rootPath: "/w/one" }
      ],
      sessionWorkspaceId: "ws-1"
    });
    harness.backend().routeValue("workspace_delete", null);
    harness.backend().routeValue("list_sessions", []);

    const result = await store.deleteWorkspace("ws-1");

    expect(result.ok).toBe(true);
    expect(store.workspaceList.map((w) => w.id)).toEqual(["default"]);
    expect(store.sessionWorkspaceId).toBe("default");
  });
});

describe("D1 · 会话目录写操作（renameSession / archiveSession / unarchiveSession）", () => {
  it("renameSession：成功调用宿主并刷新目录", async () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionList: [{ conversationId: "s1", title: "旧标题" }],
      sessionId: "s1"
    });
    harness.backend().routeValue("session_rename", null);
    harness.backend().routeValue("list_sessions", [{ conversationId: "s1", title: "新标题" }]);

    const result = await store.renameSession("s1", "  新标题  ");

    expect(result.ok).toBe(true);
    expect(harness.backend().callsFor("session_rename")[0]?.args).toMatchObject({
      sessionId: "s1",
      title: "新标题"
    });
    expect(store.sessionList[0].title).toBe("新标题");
  });

  it("renameSession：空白名 / 超 64 / 会话不存在都在本地拒绝", async () => {
    const store = harness.getStore();
    seedStore(store, { sessionList: [{ conversationId: "s1" }] });

    const blank = await store.renameSession("s1", "   ");
    const tooLong = await store.renameSession("s1", "x".repeat(65));
    const missing = await store.renameSession("nope", "合法名");

    expect(blank.error).toContain("不能为空");
    expect(tooLong.error).toContain("64");
    expect(missing.error).toContain("会话不存在");
    expect(harness.backend().callsFor("session_rename")).toHaveLength(0);
  });

  it("archiveSession：当前会话提交期拒绝归档", async () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionList: [{ conversationId: "s1" }],
      sessionId: "s1",
      isSubmitting: true
    });

    const result = await store.archiveSession("s1");

    expect(result.ok).toBe(false);
    expect(result.error).toContain("正在提交");
    expect(harness.backend().callsFor("session_archive")).toHaveLength(0);
  });

  it("archiveSession：运行中的后台会话拒绝归档", async () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionList: [{ conversationId: "s2" }],
      runningSessionMap: { s2: true }
    });

    const result = await store.archiveSession("s2");

    expect(result.ok).toBe(false);
    expect(result.error).toContain("运行中");
    expect(harness.backend().callsFor("session_archive")).toHaveLength(0);
  });

  it("archiveSession：成功调用 session_archive 并刷新目录", async () => {
    const store = harness.getStore();
    seedStore(store, { sessionList: [{ conversationId: "s2" }] });
    harness.backend().routeValue("session_archive", null);
    harness.backend().routeValue("list_sessions", [{ conversationId: "s2", archived: true }]);

    const result = await store.archiveSession("s2");

    expect(result.ok).toBe(true);
    expect(harness.backend().callsFor("session_archive")).toHaveLength(1);
    expect(store.sessionList[0].archived).toBe(true);
  });

  it("unarchiveSession：成功调用 session_unarchive 并刷新目录", async () => {
    const store = harness.getStore();
    seedStore(store, { sessionList: [{ conversationId: "s3", archived: true }] });
    harness.backend().routeValue("session_unarchive", null);
    harness.backend().routeValue("list_sessions", [{ conversationId: "s3", archived: false }]);

    const result = await store.unarchiveSession("s3");

    expect(result.ok).toBe(true);
    expect(harness.backend().callsFor("session_unarchive")).toHaveLength(1);
    expect(store.sessionList[0].archived).toBe(false);
  });
});
