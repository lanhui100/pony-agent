// PA-081：侧边栏分组纯函数单元测试（无组件挂载，直接验证契约）。
import { describe, expect, it } from "vitest";
import {
  UNGROUPED_GROUP_KEY,
  groupSessionsByWorkspace,
  loadStoredWorkspaceGroupKeys,
  persistCollapsedWorkspaceGroups
} from "@/lib/runtime/sidebar-groups";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import type { SessionOverview } from "@/types/runtime";

function session(id: string, workspaceId?: string | null): SessionOverview {
  return {
    conversationId: id,
    title: id,
    summary: null,
    turnCount: 1,
    lastReferencedFile: null,
    updatedAtMs: 1000,
    workspaceId: workspaceId ?? null
  };
}

describe("groupSessionsByWorkspace", () => {
  it("routes null workspaceId sessions to the default group", () => {
    const groups = groupSessionsByWorkspace(
      [session("a"), session("b", "ws-x")],
      [
        { id: DEFAULT_WORKSPACE_ID, name: "默认工作区" },
        { id: "ws-x", name: "X" }
      ]);
    const byKey = new Map(groups.map((group) => [group.key, group]));
    expect(byKey.get(DEFAULT_WORKSPACE_ID)?.sessions.map((s) => s.conversationId)).toEqual(["a"]);
    expect(byKey.get("ws-x")?.sessions.map((s) => s.conversationId)).toEqual(["b"]);
  });

  it("creates a synthetic default group when the registry omits it", () => {
    const groups = groupSessionsByWorkspace(
      [session("a")],
      [{ id: "ws-only", name: "Only" }]);
    expect(groups[0]?.key).toBe(DEFAULT_WORKSPACE_ID);
    expect(groups[0]?.name).toBe("默认工作区");
  });

  it("collects unknown workspaceIds into a trailing ungrouped group", () => {
    const groups = groupSessionsByWorkspace(
      [session("known", "ws-a"), session("orphan", "ws-gone"), session("plain")],
      [{ id: DEFAULT_WORKSPACE_ID, name: "默认" }, { id: "ws-a", name: "A" }]);
    const keys = groups.map((group) => group.key);
    expect(keys[keys.length - 1]).toBe(UNGROUPED_GROUP_KEY);
    const ungrouped = groups.find((group) => group.key === UNGROUPED_GROUP_KEY);
    expect(ungrouped?.workspaceId).toBeNull();
    expect(ungrouped?.name).toBe("未分组");
    expect(ungrouped?.sessions.map((s) => s.conversationId)).toEqual(["orphan"]);
  });

  it("keeps empty workspaces as zero-count groups (empty-state surface)", () => {
    const groups = groupSessionsByWorkspace(
      [session("a")],
      [
        { id: DEFAULT_WORKSPACE_ID, name: "默认" },
        { id: "ws-empty", name: "空项目" }
      ]);
    const emptyGroup = groups.find((group) => group.key === "ws-empty");
    expect(emptyGroup?.sessions.length ?? -1).toBe(0);
  });

  it("places transient entries (active workspace id) into the active group", () => {
    const groups = groupSessionsByWorkspace(
      [session("transient", "ws-active")],
      [
        { id: DEFAULT_WORKSPACE_ID, name: "默认" },
        { id: "ws-active", name: "激活项目" }
      ]
    );
    const active = groups.find((group) => group.key === "ws-active");
    expect(active?.sessions.map((s) => s.conversationId)).toEqual(["transient"]);
  });
});

describe("workspace group collapse persistence", () => {
  const storageStub = () => {
    const backing = new Map<string, string>();
    return {
      getItem: (key: string) => backing.get(key) ?? null,
      setItem: (key: string, value: string) => void backing.set(key, value)
    };
  };

  it("round-trips collapsed keys and prunes invalid ones on write", () => {
    const storage = storageStub();
    persistCollapsedWorkspaceGroups(["default", "ws-x"], new Set(["default"]), storage);
    expect(storage.getItem("pony-agent.session-sidebar-workspace-groups.v1")).toBe(
      JSON.stringify(["default"])
    );

    // 原始读取保留全部 key（合法组过滤由渲染/写回时进行）。
    storage.setItem(
      "pony-agent.session-sidebar-workspace-groups.v1",
      JSON.stringify(["default", "stale-key"])
    );
    const rawKeys = loadStoredWorkspaceGroupKeys(storage);
    expect(rawKeys.has("default")).toBe(true);
    expect(rawKeys.has("stale-key")).toBe(true);
  });

  it("tolerates corrupted payloads and missing storage", () => {
    const storage = storageStub();
    storage.setItem("pony-agent.session-sidebar-workspace-groups.v1", "{not-json");
    expect(loadStoredWorkspaceGroupKeys(storage).size).toBe(0);
    expect(loadStoredWorkspaceGroupKeys(undefined).size).toBe(0);
    expect(() =>
      persistCollapsedWorkspaceGroups(["default"], new Set(["default"]), undefined)
    ).not.toThrow();
  });
});
