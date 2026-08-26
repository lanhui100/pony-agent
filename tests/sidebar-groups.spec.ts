// PA-三级树：deriveSidebarTree 纯函数契约单元测试（无组件挂载）。
// 排序单一规则：updatedAtMs desc、tie conversationId asc；瞬态钉顶不参与排序；
// 归档过滤（缺省 ⇒ 存活）；default 不产出组行；孤儿并入平铺区。
import { describe, expect, it } from "vitest";
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import {
  deriveSidebarTree,
  isVisibleSession,
  type SidebarWorkspaceInput
} from "@/lib/runtime/sidebar-groups";
import type { SessionOverview } from "@/types/runtime";

function session(
  id: string,
  workspaceId?: string | null,
  overrides: Partial<SessionOverview> = {}
): SessionOverview {
  return {
    conversationId: id,
    title: id,
    summary: null,
    turnCount: 1,
    lastReferencedFile: null,
    updatedAtMs: 1000,
    workspaceId: workspaceId ?? null,
    ...overrides
  };
}

const registry: SidebarWorkspaceInput[] = [
  { id: DEFAULT_WORKSPACE_ID, name: "默认工作区" },
  { id: "ws-a", name: "A" },
  { id: "ws-b", name: "B" }
];

describe("deriveSidebarTree", () => {
  it("default 工作区不渲染组行；其会话进入平铺区", () => {
    const tree = deriveSidebarTree(
      [session("d1"), session("a1", "ws-a")],
      registry
    );
    expect(tree.workspaces.map((g) => g.key)).toEqual(["ws-a", "ws-b"]);
    expect(tree.flatZone.map((s) => s.conversationId)).toEqual(["d1"]);
  });

  it("孤儿与无归属会话并入平铺区并按 updatedAtMs desc、tie conversationId asc 排序", () => {
    const tree = deriveSidebarTree(
      [
        session("orphan", "ws-gone", { updatedAtMs: 3000 }),
        session("plain", null, { updatedAtMs: 3000 }),
        session("old", null, { updatedAtMs: 1000 })
      ],
      registry
    );
    expect(tree.flatZone.map((s) => s.conversationId)).toEqual(["orphan", "plain", "old"]);
  });

  it("组内同用全局排序规则（updatedAtMs desc, tie id asc）", () => {
    const tree = deriveSidebarTree(
      [
        session("a2", "ws-a", { updatedAtMs: 2000 }),
        session("a1", "ws-a", { updatedAtMs: 2000 }),
        session("a3", "ws-a", { updatedAtMs: 5000 })
      ],
      registry
    );
    expect(tree.workspaces[0].sessions.map((s) => s.conversationId)).toEqual(["a3", "a1", "a2"]);
  });

  it("归档会话从分区、计数与排序中完全消失；缺省字段视为存活", () => {
    const tree = deriveSidebarTree(
      [
        session("live", "ws-a"),
        session("hidden", "ws-a", { archived: true }),
        session("no-field")
      ],
      registry
    );
    expect(tree.workspaces[0].count).toBe(1);
    expect(tree.workspaces[0].sessions.map((s) => s.conversationId)).toEqual(["live"]);
    expect(tree.flatZone.some((s) => s.conversationId === "no-field")).toBe(true);
    expect(isVisibleSession({ ...session("x"), archived: undefined })).toBe(true);
  });

  it("瞬态条目按 target 钉顶：default 目标 → 平铺区顶", () => {
    const transient = { overview: session("transient", null, { title: "新对话" }), target: DEFAULT_WORKSPACE_ID };
    const tree = deriveSidebarTree([session("old")], registry, transient);
    expect(tree.flatZone[0].conversationId).toBe("transient");
  });

  it("瞬态条目目标为显式工作区 → 该组钉顶", () => {
    const transient = { overview: session("t2", "ws-b"), target: "ws-b" };
    const tree = deriveSidebarTree([], registry, transient);
    expect(tree.workspaces.find((g) => g.key === "ws-b").sessions[0].conversationId).toBe("t2");
  });

  it("删除工作区后残留创建目标指向已注销 id：瞬态回退平铺区顶（后端已归一，双保险）", () => {
    const transient = { overview: session("t3", "ws-gone"), target: "ws-gone" };
    const tree = deriveSidebarTree([], [{ id: DEFAULT_WORKSPACE_ID, name: "默认工作区" }], transient);
    expect(tree.workspaces).toHaveLength(0);
    expect(tree.flatZone[0].conversationId).toBe("t3");
  });

  it("零计数工作区保留空组（空态提示面）", () => {
    const tree = deriveSidebarTree([], registry.filter((w) => w.id !== DEFAULT_WORKSPACE_ID));
    expect(tree.workspaces).toHaveLength(2);
    expect(tree.workspaces.every((g) => g.count === 0)).toBe(true);
  });
});
