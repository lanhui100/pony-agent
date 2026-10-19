import { defineStore } from "pinia";
import type { GoalData, CreateGoalPayload, UpdateGoalPayload } from "@/types/goal";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";

export const useGoalStore = defineStore("goal", {
  state: () => ({
    currentGoal: null as GoalData | null,
    loading: false,
    error: null as string | null,
    updating: false,
  }),

  getters: {
    hasGoal: (state) => state.currentGoal !== null,
    isBlocked: (state) => state.currentGoal?.phase === "blocked",
    isActive: (state) => state.currentGoal?.phase === "active",
    isPaused: (state) => state.currentGoal?.phase === "paused",
    isCompleted: (state) => state.currentGoal?.phase === "completed",
    progressPercent: (state) => {
      if (!state.currentGoal || state.currentGoal.maxGoalRounds <= 0) return 0;
      return Math.min(
        100,
        Math.round((state.currentGoal.roundsStarted / state.currentGoal.maxGoalRounds) * 100)
      );
    },
  },

  actions: {
    async fetchGoal(sessionId: string): Promise<GoalData | null> {
      if (!isTauriAvailable() || !sessionId?.trim()) {
        return null;
      }

      this.loading = true;
      this.error = null;
      try {
        const goal = await safeInvoke<GoalData | null>("goal_get", { sessionId });
        this.currentGoal = goal;
        return goal;
      } catch (err) {
        this.error = `获取目标失败: ${String(err)}`;
        return null;
      } finally {
        this.loading = false;
      }
    },

    async createGoal(payload: CreateGoalPayload): Promise<GoalData | null> {
      if (!isTauriAvailable()) return null;

      this.updating = true;
      this.error = null;
      try {
        const goal = await safeInvoke<GoalData>("goal_create", {
          sessionId: payload.sessionId || "default",
          objective: payload.objective,
          maxGoalRounds: payload.maxGoalRounds,
        });
        this.currentGoal = goal;
        return goal;
      } catch (err) {
        this.error = `创建目标失败: ${String(err)}`;
        return null;
      } finally {
        this.updating = false;
      }
    },

    async updateGoal(payload: UpdateGoalPayload): Promise<GoalData | null> {
      if (!isTauriAvailable()) return null;

      this.updating = true;
      this.error = null;
      try {
        const goal = await safeInvoke<GoalData>("goal_update", {
          sessionId: payload.sessionId || "default",
          goalId: payload.goalId,
          revision: payload.revision,
          action: payload.action,
          objective: payload.objective,
          blockedReason: payload.blockedReason,
          maxGoalRounds: payload.maxGoalRounds,
        });
        this.currentGoal = goal;
        return goal;
      } catch (err) {
        this.error = `更新目标失败: ${String(err)}`;
        return null;
      } finally {
        this.updating = false;
      }
    },

    async pause(sessionId: string): Promise<boolean> {
      if (!this.currentGoal) return false;
      const res = await this.updateGoal({
        sessionId,
        goalId: this.currentGoal.id,
        revision: this.currentGoal.revision,
        action: "pause",
      });
      return !!res;
    },

    async resume(sessionId: string): Promise<boolean> {
      if (!this.currentGoal) return false;
      const res = await this.updateGoal({
        sessionId,
        goalId: this.currentGoal.id,
        revision: this.currentGoal.revision,
        action: "resume",
      });
      return !!res;
    },

    async complete(sessionId: string): Promise<boolean> {
      if (!this.currentGoal) return false;
      const res = await this.updateGoal({
        sessionId,
        goalId: this.currentGoal.id,
        revision: this.currentGoal.revision,
        action: "complete",
      });
      return !!res;
    },
  },
});

// ===========================================================================
// Wave-1 冻结契约空桩（protocol-agent 签署；实现方见
// docs/architecture/plan-goal-contract.md C6）。仅签名 + throw，禁止在空桩内写实现。
// ===========================================================================

/**
 * C6：turn 结束（`turn:completed` / graph 流结束）或 `goal://changed` / `goal://round`
 * 事件到达后，以真实 sessionId 刷新目标条（`fetchGoal(sessionId)`）。
 */
export async function refreshForTurn(sessionId: string): Promise<void> {
  void sessionId; // 空桩期参数保留签名但未读取（noUnusedParameters）；实现时删除本行。
  throw new Error("NotImplemented: wave-1");
}
