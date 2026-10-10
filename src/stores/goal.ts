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
