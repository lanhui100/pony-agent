import { isTauriAvailable } from "@/lib/tauri";
import type { TeammateMember, TeamTaskItem } from "@/types/team";

/**
 * 浏览器 UI 评审专用预览夹具（preview fixtures）。
 *
 * - 唯一用途：在没有 Tauri 运行时的浏览器 dev 环境里给右侧栏「队友 / 任务看板」两块面板
 *   提供可见的形状与数据，便于人工评审布局、truncate 行为与两种排序切换效果。
 * - 生效条件严格限定：仅 `import.meta.env.DEV === true` 且 `isTauriAvailable() === false`。
 *   Tauri 运行时（含 dev 构建）永不启用，因此不参与任何生产数据路径。
 * - 本文件为纯常量 + 纯函数，不写 store、不发 IPC、不做兜底假数据注入；
 *   一旦 `useTeamStore()` 拿到真实数据，组件立即改用 store 数据，夹具自动让位。
 */

/** 仅浏览器 dev 预览生效：Tauri 运行时（含 dev）永不启用。 */
export function shouldUseTeamPreview(): boolean {
  return import.meta.env.DEV && !isTauriAvailable();
}

export const TEAM_PREVIEW_MEMBERS: TeammateMember[] = [
  {
    target: "lead",
    name: "总指挥 Lead",
    role: "lead",
    status: "running",
    description: "编排与最终裁决"
  },
  {
    target: "executor-frontend",
    name: "前端实施者 Executor-Frontend 超长名称",
    role: "backend",
    status: "active",
    description: "右侧栏面板实现"
  },
  {
    target: "verifier",
    name: "验收 Verifier",
    role: "qa",
    status: "provisioning",
    description: "独立验收断言"
  },
  {
    target: "scribe",
    name: "文档 Scribe",
    role: "docs",
    status: "inactive",
    description: "文档与收据整理"
  },
  {
    target: "infra-ops",
    name: "运维 Ops",
    role: "infra",
    status: "failed",
    description: "打包冒烟失败待重启"
  }
];

/** created_at_ms 以 1_700_000_000_000 为基准，每条 +60000，严格递增。 */
const PREVIEW_TASK_EPOCH_MS = 1_700_000_000_000;
const PREVIEW_TASK_STEP_MS = 60_000;

function previewCreatedAt(index: number): number {
  return PREVIEW_TASK_EPOCH_MS + index * PREVIEW_TASK_STEP_MS;
}

/**
 * 7 条任务，创建次序的状态序列为：
 * completed, pending, in_progress, completed, pending, in_progress, pending
 *
 * =>「生成次序」呈混排；「按进展」重排为 in_progress ×2 → pending ×3 → completed ×2，
 *    两种模式切换时列表顺序肉眼可区分。
 */
export const TEAM_PREVIEW_TASKS: TeamTaskItem[] = [
  {
    task_id: "task-01",
    subject: "搭建右侧栏面板",
    description: "复用折叠面板样式，对齐 PlanPanel 观感",
    status: "completed",
    owner: "executor-frontend",
    revision: 2,
    created_at_ms: previewCreatedAt(0),
    write_scopes: ["src/components/team"],
    blocked_by: []
  },
  {
    task_id: "task-02",
    subject: "排序契约对齐",
    description: "以 created_at_ms 为生成次序排序键",
    status: "pending",
    owner: null,
    revision: 1,
    created_at_ms: previewCreatedAt(1),
    write_scopes: ["src/stores/team.ts", "src/types/team.ts"],
    blocked_by: ["task-01"]
  },
  {
    task_id: "task-03",
    subject: "花名册面板",
    description: "渲染队友状态点、名称与角色标签",
    status: "in_progress",
    owner: "executor-frontend",
    revision: 4,
    created_at_ms: previewCreatedAt(2),
    write_scopes: ["src/components/team/TeamRosterPanel.vue"],
    blocked_by: []
  },
  {
    task_id: "task-04",
    subject: "预览夹具隔离",
    description: "仅 dev 浏览器生效，绝不写入 store",
    status: "completed",
    owner: "scribe",
    revision: 3,
    created_at_ms: previewCreatedAt(3),
    write_scopes: ["src/components/team/team-preview-fixtures.ts"],
    blocked_by: []
  },
  {
    task_id: "task-05",
    subject: "真机 IPC 校验",
    description: "Tauri 内验证任务字段完整回传",
    status: "pending",
    owner: null,
    revision: 1,
    created_at_ms: previewCreatedAt(4),
    write_scopes: ["src-tauri/src"],
    blocked_by: ["task-03"]
  },
  {
    task_id: "task-06",
    subject: "双排序验收",
    description: "断言两种排序顺序与状态分组均正确",
    status: "in_progress",
    owner: "verifier",
    revision: 4,
    created_at_ms: previewCreatedAt(5),
    write_scopes: ["tests/acceptance", "tests/"],
    blocked_by: ["task-02", "task-03"]
  },
  {
    task_id: "task-07",
    subject: "打包回归",
    description: "构建并冒烟验证右栏渲染无报错",
    status: "pending",
    owner: "infra-ops",
    revision: 2,
    created_at_ms: previewCreatedAt(6),
    write_scopes: ["scripts/"],
    blocked_by: []
  }
];

export type TeamTaskSortMode = "created" | "progress";

/** 与 useTeamStore().sortedTasks 完全一致的排序规则（供预览数据复用同一套语义）。 */
const PREVIEW_PROGRESS_GROUP_ORDER: Record<TeamTaskItem["status"], number> = {
  in_progress: 0,
  pending: 1,
  completed: 2,
  deleted: 3
};

function comparePreviewTasks(left: TeamTaskItem, right: TeamTaskItem): number {
  if (left.created_at_ms !== right.created_at_ms) {
    return left.created_at_ms - right.created_at_ms;
  }
  return left.task_id < right.task_id ? -1 : left.task_id > right.task_id ? 1 : 0;
}

/**
 * 纯排序函数：按传入 mode 返回新数组，不修改入参，deleted 永远被过滤。
 * 预览模式下的任务看板必须走这里，才能真实响应排序切换开关。
 */
export function sortTeamTasks(
  tasks: TeamTaskItem[],
  mode: TeamTaskSortMode
): TeamTaskItem[] {
  const visible = tasks.filter((task) => task.status !== "deleted");
  if (mode === "progress") {
    return visible.slice().sort((left, right) => {
      const groupDelta =
        PREVIEW_PROGRESS_GROUP_ORDER[left.status] - PREVIEW_PROGRESS_GROUP_ORDER[right.status];
      if (groupDelta !== 0) {
        return groupDelta;
      }
      return comparePreviewTasks(left, right);
    });
  }
  return visible.slice().sort(comparePreviewTasks);
}
