import { describe, it, expect } from "vitest";
import {
  deriveTrajectoryTimeline,
  deriveTrajectoryRecords,
  filterTrajectoryRecords,
  type TrajectoryRecordCell,
} from "@/lib/runtime/trajectory-core";
import type { TurnTraceRecord } from "@/types/runtime";

describe("Trajectory Core Contract & Acceptance (Stage 2 Green Phase Verification)", () => {
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
          text: "Please read the config file",
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
          reasoningTokens: 25,
          cacheHitInputTokens: 10,
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

  describe("deriveTrajectoryTimeline", () => {
    it("verifies 3-lane assignment, spans, and turnBoundaries contract", () => {
      const timeline = deriveTrajectoryTimeline(mockTurns, "sequence");
      expect(timeline).toBeDefined();
      expect(timeline.turnBoundaries).toBeDefined();
      expect(timeline.turnBoundaries.length).toBe(2);
      expect(timeline.turnBoundaries[0].turnId).toBe("turn-1");
      expect(timeline.turnBoundaries[1].turnId).toBe("turn-2");

      expect(timeline.spans).toBeDefined();
      expect(timeline.spans.length).toBe(5);

      const inputSpan = timeline.spans.find((s) => s.id === "step-1-input");
      const contextSpan = timeline.spans.find((s) => s.id === "step-1-context");
      const modelSpan = timeline.spans.find((s) => s.id === "step-1-model");
      const toolSpan = timeline.spans.find((s) => s.id === "step-1-tool");
      const errorModelSpan = timeline.spans.find((s) => s.id === "step-2-model");

      // Lane 0: user/system/context, Lane 1: model/message, Lane 2: tool
      expect(inputSpan).toBeDefined();
      expect(inputSpan?.lane).toBe(0);
      expect(contextSpan?.lane).toBe(0);

      expect(modelSpan).toBeDefined();
      expect(modelSpan?.lane).toBe(1);

      expect(toolSpan).toBeDefined();
      expect(toolSpan?.lane).toBe(2);

      expect(errorModelSpan).toBeDefined();
      expect(errorModelSpan?.lane).toBe(1);
      expect(errorModelSpan?.isError).toBe(true);

      // Total metrics
      expect(timeline.totalDurationMs).toBeGreaterThan(0);
      expect(timeline.totalEnd).toBeGreaterThanOrEqual(timeline.totalStart);
    });

    it("supports duration mode and time mode calculations", () => {
      const durationTimeline = deriveTrajectoryTimeline(mockTurns, "duration");
      expect(durationTimeline.spans.length).toBe(5);
      const firstSpan = durationTimeline.spans[0];
      expect(firstSpan.end).toBeGreaterThan(firstSpan.start);
    });
  });

  describe("deriveTrajectoryRecords", () => {
    it("extracts flattened measurable cells, statuses, and token metrics", () => {
      const records = deriveTrajectoryRecords(mockTurns);
      expect(Array.isArray(records)).toBe(true);
      expect(records.length).toBe(5);

      const modelRecord = records.find((r) => r.id === "step-1-model");
      expect(modelRecord).toBeDefined();
      expect(modelRecord?.turnId).toBe("turn-1");
      expect(modelRecord?.kind).toBe("message");
      expect(modelRecord?.status).toBe("completed");
      expect(modelRecord?.durationMs).toBe(800);
      expect(modelRecord?.inputTokens).toBe(100);
      expect(modelRecord?.outputTokens).toBe(40);
      expect(modelRecord?.reasoningTokens).toBe(25);
      expect(modelRecord?.cacheHitTokens).toBe(10);
      expect(modelRecord?.isError).toBe(false);

      const errorRecord = records.find((r) => r.id === "step-2-model");
      expect(errorRecord).toBeDefined();
      expect(errorRecord?.turnId).toBe("turn-2");
      expect(errorRecord?.status).toBe("failed");
      expect(errorRecord?.isError).toBe(true);
    });

    it("extracts from multiple turns with distinct turns and sequence numbers", () => {
      const records = deriveTrajectoryRecords(mockTurns);
      const turn1Records = records.filter((r) => r.turnId === "turn-1");
      const turn2Records = records.filter((r) => r.turnId === "turn-2");
      expect(turn1Records.length).toBe(4);
      expect(turn2Records.length).toBe(1);
    });
  });

  describe("filterTrajectoryRecords", () => {
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
      {
        id: "cell-3",
        turnId: "turn-2",
        turnTitle: "Turn 2",
        index: 0,
        kind: "context",
        title: "Build Context",
        preview: "Loaded 3 system memories",
        status: "completed",
        durationMs: 100,
        isError: false,
      },
    ];

    it("returns all records when search query is empty or blank", () => {
      expect(filterTrajectoryRecords(dummyCells, "").length).toBe(3);
      expect(filterTrajectoryRecords(dummyCells, "   ").length).toBe(3);
    });

    it("filters records case-insensitively across title, preview, and kind", () => {
      const filteredByTitle = filterTrajectoryRecords(dummyCells, "COMMAND");
      expect(filteredByTitle.length).toBe(1);
      expect(filteredByTitle[0].id).toBe("cell-1");

      const filteredByPreview = filterTrajectoryRecords(dummyCells, "content");
      expect(filteredByPreview.length).toBe(1);
      expect(filteredByPreview[0].id).toBe("cell-2");

      const filteredByKind = filterTrajectoryRecords(dummyCells, "context");
      expect(filteredByKind.length).toBe(1);
      expect(filteredByKind[0].id).toBe("cell-3");
    });
  });
});
