import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type { TeammateMember, TeamTaskItem } from "@/types/team";

export type TeamSortMode = "created" | "progress";

interface TeamState {
  members: TeammateMember[];
  tasks: TeamTaskItem[];
  loading: boolean;
  error: string | null;
  sortMode: TeamSortMode;
}

function compareByCreatedAt(left: TeamTaskItem, right: TeamTaskItem): number {
  if (left.created_at_ms !== right.created_at_ms) {
    return left.created_at_ms - right.created_at_ms;
  }
  return left.task_id < right.task_id ? -1 : left.task_id > right.task_id ? 1 : 0;
}

const PROGRESS_GROUP_ORDER: Record<TeamTaskItem["status"], number> = {
  in_progress: 0,
  pending: 1,
  completed: 2,
  deleted: 3,
};

// 团队面板是「拉取式」可视化（后端无事件流）：面板挂载期间按固定间隔刷新，
// 引用计数保证多个面板共用同一个定时器，卸载时全部释放。
const TEAM_POLL_INTERVAL_MS = 2_000;
let teamPollTimer: ReturnType<typeof setInterval> | null = null;
let teamPollRefCount = 0;

export const useTeamStore = defineStore("team", {
  state: (): TeamState => ({
    members: [],
    tasks: [],
    loading: false,
    error: null,
    sortMode: "created",
  }),
  getters: {
    activeMembersCount(state): number {
      return state.members.filter((m) => m.status === "running" || m.status === "active").length;
    },
    pendingTasksCount(state): number {
      return state.tasks.filter((t) => t.status === "pending").length;
    },
    inProgressTasksCount(state): number {
      return state.tasks.filter((t) => t.status === "in_progress").length;
    },
    completedTasksCount(state): number {
      return state.tasks.filter((t) => t.status === "completed").length;
    },
    sortedTasks(state): TeamTaskItem[] {
      const visible = state.tasks.filter((t) => t.status !== "deleted");
      if (state.sortMode === "progress") {
        return visible.slice().sort((left, right) => {
          const groupDelta =
            PROGRESS_GROUP_ORDER[left.status] - PROGRESS_GROUP_ORDER[right.status];
          if (groupDelta !== 0) return groupDelta;
          return compareByCreatedAt(left, right);
        });
      }
      return visible.slice().sort(compareByCreatedAt);
    },
  },
  actions: {
    setSortMode(mode: TeamSortMode): void {
      this.sortMode = mode;
    },

    /// 面板挂载期间启动轮询（引用计数，多面板共用同一 timer）。
    startPolling(): void {
      teamPollRefCount += 1;
      if (teamPollTimer !== null) return;
      teamPollTimer = setInterval(() => {
        void this.fetchTeamOverview();
      }, TEAM_POLL_INTERVAL_MS);
    },

    /// 面板卸载时释放一次引用；归零后停止轮询。
    stopPolling(): void {
      teamPollRefCount = Math.max(0, teamPollRefCount - 1);
      if (teamPollRefCount > 0 || teamPollTimer === null) return;
      clearInterval(teamPollTimer);
      teamPollTimer = null;
    },

    async fetchTeamOverview(): Promise<void> {
      if (!isTauriAvailable()) {
        return;
      }
      this.loading = true;
      this.error = null;
      try {
        const [membersRes, tasksRes] = await Promise.all([
          safeInvoke<TeammateMember[]>("team_get_roster"),
          safeInvoke<TeamTaskItem[]>("team_get_tasks"),
        ]);
        if (Array.isArray(membersRes)) {
          this.members = membersRes;
        }
        if (Array.isArray(tasksRes)) {
          this.tasks = tasksRes;
        }
      } catch (err) {
        this.error = String(err);
      } finally {
        this.loading = false;
      }
    },

    async claimTask(taskId: string, expectedRevision: number, owner: string): Promise<boolean> {
      if (!isTauriAvailable()) return false;
      try {
        const updated = await safeInvoke<TeamTaskItem>("team_claim_task", {
          taskId,
          expectedRevision,
          owner,
        });
        if (updated) {
          const idx = this.tasks.findIndex((t) => t.task_id === taskId);
          if (idx !== -1) {
            this.tasks[idx] = updated;
          }
          return true;
        }
      } catch (err) {
        this.error = String(err);
      }
      return false;
    },

    async completeTask(taskId: string, expectedRevision: number): Promise<boolean> {
      if (!isTauriAvailable()) return false;
      try {
        const updated = await safeInvoke<TeamTaskItem>("team_complete_task", {
          taskId,
          expectedRevision,
        });
        if (updated) {
          const idx = this.tasks.findIndex((t) => t.task_id === taskId);
          if (idx !== -1) {
            this.tasks[idx] = updated;
          }
          return true;
        }
      } catch (err) {
        this.error = String(err);
      }
      return false;
    },
  },
});
