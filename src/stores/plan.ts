import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type { Plan, PlanPayload, PlanStepSpec } from "@/types/ask-plan";

type PlanState = {
  plans: Plan[];
  loading: boolean;
  error: string | null;
  selectedPlanId: string | null;
  /** `${planId}:${stepId}` while a step completion is in flight. */
  completingStepKey: string | null;
};

export const usePlanStore = defineStore("plan", {
  state: (): PlanState => ({
    plans: [],
    loading: false,
    error: null,
    selectedPlanId: null,
    completingStepKey: null
  }),
  getters: {
    selectedPlan(state): Plan | null {
      return (
        state.plans.find((plan) => plan.planId === state.selectedPlanId) ??
        state.plans[0] ??
        null
      );
    },
    isCompletingStep(state) {
      return (planId: string, stepId: string) =>
        state.completingStepKey === `${planId}:${stepId}`;
    }
  },
  actions: {
    /** Every plan owned by a session, in creation order. */
    async list(sessionId: string): Promise<Plan[]> {
      if (!isTauriAvailable() || !sessionId.trim()) {
        return [];
      }

      this.loading = true;
      this.error = null;
      try {
        const plans = await safeInvoke<Plan[] | null>("plan_list", { sessionId });
        this.plans = Array.isArray(plans) ? plans : [];
        this.reconcileSelection();
        return this.plans;
      } catch (error) {
        this.error = `加载计划失败：${String(error)}`;
        return [];
      } finally {
        this.loading = false;
      }
    },

    /** Read a single plan by stable id. */
    async get(sessionId: string, planId: string): Promise<Plan | null> {
      if (!isTauriAvailable() || !sessionId.trim() || !planId.trim()) {
        return null;
      }

      this.error = null;
      try {
        const plan = await safeInvoke<Plan | null>("plan_get", { sessionId, planId });
        if (plan) {
          this.upsertPlan(plan);
          this.selectedPlanId = plan.planId;
        }
        return plan;
      } catch (error) {
        this.error = `读取计划失败：${String(error)}`;
        return null;
      }
    },

    /** Create a session-owned `Draft` plan. */
    async create(sessionId: string, payload: PlanPayload): Promise<Plan | null> {
      if (!isTauriAvailable() || !sessionId.trim()) {
        return null;
      }

      this.error = null;
      try {
        const plan = await safeInvoke<Plan | null>("plan_create", { sessionId, payload });
        if (plan) {
          this.upsertPlan(plan);
          this.selectedPlanId = plan.planId;
        }
        return plan;
      } catch (error) {
        this.error = `创建计划失败：${String(error)}`;
        return null;
      }
    },

    /** Replace a plan's content, preserving its `plan_id`, via compare-and-swap. */
    async replace(
      sessionId: string,
      planId: string,
      revision: number,
      payload: PlanPayload
    ): Promise<Plan | null> {
      if (!isTauriAvailable() || !sessionId.trim() || !planId.trim()) {
        return null;
      }

      this.error = null;
      try {
        const plan = await safeInvoke<Plan | null>("plan_replace", {
          sessionId,
          planId,
          revision,
          payload
        });
        if (plan) {
          this.upsertPlan(plan);
        }
        return plan;
      } catch (error) {
        this.error = `替换计划失败：${String(error)}`;
        return null;
      }
    },

    /** Append a single step to a plan via compare-and-swap. */
    async mergeStep(
      sessionId: string,
      planId: string,
      revision: number,
      step: PlanStepSpec
    ): Promise<Plan | null> {
      if (!isTauriAvailable() || !sessionId.trim() || !planId.trim()) {
        return null;
      }

      this.error = null;
      try {
        const plan = await safeInvoke<Plan | null>("plan_merge", {
          sessionId,
          planId,
          revision,
          step
        });
        if (plan) {
          this.upsertPlan(plan);
        }
        return plan;
      } catch (error) {
        this.error = `追加步骤失败：${String(error)}`;
        return null;
      }
    },

    /**
     * Mark a plan step completed. The compare-and-swap `revision` is the one
     * from the listed plan; the response is the updated plan (with a bumped
     * revision) which replaces the local copy so later mutations stay fresh.
     */
    async completeStep(
      sessionId: string,
      planId: string,
      revision: number,
      stepId: string
    ): Promise<Plan | null> {
      const stepKey = `${planId}:${stepId}`;
      if (this.completingStepKey) {
        return null;
      }
      if (!isTauriAvailable() || !sessionId.trim() || !planId.trim() || !stepId.trim()) {
        return null;
      }

      this.completingStepKey = stepKey;
      this.error = null;
      try {
        const plan = await safeInvoke<Plan | null>("plan_complete_step", {
          sessionId,
          planId,
          revision,
          stepId
        });
        if (plan) {
          this.upsertPlan(plan);
        }
        return plan;
      } catch (error) {
        this.error = `标记步骤完成失败：${String(error)}`;
        return null;
      } finally {
        this.completingStepKey = null;
      }
    },

    upsertPlan(plan: Plan): void {
      const index = this.plans.findIndex((item) => item.planId === plan.planId);
      if (index >= 0) {
        this.plans[index] = plan;
      } else {
        this.plans.push(plan);
      }
    },

    select(planId: string): void {
      if (this.plans.some((plan) => plan.planId === planId)) {
        this.selectedPlanId = planId;
      }
    },

    reconcileSelection(): void {
      if (
        this.selectedPlanId &&
        !this.plans.some((plan) => plan.planId === this.selectedPlanId)
      ) {
        this.selectedPlanId = null;
      }
      if (!this.selectedPlanId && this.plans.length > 0) {
        this.selectedPlanId = this.plans[0]!.planId;
      }
    },

    reset(): void {
      this.plans = [];
      this.loading = false;
      this.error = null;
      this.selectedPlanId = null;
      this.completingStepKey = null;
    }
  }
});
