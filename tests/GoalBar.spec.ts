import { describe, it, expect, beforeEach, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import GoalBar from "@/components/GoalBar.vue";
import { useGoalStore } from "@/stores/goal";

describe("GoalBar Component", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it("does not render when currentGoal is null", () => {
    const wrapper = mount(GoalBar, {
      props: { sessionId: "s1" },
    });
    expect(wrapper.find('[data-testid="goal-bar"]').exists()).toBe(false);
  });

  it("renders objective, phase badge, and rounds correctly", () => {
    const goalStore = useGoalStore();
    goalStore.currentGoal = {
      id: "goal-1",
      session_id: "s1",
      objective: "Refactor architecture",
      phase: "active",
      activation: "armed",
      revision: 1,
      roundsStarted: 3,
      maxGoalRounds: 10,
    };

    const wrapper = mount(GoalBar, {
      props: { sessionId: "s1" },
    });

    expect(wrapper.find('[data-testid="goal-bar"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("Refactor architecture");
    expect(wrapper.text()).toContain("活跃中");
    expect(wrapper.text()).toContain("轮次: 3 / 10");
  });

  it("renders blocked reason banner when phase is blocked", () => {
    const goalStore = useGoalStore();
    goalStore.currentGoal = {
      id: "goal-1",
      session_id: "s1",
      objective: "Refactor architecture",
      phase: "blocked",
      activation: "disarmed",
      revision: 2,
      roundsStarted: 3,
      maxGoalRounds: 10,
      blocked_reason: "Token quota depleted",
    };

    const wrapper = mount(GoalBar, {
      props: { sessionId: "s1" },
    });

    const banner = wrapper.find('[data-testid="goal-blocked-banner"]');
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("Token quota depleted");
  });
});
