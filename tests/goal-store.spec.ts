import { describe, it, expect, beforeEach, vi } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { useGoalStore } from "@/stores/goal";
import * as tauriBridge from "@/lib/tauri";

describe("Goal Store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.restoreAllMocks();
  });

  it("initializes with default empty state", () => {
    const store = useGoalStore();
    expect(store.currentGoal).toBeNull();
    expect(store.hasGoal).toBe(false);
    expect(store.isActive).toBe(false);
  });

  it("fetches goal successfully via safeInvoke", async () => {
    vi.spyOn(tauriBridge, "isTauriAvailable").mockReturnValue(true);
    vi.spyOn(tauriBridge, "safeInvoke").mockResolvedValue({
      id: "goal-123",
      session_id: "session-1",
      objective: "Test Goal",
      phase: "active",
      activation: "armed",
      revision: 1,
      roundsStarted: 2,
      maxGoalRounds: 10,
    });

    const store = useGoalStore();
    const result = await store.fetchGoal("session-1");

    expect(result).not.toBeNull();
    expect(store.currentGoal?.id).toBe("goal-123");
    expect(store.isActive).toBe(true);
    expect(store.progressPercent).toBe(20);
  });

  it("computes getters correctly for paused and blocked phases", () => {
    const store = useGoalStore();
    store.currentGoal = {
      id: "goal-1",
      session_id: "session-1",
      objective: "Goal 1",
      phase: "blocked",
      activation: "disarmed",
      revision: 2,
      roundsStarted: 5,
      maxGoalRounds: 10,
      blocked_reason: "API quota exceeded",
    };

    expect(store.isBlocked).toBe(true);
    expect(store.isActive).toBe(false);
    expect(store.progressPercent).toBe(50);

    store.currentGoal.phase = "paused";
    expect(store.isPaused).toBe(true);
    expect(store.isBlocked).toBe(false);
  });
});
