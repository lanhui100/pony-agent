// PA-081：侧边栏两级树（Workspace → 会话）的纯函数分组与折叠持久化。
// 单一事实源规则：
// - session.workspaceId 为空 → 归 DEFAULT_WORKSPACE_ID 组；
// - workspaceId 命中注册表 → 对应 Workspace 组（组名取注册表名称）；
// - workspaceId 非空但不在注册表 → "未分组"孤儿组（会话仍可切换/删除）；
// - 瞬态"新对话"条目由 store 侧携带 activeWorkspaceId，自然归入激活组。
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import type { SessionOverview } from "@/types/runtime";

/** 孤儿组的保留 key（workspaceId 不在注册表中的会话）。 */
export const UNGROUPED_GROUP_KEY = "__ungrouped__";

export interface SidebarWorkspaceInput {
  id: string;
  name: string;
}

export interface SidebarSessionGroup {
  /** 折叠持久化 key：workspace id 或 UNGROUPED_GROUP_KEY。 */
  key: string;
  /** 关联的 workspace id；孤儿组为 null。 */
  workspaceId: string | null;
  name: string;
  sessions: SessionOverview[];
}

/**
 * 按 Workspace 分组会话。组顺序 = 注册表顺序（default 缺席时补默认组），
 * 孤儿"未分组"恒排最后；空组保留（渲染层显示空态提示）。
 */
export function groupSessionsByWorkspace(
  sessions: SessionOverview[],
  workspaces: SidebarWorkspaceInput[]
): SidebarSessionGroup[] {
  const workspaceById = new Map(workspaces.map((workspace) => [workspace.id, workspace]));
  const ordered: SidebarSessionGroup[] = [];
  const groupByKey = new Map<string, SidebarSessionGroup>();

  const ensureGroup = (key: string, workspaceId: string | null, name: string) => {
    const existing = groupByKey.get(key);
    if (existing) {
      return existing;
    }
    const group: SidebarSessionGroup = { key, workspaceId, name, sessions: [] };
    groupByKey.set(key, group);
    ordered.push(group);
    return group;
  };

  // 注册表顺序决定组顺序；default 未注册时补默认组（保持首位语义）。
  const defaultEntry = workspaceById.get(DEFAULT_WORKSPACE_ID);
  if (!defaultEntry) {
    ensureGroup(DEFAULT_WORKSPACE_ID, DEFAULT_WORKSPACE_ID, "默认工作区");
  }
  for (const workspace of workspaces) {
    ensureGroup(workspace.id, workspace.id, workspace.name || workspace.id);
  }

  for (const session of sessions) {
    const rawId = session.workspaceId?.trim() ?? "";
    if (rawId === "") {
      ensureGroup(DEFAULT_WORKSPACE_ID, DEFAULT_WORKSPACE_ID, defaultEntry?.name ?? "默认工作区").sessions.push(
        session
      );
      continue;
    }
    const known = workspaceById.get(rawId);
    if (known) {
      ensureGroup(known.id, known.id, known.name || known.id).sessions.push(session);
      continue;
    }
    ensureGroup(UNGROUPED_GROUP_KEY, null, "未分组").sessions.push(session);
  }

  return ordered;
}

/** 折叠状态持久化 key（PA-081 v1）。 */
export const SIDEBAR_WORKSPACE_GROUPS_STORAGE_KEY = "pony-agent.session-sidebar-workspace-groups.v1";

/**
 * 读取折叠组 key 集合（原始形态：不按合法组过滤——挂载时组集合尚未就绪，
 * 过滤由调用方在渲染/写回时进行）。损坏 JSON / 非字符串元素防御性忽略。
 */
export function loadStoredWorkspaceGroupKeys(
  storage: Pick<Storage, "getItem"> | undefined
): Set<string> {
  if (!storage) {
    return new Set();
  }
  try {
    const raw = storage.getItem(SIDEBAR_WORKSPACE_GROUPS_STORAGE_KEY);
    if (!raw) {
      return new Set();
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return new Set();
    }
    return new Set(parsed.filter((key): key is string => typeof key === "string"));
  } catch {
    return new Set();
  }
}

/** 持久化折叠的组 key 集合（仅当前合法 key，避免无限增长；存储不可写时静默降级）。 */
export function persistCollapsedWorkspaceGroups(
  collapsedKeys: Iterable<string>,
  validKeys: ReadonlySet<string>,
  storage: Pick<Storage, "setItem"> | undefined
): void {
  if (!storage) {
    return;
  }
  const filtered = [...collapsedKeys].filter((key) => validKeys.has(key));
  try {
    storage.setItem(SIDEBAR_WORKSPACE_GROUPS_STORAGE_KEY, JSON.stringify(filtered));
  } catch {
    // 存储不可写（隐私模式等）：折叠态退化为会话级内存态。
  }
}

/** 激活 Workspace 的 localStorage 单一真相源 key（常量单源在 workspace-constants）。 */
export { ACTIVE_WORKSPACE_STORAGE_KEY } from "@/lib/runtime/workspace-constants";
