import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const tauriMocks = vi.hoisted(() => ({
  isTauriAvailable: vi.fn(() => true),
  safeInvoke: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  isTauriAvailable: tauriMocks.isTauriAvailable,
  safeInvoke: tauriMocks.safeInvoke
}));

import { usePlanStore } from "@/stores/plan";

const modeProjection = (overrides: Record<string, unknown> = {}) => ({
  sessionId: "sess-1",
  active: false,
  plan: null,
  planApproved: false,
  revision: 0,
  ...overrides
});

describe("plan store（Plan 模式 + 独立任务进度）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    tauriMocks.isTauriAvailable.mockReturnValue(true);
    tauriMocks.safeInvoke.mockReset();
  });

  it("通过 plan_mode_get 读取模式状态并落到 store", async () => {
    tauriMocks.safeInvoke.mockResolvedValue(
      modeProjection({ active: true, plan: "# 方案", revision: 3 })
    );

    const store = usePlanStore();
    const mode = await store.fetchMode("sess-1");

    expect(tauriMocks.safeInvoke).toHaveBeenCalledWith("plan_mode_get", { sessionId: "sess-1" });
    expect(mode.active).toBe(true);
    expect(store.mode.plan).toBe("# 方案");
    expect(store.mode.revision).toBe(3);
  });

  it("宿主返回 null 时降级为「未激活」，不抛错", async () => {
    tauriMocks.safeInvoke.mockResolvedValue(null);

    const store = usePlanStore();
    const mode = await store.fetchMode("sess-1");

    expect(mode.active).toBe(false);
    expect(mode.plan).toBeNull();
    expect(store.error).toBeNull();
  });

  it("浏览器预览或空会话不发起 IPC", async () => {
    tauriMocks.isTauriAvailable.mockReturnValue(false);
    const store = usePlanStore();
    await store.fetchMode("sess-1");
    expect(tauriMocks.safeInvoke).not.toHaveBeenCalled();

    tauriMocks.isTauriAvailable.mockReturnValue(true);
    await store.fetchMode("   ");
    expect(tauriMocks.safeInvoke).not.toHaveBeenCalled();
  });

  it("plan_mode_set 进入 / 退出模式", async () => {
    tauriMocks.safeInvoke.mockResolvedValue(modeProjection({ active: true }));
    const store = usePlanStore();
    await store.setMode("sess-1", true);
    expect(tauriMocks.safeInvoke).toHaveBeenCalledWith("plan_mode_set", {
      sessionId: "sess-1",
      active: true
    });
    expect(store.mode.active).toBe(true);

    tauriMocks.safeInvoke.mockResolvedValue(modeProjection({ active: false, plan: "# 方案" }));
    await store.setMode("sess-1", false);
    expect(store.mode.active).toBe(false);
  });

  it("plan_mode_review 批准后退出模式并标记方案已批准", async () => {
    tauriMocks.safeInvoke.mockResolvedValue(
      modeProjection({ active: false, plan: "# 方案", planApproved: true })
    );
    const store = usePlanStore();

    const mode = await store.reviewPlan("sess-1", true);

    expect(tauriMocks.safeInvoke).toHaveBeenCalledWith("plan_mode_review", {
      sessionId: "sess-1",
      approved: true
    });
    expect(mode.planApproved).toBe(true);
    expect(mode.active).toBe(false);
  });

  it("IPC 失败时记录错误且状态保持可用（不抛错）", async () => {
    tauriMocks.safeInvoke.mockRejectedValue(new Error("ipc unavailable"));
    const store = usePlanStore();

    await expect(store.fetchMode("sess-1")).resolves.toBeDefined();
    expect(store.error).toContain("加载 Plan 模式失败");
    expect(store.mode.active).toBe(false);
  });

  it("任务进度独立计数（不与 Plan 混加）", async () => {
    tauriMocks.safeInvoke.mockResolvedValue([
      { content: "a", status: "completed" },
      { content: "b", status: "in_progress" },
      { content: "c", status: "pending" }
    ]);
    const store = usePlanStore();
    await store.listTodos("sess-1");

    expect(store.todoProgress).toEqual({ completed: 1, inProgress: 1, pending: 1, total: 3 });
    expect(store.todoProgressLabel).toBe("1/3");
  });

  it("没有任务时进度文案为空串（调用方据此隐藏角标）", () => {
    const store = usePlanStore();
    expect(store.todoProgress.total).toBe(0);
    expect(store.todoProgressLabel).toBe("");
  });

  it("reset 清空模式与任务（切会话不残留）", async () => {
    tauriMocks.safeInvoke.mockResolvedValue(
      modeProjection({ active: true, plan: "# 方案" })
    );
    const store = usePlanStore();
    await store.fetchMode("sess-1");
    store.todos = [{ content: "a", status: "pending" }];

    store.reset();

    expect(store.mode.active).toBe(false);
    expect(store.mode.plan).toBeNull();
    expect(store.todos).toEqual([]);
  });
});
