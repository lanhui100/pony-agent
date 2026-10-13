import { describe, expect, it } from "vitest";
import {
  isGraphTerminalPhase,
  resolveGraphRunSubmissionFromCheckpoint,
  resolveGraphRunSubmissionFromPlan,
  resolveGraphRunSubmissionFromRunState
} from "@/lib/runtime/phases";
import type { ExecutionCheckpoint } from "@/types/runtime";

describe("Graph Run Submission Resolution", () => {
  it("resolves to start_graph_run_stream when checkpoint is in terminal phase", () => {
    const completedCheckpoint: ExecutionCheckpoint = {
      checkpointKind: "recovery",
      phase: "completed",
      status: "completed",
      runId: "run-test-completed",
      turnId: "turn-1",
      sessionId: "session-1",
      resumable: false,
      replayable: true,
      submissionCommand: null
    };

    const submission = resolveGraphRunSubmissionFromCheckpoint(completedCheckpoint);
    expect(submission).toEqual({
      command: "start_graph_run_stream",
      runId: null
    });
  });

  it("resolves to start_graph_run_stream when checkpoint is failed or cancelled", () => {
    const failedCheckpoint: ExecutionCheckpoint = {
      checkpointKind: "recovery",
      phase: "failed",
      status: "failed",
      runId: "run-test-failed",
      turnId: "turn-1",
      sessionId: "session-1",
      resumable: false,
      replayable: true,
      submissionCommand: null
    };

    const submission = resolveGraphRunSubmissionFromCheckpoint(failedCheckpoint);
    expect(submission).toEqual({
      command: "start_graph_run_stream",
      runId: null
    });
  });

  it("resolves to continue_graph_run_stream only when checkpoint is non-terminal (ready or waiting_user)", () => {
    const readyCheckpoint: ExecutionCheckpoint = {
      checkpointKind: "recovery",
      phase: "ready",
      status: "ready",
      runId: "run-test-ready",
      turnId: "turn-1",
      sessionId: "session-1",
      resumable: false,
      replayable: false,
      submissionCommand: null
    };

    const submission = resolveGraphRunSubmissionFromCheckpoint(readyCheckpoint);
    expect(submission).toEqual({
      command: "continue_graph_run_stream",
      runId: "run-test-ready"
    });
  });

  it("resolves to start_graph_run_stream from runState when phase is terminal", () => {
    const submission = resolveGraphRunSubmissionFromRunState({
      runId: "run-test-terminal",
      phase: "completed"
    });
    expect(submission).toEqual({
      command: "start_graph_run_stream",
      runId: null
    });
  });
});
