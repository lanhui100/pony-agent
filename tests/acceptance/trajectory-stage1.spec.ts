import { describe, it, expect } from "vitest";
import {
  deriveTrajectoryTimeline,
  deriveTrajectoryRecords,
  filterTrajectoryRecords,
  type TrajectoryRecordCell,
} from "@/lib/runtime/trajectory-core";
import type { TurnTraceRecord } from "@/types/runtime";

describe("Trajectory Core Contract & Acceptance (Stage 1)", () => {
  const mockTurns: TurnTraceRecord[] = [
    {
      turnId: "turn-1",
      title: "User Prompt 1",
      phase: "completed",
      updatedAt: 1000,
      traceSteps: [],
      toolActivities: [],
      traceTimeline: [
        {
          id: "step-1-input",
          kind: "input",
          label: "User Input",
          state: "done",
          sequence: 1,
          durationMs: 50,
        },
        {
          id: "step-1-context",
          kind: "build_context",
          label: "Build Context",
          state: "done",
          sequence: 2,
          durationMs: 120,
        },
        {
          id: "step-1-model",
          kind: "call_model",
          label: "DeepSeek Chat",
          state: "done",
          sequence: 3,
          durationMs: 800,
          inputTokens: 100,
          outputTokens: 40,
        },
        {
          id: "step-1-tool",
          kind: "call_tool",
          label: "read_file",
          state: "done",
          sequence: 4,
          durationMs: 300,
        },
      ],
    },
    {
      turnId: "turn-2",
      title: "User Prompt 2",
      phase: "failed",
      updatedAt: 2500,
      traceSteps: [],
      toolActivities: [],
      traceTimeline: [
        {
          id: "step-2-model",
          kind: "call_model",
          label: "DeepSeek Reasoner",
          state: "error",
          sequence: 1,
          durationMs: 400,
          error: "API timeout",
        },
      ],
    },
  ];

  describe("deriveTrajectoryTimeline (Red Phase Acceptance)", () => {
    it("throws NotImplementedError prior to implementation", () => {
      expect(() => deriveTrajectoryTimeline(mockTurns, "sequence")).toThrowError(
        "NotImplementedError: deriveTrajectoryTimeline"
      );
    });

    it("verifies 3-lane assignment, spans, and turnBoundaries contract", () => {
      // In Red Phase, calling this directly must fail with NotImplementedError
      // Once implemented by UI Executor, it will verify the contract:
      const timeline = deriveTrajectoryTimeline(mockTurns, "sequence");
      expect(timeline.turnBoundaries).toBeDefined();
      expect(timeline.turnBoundaries.length).toBe(2);
      expect(timeline.spans).toBeDefined();

      const inputSpan = timeline.spans.find((s) => s.id === "step-1-input");
      const contextSpan = timeline.spans.find((s) => s.id === "step-1-context");
      const modelSpan = timeline.spans.find((s) => s.id === "step-1-model");
      const toolSpan = timeline.spans.find((s) => s.id === "step-1-tool");

      // Lane 0: user/system/context, Lane 1: model/message, Lane 2: tool
      expect(inputSpan?.lane).toBe(0);
      expect(contextSpan?.lane).toBe(0);
      expect(modelSpan?.lane).toBe(1);
      expect(toolSpan?.lane).toBe(2);
    });
  });

  describe("deriveTrajectoryRecords (Red Phase Acceptance)", () => {
    it("throws NotImplementedError prior to implementation", () => {
      expect(() => deriveTrajectoryRecords(mockTurns)).toThrowError(
        "NotImplementedError: deriveTrajectoryRecords"
      );
    });

    it("verifies flattened cells, status, and token metrics extraction contract", () => {
      const records = deriveTrajectoryRecords(mockTurns);
      expect(Array.isArray(records)).toBe(true);
      expect(records.length).toBeGreaterThan(0);
      const first = records[0];
      expect(first).toHaveProperty("id");
      expect(first).toHaveProperty("turnId");
      expect(first).toHaveProperty("kind");
      expect(first).toHaveProperty("status");
      expect(first).toHaveProperty("durationMs");
    });
  });

  describe("filterTrajectoryRecords (Red Phase Acceptance)", () => {
    const dummyCells: TrajectoryRecordCell[] = [
      {
        id: "cell-1",
        turnId: "turn-1",
        turnTitle: "Turn 1",
        index: 0,
        kind: "tool",
        title: "Execute_Command",
        preview: "ls -la workspace",
        status: "completed",
        durationMs: 200,
        isError: false,
      },
      {
        id: "cell-2",
        turnId: "turn-1",
        turnTitle: "Turn 1",
        index: 1,
        kind: "message",
        title: "Assistant Response",
        preview: "Here is your file content",
        status: "completed",
        durationMs: 500,
        isError: false,
      },
    ];

    it("throws NotImplementedError prior to implementation", () => {
      expect(() => filterTrajectoryRecords(dummyCells, "read")).toThrowError(
        "NotImplementedError: filterTrajectoryRecords"
      );
    });

    it("verifies case-insensitive filtering by title or preview contract", () => {
      const filteredByTitle = filterTrajectoryRecords(dummyCells, "command");
      expect(filteredByTitle.length).toBe(1);
      expect(filteredByTitle[0].id).toBe("cell-1");

      const filteredByPreview = filterTrajectoryRecords(dummyCells, "CONTENT");
      expect(filteredByPreview.length).toBe(1);
      expect(filteredByPreview[0].id).toBe("cell-2");
    });
  });
});
