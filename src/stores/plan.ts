import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type { PlanModeState, TodoItem } from "@/types/ask-plan";

type PlanStoreState = {
  /** 会话级 Plan 模式投影（模式开关 + 方案正文，无步骤清单）。 */
  mode: PlanModeState;
  todos: TodoItem[];
  loading: boolean;
  todosLoading: boolean;
  error: string | null;
};

/**
 * Plan 模式 store（ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count）。
 *
 * Plan 是**会话级协作模式**，不是任务清单：这里只有模式开关与一份 Markdown 方案正文。
 * 步骤进度归 `todos`（TodoWrite），团队分工归 team_task_*，长程目标归 goal store。
 */
export const usePlanStore = defineStore("plan", {
  state: (): PlanStoreState => ({
    mode: {
      sessionId: "",
      active: false,
      plan: null,
      planApproved: false,
      revision: 0
    },
    todos: [],
    loading: false,
    todosLoading: false,
    error: null
  }),
  getters: {
    /** 模式是否生效（含未 commit 的 pending 意图）。 */
    isPlanModeActive(state): boolean {
      return state.mode.active;
    },
    /** 方案正文（未提交时为空串，便于模板直接渲染）。 */
    planBody(state): string {
      return state.mode.plan ?? "";
    },
    /** 是否有待审方案（激活 + 已提交 + 未批准）。 */
    hasPendingPlan(state): boolean {
      return state.mode.active && Boolean(state.mode.plan) && !state.mode.planApproved;
    },
    /**
     * Todo 任务进度（**独立计数**，不与 Plan 混加）。
     *
     * 旧实现把 `plans.length + todos.length` 混在一起当角标，两个量纲相加没有意义；
     * 这里只回答「本会话的待办做到第几条」。
     */
    todoProgress(state): { completed: number; inProgress: number; pending: number; total: number } {
      const completed = state.todos.filter((t) => t.status === "completed").length;
      const inProgress = state.todos.filter((t) => t.status === "in_progress").length;
      const pending = state.todos.filter((t) => t.status === "pending").length;
      return { completed, inProgress, pending, total: state.todos.length };
    },
    /** `x/x` 形式的任务进度文案；没有任务时为空串（调用方据此隐藏角标）。 */
    todoProgressLabel(): string {
      const { completed, total } = this.todoProgress;
      return total === 0 ? "" : `${completed}/${total}`;
    }
  },
  actions: {
    /** 读取会话的 Plan 模式状态。 */
    async fetchMode(sessionId: string): Promise<PlanModeState> {
      if (!isTauriAvailable() || !sessionId.trim()) {
        return this.mode;
      }

      this.loading = true;
      this.error = null;
      try {
        const mode = await safeInvoke<PlanModeState | null>("plan_mode_get", { sessionId });
        this.mode = normalizeMode(mode, sessionId);
        return this.mode;
      } catch (error) {
        this.error = `加载 Plan 模式失败：${String(error)}`;
        return this.mode;
      } finally {
        this.loading = false;
      }
    },

    /** 进入（`active = true`）/ 退出（`active = false`）Plan 模式。 */
    async setMode(sessionId: string, active: boolean): Promise<PlanModeState> {
      if (!isTauriAvailable() || !sessionId.trim()) {
        return this.mode;
      }

      this.error = null;
      try {
        const mode = await safeInvoke<PlanModeState | null>("plan_mode_set", {
          sessionId,
          active
        });
        this.mode = normalizeMode(mode, sessionId);
        return this.mode;
      } catch (error) {
        this.error = `切换 Plan 模式失败：${String(error)}`;
        return this.mode;
      }
    },

    /** 把上一轮的切换意图落地（宿主在每次 pre-step 前调用；幂等）。 */
    async commitMode(sessionId: string): Promise<PlanModeState> {
      if (!isTauriAvailable() || !sessionId.trim()) {
        return this.mode;
      }
      try {
        const mode = await safeInvoke<PlanModeState | null>("plan_mode_commit", { sessionId });
        this.mode = normalizeMode(mode, sessionId);
      } catch (error) {
        console.warn("[plan-mode] commit failed:", error);
      }
      return this.mode;
    },

    /**
     * 方案审核决定：`approved = true` 退出模式并解锁写/执行；
     * `false` 表示「继续规划」，模式保持激活。
     */
    async reviewPlan(sessionId: string, approved: boolean): Promise<PlanModeState> {
      if (!isTauriAvailable() || !sessionId.trim()) {
        return this.mode;
      }

      this.error = null;
      try {
        const mode = await safeInvoke<PlanModeState | null>("plan_mode_review", {
          sessionId,
          approved
        });
        this.mode = normalizeMode(mode, sessionId);
        return this.mode;
      } catch (error) {
        this.error = `方案审核失败：${String(error)}`;
        return this.mode;
      }
    },

    /** List todos for a session. */
    async listTodos(sessionId: string): Promise<TodoItem[]> {
      if (!isTauriAvailable() || !sessionId.trim()) {
        return [];
      }

      this.todosLoading = true;
      try {
        const todos = await safeInvoke<TodoItem[] | null>("todo_list", { sessionId });
        this.todos = Array.isArray(todos) ? todos : [];
        return this.todos;
      } catch (error) {
        console.warn("Failed to load todos for session:", error);
        return [];
      } finally {
        this.todosLoading = false;
      }
    },

    reset(): void {
      this.mode = {
        sessionId: "",
        active: false,
        plan: null,
        planApproved: false,
        revision: 0
      };
      this.todos = [];
      this.loading = false;
      this.todosLoading = false;
      this.error = null;
    }
  }
});

/** 把 IPC 返回的空值/缺字段补成完整投影（前端不因后端少键而崩）。 */
function normalizeMode(mode: PlanModeState | null, sessionId: string): PlanModeState {
  if (!mode || typeof mode !== "object") {
    return { sessionId, active: false, plan: null, planApproved: false, revision: 0 };
  }
  return {
    sessionId: mode.sessionId || sessionId,
    active: Boolean(mode.active),
    plan: mode.plan ?? null,
    planApproved: Boolean(mode.planApproved),
    revision: Number(mode.revision ?? 0),
    pending: mode.pending ?? null,
    committedActive: mode.committedActive,
    policy: mode.policy,
    reviewOptions: mode.reviewOptions
  };
}

// ===========================================================================
// Wave-1 C6：turn 结束后的面板刷新（docs/architecture/plan-goal-contract.md §6）
// ===========================================================================

/**
 * C6：turn 结束（`turn:completed` / graph 流结束）后，以真实 sessionId 刷新 Plan 模式与待办。
 *
 * 契约约束：
 * - 以真实 sessionId 为键，禁止 `"default"` 兜底；
 * - 空 sessionId 直接返回，不发起任何 IPC；
 * - IPC 异常必须吞掉（store action 内已降级为空），不得冒泡到 turn 结束路径。
 */
export async function refreshForTurn(sessionId: string): Promise<void> {
  const normalizedSessionId = sessionId?.trim() ?? "";
  if (!normalizedSessionId) {
    return;
  }

  const planStore = usePlanStore();
  try {
    await Promise.all([
      planStore.fetchMode(normalizedSessionId),
      planStore.listTodos(normalizedSessionId)
    ]);
  } catch (error) {
    // turn 结束路径禁止因面板刷新失败而中断主流程。
    console.warn("[wave-1] plan refreshForTurn failed:", error);
  }
}
