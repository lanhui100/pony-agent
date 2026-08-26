// PA-三级树：侧边栏单一树派生（Workspace 一级标题行 → 工作区二级 → 会话三级）。
// 取代 PA-081 的 groupSessionsByWorkspace 两级分组 + 折叠持久化（该契约已废弃：
// 组不可折叠、default 不渲染组头、孤儿并入平铺区）。
// 单一事实源规则：
// - 归档会话（Boolean(session.archived) === true）不进入任何分区、计数或排序；
// - 平铺区 = 默认工作区名下 ∪ 无 workspaceId ∪ 指向未注册 id 的孤儿，
//   无组头直接列于「工作区」标题行之下；
// - 工作区行仅渲染注册表中的**非 default** 记录（default 恒不渲染二级行）；
// - 排序禁止发明新规则：全局 list_sessions 序 = updatedAtMs desc、
//   tie conversationId asc；平铺区与各组分区内均为其过滤投影；
// - 瞬态"新对话"条目按其显式 target 钉顶归属，不参与排序。
// 缺省 ⇒ 存活：archived 字段缺省（undefined，本地构造）一律视为未归档。
import { DEFAULT_WORKSPACE_ID } from "@/lib/runtime/workspace-constants";
import type { SessionOverview } from "@/types/runtime";

export interface SidebarWorkspaceInput {
  id: string;
  name: string;
}

/** 单一树的两个分区：无组头平铺区 + 各显式工作区组。 */
export interface SidebarTree {
  flatZone: SessionOverview[];
  workspaces: SidebarWorkspaceGroup[];
}

export interface SidebarWorkspaceGroup {
  /** 注册表中的工作区 id（不含 default）。 */
  key: string;
  name: string;
  /** 可见（非归档）会话计数徽标口径。 */
  count: number;
  sessions: SessionOverview[];
}

function byRecencyThenId(left: SessionOverview, right: SessionOverview): number {
  if (right.updatedAtMs !== left.updatedAtMs) {
    return right.updatedAtMs - left.updatedAtMs;
  }
  return left.conversationId < right.conversationId ? -1 : 1;
}

/** 存活判定：归档过滤的唯一入口（缺省 undefined ⇒ 存活）。 */
export function isVisibleSession(session: SessionOverview): boolean {
  return !session.archived;
}

/**
 * 派生侧边栏单一树。sessions 期望已是全局 recency 序（list_sessions 投影），
 * 函数内部仍按契约重排以保证纯函数自洽。
 * @param sessions 全量可见候选（含归档，由本函数过滤）
 * @param workspaces 工作区注册表（含 default；default 不产出组行）
 * @param transient 瞬态"新对话"条目（未保存）；target 为其显式创建目标
 */
export function deriveSidebarTree(
  sessions: SessionOverview[],
  workspaces: SidebarWorkspaceInput[],
  transient?: { target: string | null; overview: SessionOverview }
): SidebarTree {
  const registryIds = new Set(workspaces.map((workspace) => workspace.id));

  const flatZone: SessionOverview[] = [];
  const grouped = new Map<string, SessionOverview[]>();
  for (const workspace of workspaces) {
    if (workspace.id === DEFAULT_WORKSPACE_ID || !registryIds.has(workspace.id)) continue;
    grouped.set(workspace.id, []);
  }

  for (const session of sessions) {
    if (!isVisibleSession(session)) continue;
    const owner = session.workspaceId?.trim() ?? "";
    if (owner === "" || owner === DEFAULT_WORKSPACE_ID || !registryIds.has(owner)) {
      // default / 无归属 / 孤儿 → 平铺区
      flatZone.push(session);
    } else {
      grouped.get(owner)?.push(session);
    }
  }

  flatZone.sort(byRecencyThenId);

  const workspaceGroups: SidebarWorkspaceGroup[] = [];
  for (const workspace of workspaces) {
    const members = grouped.get(workspace.id);
    if (members === undefined) continue;
    members.sort(byRecencyThenId);
    workspaceGroups.push({
      key: workspace.id,
      name: workspace.name || workspace.id,
      count: members.length,
      sessions: members
    });
  }

  // 瞬态条目：排序完成后钉在所属分区顶部，不参与排序。
  if (transient && isVisibleSession(transient.overview)) {
    const target = transient.target?.trim() ?? "";
    if (target !== "" && target !== DEFAULT_WORKSPACE_ID && grouped.has(target)) {
      grouped.get(target)?.unshift(transient.overview);
    } else {
      flatZone.unshift(transient.overview);
    }
  }

  return { flatZone, workspaces: workspaceGroups };
}

/** 激活 Workspace 的 localStorage 单一真相源 key（常量单源在 workspace-constants）。 */
export { ACTIVE_WORKSPACE_STORAGE_KEY } from "@/lib/runtime/workspace-constants";
