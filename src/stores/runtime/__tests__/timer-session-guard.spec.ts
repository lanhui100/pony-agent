/**
 * B279 回归 · runtime store 定时器跨会话守卫
 *
 * 覆盖三处缺口（拆解 Wave 2 暴露）：
 * 1. trace 节流回调：定时器存活期内会话/轮次变更 ⇒ 旧 pending 不得落到新会话；
 * 2. stream flush 定时器：切会话后旧缓冲不得写进新会话；
 * 3. 会话落点 `applySessionSnapshot`：会话变化时清理旧会话定时器与待投递节流帧；
 * 4. rIC 延迟 timeline 恢复：旧会话回调晚于下次切换时必须被归属守卫拦下。
 *
 * 纪律：只用假定时器 + 固定时钟推进，禁止真实 sleep。
 */
import { describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

/** 可控的空闲调度队列：让 rIC 回调用例能把「延迟执行」显式搬到断言点。 */
const deferred = vi.hoisted(() => ({ work: [] as Array<() => void> }));

vi.mock("@/lib/runtime/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/runtime/utils")>();
  return {
    ...actual,
    waitForNextPaint: () => Promise.resolve(),
    runLowPriorityTurnWork: (callback: () => void) => {
      deferred.work.push(callback);
    }
  };
});

import { STREAM_FLUSH_INTERVAL_MS, TRACE_TIMELINE_THROTTLE_MS } from "@/lib/runtime/constants";
import { createDefaultTraceTimeline } from "@/lib/runtime/trace";
import type { SessionSnapshot, TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";
import { advance, BASE_TIME, seedStore, SESSION_ID, setupRuntimeHarness } from "./harness";

const harness = setupRuntimeHarness(tauriMocks);
const OTHER_SESSION_ID = "rt-session-other";

function emptySnapshot(conversationId: string): SessionSnapshot {
  return {
    conversationId,
    title: null,
    summary: "",
    history: [],
    attachmentAssets: [],
    turnTraceHistory: [],
    turnCount: 0,
    lastReferencedFile: null,
    updatedAtMs: BASE_TIME
  };
}

function timelineEntry(id: string): TraceTimelineEntry {
  return { id, kind: "input", label: id, state: "completed", sequence: 1 };
}

describe("B279 · 定时器跨会话守卫", () => {
  it("trace 节流：会话或轮次变更后旧 pending 不得落到新会话", async () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionId: SESSION_ID,
      activeTurnId: "turn-a",
      traceTimeline: createDefaultTraceTimeline()
    });

    store.scheduleThrottledTraceTimeline([timelineEntry("old-session-entry")]);
    expect(store.traceTimelineThrottleTimerId).not.toBeNull();

    // 模拟定时器存活期内发生会话切换
    seedStore(store, { sessionId: OTHER_SESSION_ID, activeTurnId: "turn-b" });
    await advance(TRACE_TIMELINE_THROTTLE_MS + 1);

    expect(store.traceTimeline.map((entry) => entry.id)).not.toContain("old-session-entry");
    expect(store.traceTimelineThrottleTimerId).toBeNull();
  });

  it("stream flush：切会话后旧缓冲不得写进新会话", async () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionId: SESSION_ID,
      activeTurnId: "turn-a",
      streamBufferTurnId: "turn-a",
      streamBufferText: "AAA"
    });

    store.scheduleStreamFlush("turn-a");
    expect(store.streamFlushTimerId).not.toBeNull();

    seedStore(store, { sessionId: OTHER_SESSION_ID, activeTurnId: "turn-b" });
    await advance(STREAM_FLUSH_INTERVAL_MS + 1);

    expect(store.streamBufferText).toBe("AAA");
    expect(store.streamFlushTimerId).toBeNull();
  });

  it("applySessionSnapshot：会话变化时清理定时器与待投递节流帧", () => {
    const store = harness.getStore();
    seedStore(store, {
      sessionId: SESSION_ID,
      activeTurnId: "turn-a",
      streamBufferTurnId: "turn-a",
      streamBufferText: "AAA"
    });

    store.scheduleStreamFlush("turn-a");
    store.scheduleThrottledTraceTimeline([timelineEntry("old-session-entry")]);
    expect(store.streamFlushTimerId).not.toBeNull();
    expect(store.traceTimelineThrottleTimerId).not.toBeNull();

    store.applySessionSnapshot(OTHER_SESSION_ID, emptySnapshot(OTHER_SESSION_ID), null, null, {
      preserveMessages: true
    });

    expect(store.sessionId).toBe(OTHER_SESSION_ID);
    expect(store.streamFlushTimerId).toBeNull();
    expect(store.traceTimelineThrottleTimerId).toBeNull();
    expect(store.pendingThrottledTraceTimeline).toBeNull();
    // 缓冲不清：由目标会话的后台轮恢复分支负责重建（本用例无后台轮 ⇒ 保持原值不被 flush）
    expect(store.streamBufferText).toBe("AAA");
  });

  it("rIC 延迟 timeline 恢复：旧会话回调在切换后不得覆盖新会话", () => {
    const store = harness.getStore();
    deferred.work.length = 0;

    const oldTrace: TurnTraceRecord = {
      turnId: "turn-old",
      title: "旧轮",
      phase: "idle",
      traceSteps: [],
      toolActivities: [],
      traceTimeline: [timelineEntry("old-session-entry")],
      updatedAt: BASE_TIME
    };

    store.applySessionSnapshot(
      SESSION_ID,
      { ...emptySnapshot(SESSION_ID), turnTraceHistory: [oldTrace] },
      null,
      null,
      { preserveMessages: true }
    );
    expect(deferred.work.length).toBeGreaterThan(0);

    store.applySessionSnapshot(OTHER_SESSION_ID, emptySnapshot(OTHER_SESSION_ID), null, null, {
      preserveMessages: true
    });
    const beforeLateWork = store.traceTimeline.map((entry) => entry.id);

    // 晚到的旧会话空闲回调
    for (const work of deferred.work) {
      work();
    }

    expect(store.traceTimeline.map((entry) => entry.id)).toEqual(beforeLateWork);
    expect(store.traceTimeline.map((entry) => entry.id)).not.toContain("old-session-entry");
  });
});
