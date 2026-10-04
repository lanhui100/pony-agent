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
 * Lead Scaffold Stub: deriveTrajectoryTimeline
 * Projects TurnTraceRecord array into a normalized timeline model.
 */
export function deriveTrajectoryTimeline(
  _turns: TurnTraceRecord[],
  _mode: "sequence" | "duration" | "time" = "sequence"
): TrajectoryTimelineModel {
  throw new Error("NotImplementedError: deriveTrajectoryTimeline");
}

/**
 * Lead Scaffold Stub: deriveTrajectoryRecords
 * Extracts flattened measurable cells from TurnTraceRecord array.
 */
export function deriveTrajectoryRecords(
  _turns: TurnTraceRecord[]
): TrajectoryRecordCell[] {
  throw new Error("NotImplementedError: deriveTrajectoryRecords");
}

/**
 * Lead Scaffold Stub: filterTrajectoryRecords
 * Filters cells by search query and optional status / kind.
 */
export function filterTrajectoryRecords(
  _cells: TrajectoryRecordCell[],
  _searchQuery: string
): TrajectoryRecordCell[] {
  throw new Error("NotImplementedError: filterTrajectoryRecords");
}
