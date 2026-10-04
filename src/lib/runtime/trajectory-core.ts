/**
 * Trajectory Core Contract and Projection Types
 * Inspired by deepseek-harness ui-trajectory architecture.
 */

import type { TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";

export type TrajectoryCellKind =
  | "system"
  | "user"
  | "context"
  | "compacted"
  | "message"
  | "tool"
  | "subtool";

export interface TrajectoryTimelineSpan {
  id: string;
  turnId: string;
  index: number;
  kind: TrajectoryCellKind;
  label: string;
  lane: number; // 0: user/system/context, 1: model/message, 2: tool
  start: number;
  end: number;
  durationMs: number;
  isError: boolean;
}

export interface TrajectoryTimelineTurnBoundary {
  turnId: string;
  title: string;
  time: number;
}

export interface TrajectoryTimelineModel {
  spans: TrajectoryTimelineSpan[];
  turnBoundaries: TrajectoryTimelineTurnBoundary[];
  totalStart: number;
  totalEnd: number;
  totalDurationMs: number;
}

export interface TrajectoryRecordCell {
  id: string;
  turnId: string;
  turnTitle: string;
  index: number;
  kind: TrajectoryCellKind;
  title: string;
  preview: string;
  status: "pending" | "running" | "completed" | "failed";
  durationMs: number;
  startedAt?: number;
  inputTokens?: number;
  outputTokens?: number;
  reasoningTokens?: number;
  cacheHitTokens?: number;
  isError: boolean;
  rawEntry?: TraceTimelineEntry;
}

export interface TrajectoryFilterState {
  searchQuery: string;
  actualDuration: boolean;
  actualTime: boolean;
  allTurnsCollapsed: boolean;
  allCallsCollapsed: boolean;
  selectedSpanId: string | null;
}

/**
 * Maps a trace timeline entry kind or fallback to TrajectoryCellKind
 */
export function mapEntryKindToTrajectoryKind(kind: string): TrajectoryCellKind {
  const normalized = kind.toLowerCase();
  if (normalized === "input" || normalized === "user") {
    return "user";
  }
  if (
    normalized === "prepare_retrieval" ||
    normalized === "build_context" ||
    normalized === "context"
  ) {
    return "context";
  }
  if (normalized === "system") {
    return "system";
  }
  if (normalized === "compacted") {
    return "compacted";
  }
  if (
    normalized === "call_model" ||
    normalized === "model" ||
    normalized === "message"
  ) {
    return "message";
  }
  if (normalized === "subtool") {
    return "subtool";
  }
  if (
    normalized === "call_tool" ||
    normalized === "tool" ||
    normalized === "return_result" ||
    normalized === "return"
  ) {
    return "tool";
  }
  return "context";
}

/**
 * Lane mapping:
 * Lane 0: input / user / system / context / checkpoint / build_context
 * Lane 1: call_model / message / compacted
 * Lane 2: call_tool / tool / subtool
 */
export function mapTrajectoryKindToLane(kind: TrajectoryCellKind): number {
  switch (kind) {
    case "user":
    case "system":
    case "context":
      return 0;
    case "message":
    case "compacted":
      return 1;
    case "tool":
    case "subtool":
      return 2;
    default:
      return 0;
  }
}

function mapTraceStepStateToStatus(
  state?: string,
  isTurnFailed?: boolean
): "pending" | "running" | "completed" | "failed" {
  if (state === "error") return "failed";
  if (state === "running") return "running";
  if (state === "pending") return "pending";
  if (state === "done") return "completed";
  if (isTurnFailed) return "failed";
  return "completed";
}

/**
 * deriveTrajectoryTimeline
 * Projects TurnTraceRecord array into a normalized timeline model with 3 lanes and boundaries.
 */
export function deriveTrajectoryTimeline(
  turns: TurnTraceRecord[],
  mode: "sequence" | "duration" | "time" = "sequence"
): TrajectoryTimelineModel {
  const spans: TrajectoryTimelineSpan[] = [];
  const turnBoundaries: TrajectoryTimelineTurnBoundary[] = [];

  let currentTime = 0;
  let globalIndex = 0;

  for (const turn of turns) {
    const turnStartTime = currentTime;
    turnBoundaries.push({
      turnId: turn.turnId,
      title: turn.title || `Turn ${turn.turnId}`,
      time: turnStartTime,
    });

    const entries = turn.traceTimeline || [];
    for (const entry of entries) {
      const trajKind = mapEntryKindToTrajectoryKind(entry.kind);
      const lane = mapTrajectoryKindToLane(trajKind);
      const isError =
        entry.state === "error" ||
        Boolean(entry.error) ||
        (turn.phase === "failed" && entry === entries[entries.length - 1]);

      let duration = 0;
      if (mode === "sequence") {
        duration = 1;
      } else {
        duration = Math.max(1, entry.durationMs || 10);
      }

      const start = currentTime;
      const end = start + duration;
      currentTime = end;

      spans.push({
        id: entry.id,
        turnId: turn.turnId,
        index: globalIndex++,
        kind: trajKind,
        label: entry.label || entry.kind,
        lane,
        start,
        end,
        durationMs: entry.durationMs || (mode === "sequence" ? 0 : duration),
        isError,
      });
    }
  }

  const totalStart = spans.length > 0 ? spans[0].start : 0;
  const totalEnd = spans.length > 0 ? currentTime : 0;
  const totalDurationMs = spans.reduce((sum, s) => sum + s.durationMs, 0);

  return {
    spans,
    turnBoundaries,
    totalStart,
    totalEnd,
    totalDurationMs,
  };
}

/**
 * deriveTrajectoryRecords
 * Extracts flattened measurable cells from TurnTraceRecord array.
 * Falls back to traceSteps and toolActivities when traceTimeline is absent or empty.
 */
export function deriveTrajectoryRecords(
  turns: TurnTraceRecord[]
): TrajectoryRecordCell[] {
  const cells: TrajectoryRecordCell[] = [];
  let globalIndex = 0;

  for (const turn of turns) {
    const entries = turn.traceTimeline || [];
    if (entries.length > 0) {
      for (const entry of entries) {
        const trajKind = mapEntryKindToTrajectoryKind(entry.kind);
        const isError =
          entry.state === "error" ||
          Boolean(entry.error) ||
          (turn.phase === "failed" && entry === entries[entries.length - 1]);
        const status = mapTraceStepStateToStatus(entry.state, isError);

        let preview = "";
        if (entry.text) {
          preview = entry.text;
        } else if (entry.reasoningContent) {
          preview = entry.reasoningContent;
        } else if (entry.error) {
          preview = entry.error;
        } else if (entry.buildContextObservation) {
          const obs = entry.buildContextObservation as unknown as Record<string, unknown>;
          preview = `Context: ${obs.injectedTokens ?? obs.requestFormat ?? 0}`;
        } else if (entry.toolActivities && entry.toolActivities.length > 0) {
          preview = entry.toolActivities.map((a) => (a as unknown as Record<string, unknown>).toolName || a.name || "").join(", ");
        } else {
          preview = entry.label || entry.kind;
        }

        cells.push({
          id: entry.id,
          turnId: turn.turnId,
          turnTitle: turn.title || `Turn ${turn.turnId}`,
          index: globalIndex++,
          kind: trajKind,
          title: entry.label || entry.kind,
          preview,
          status,
          durationMs: entry.durationMs || 0,
          inputTokens: entry.inputTokens ?? undefined,
          outputTokens: entry.outputTokens ?? undefined,
          reasoningTokens: entry.reasoningTokens ?? undefined,
          cacheHitTokens: entry.cacheHitInputTokens ?? undefined,
          isError,
          rawEntry: entry,
        });
      }
    } else {
      // Fallback: use traceSteps and toolActivities if traceTimeline is empty
      if (turn.traceSteps && turn.traceSteps.length > 0) {
        for (const step of turn.traceSteps) {
          const isError = step.state === "error" || Boolean((step as unknown as Record<string, unknown>).error);
          const status = mapTraceStepStateToStatus(step.state, isError);
          const stepObj = step as unknown as Record<string, unknown>;
          cells.push({
            id: step.id,
            turnId: turn.turnId,
            turnTitle: turn.title || `Turn ${turn.turnId}`,
            index: globalIndex++,
            kind: "context",
            title: step.label || "Step",
            preview: (stepObj.detail as string) || step.label || "",
            status,
            durationMs: 0,
            isError,
          });
        }
      }

      if (turn.toolActivities && turn.toolActivities.length > 0) {
        for (const tool of turn.toolActivities) {
          const isError = tool.status === "error";
          const status =
            tool.status === "done"
              ? "completed"
              : tool.status === "error"
              ? "failed"
              : tool.status === "running"
              ? "running"
              : "pending";
          const toolObj = tool as unknown as Record<string, unknown>;
          cells.push({
            id: tool.id,
            turnId: turn.turnId,
            turnTitle: turn.title || `Turn ${turn.turnId}`,
            index: globalIndex++,
            kind: "tool",
            title: tool.name || (toolObj.toolName as string) || "Tool",
            preview: tool.description || (toolObj.inputPreview as string) || "",
            status,
            durationMs: (toolObj.durationMs as number) || (tool.durationSeconds ? Math.round(tool.durationSeconds * 1000) : 0),
            isError,
          });
        }
      }
    }
  }

  return cells;
}

/**
 * filterTrajectoryRecords
 * Filters cells by search query across multiple fields (case-insensitive).
 */
export function filterTrajectoryRecords(
  cells: TrajectoryRecordCell[],
  searchQuery: string
): TrajectoryRecordCell[] {
  const query = searchQuery.trim().toLowerCase();
  if (!query) {
    return cells;
  }

  return cells.filter((cell) => {
    if (cell.title && cell.title.toLowerCase().includes(query)) return true;
    if (cell.preview && cell.preview.toLowerCase().includes(query)) return true;
    if (cell.turnTitle && cell.turnTitle.toLowerCase().includes(query)) return true;
    if (cell.kind && cell.kind.toLowerCase().includes(query)) return true;
    if (cell.rawEntry) {
      if (cell.rawEntry.error && cell.rawEntry.error.toLowerCase().includes(query))
        return true;
      if (
        cell.rawEntry.providerModel &&
        cell.rawEntry.providerModel.toLowerCase().includes(query)
      )
        return true;
      if (
        cell.rawEntry.text &&
        cell.rawEntry.text.toLowerCase().includes(query)
      )
        return true;
    }
    return false;
  });
}
