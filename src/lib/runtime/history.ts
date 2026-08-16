// history domain：历史状态（node/branch/cursor）克隆、checkout/restore/fork/switch 结果归一化、
// 会话检查点条目构建。依赖 trace（turn trace 归一化）。
import type {
  ConversationCheckpointEntry,
  HistoryBranch,
  HistoryBranchSwitchResult,
  HistoryCheckoutMode,
  HistoryCheckoutResult,
  HistoryCursorMode,
  HistoryCursorState,
  HistoryForkResult,
  HistoryNode,
  HistoryRestoreResult,
  HistoryStateAuditSummary,
  HistoryStateHookEvidence,
  MessageStateDelta,
  RunControlAuditSummary,
  SessionRuntimeView
} from "../../types/runtime";
import { normalizeTurnTraceRecord } from "./trace";

export type HistoryCheckoutWireResult = {
  sessionId: string;
  nodeId: string;
  requestedMode: HistoryCheckoutMode;
  appliedMode: HistoryCheckoutMode;
  transcriptRestoreApplied: boolean;
  workspaceRollbackCapable: boolean;
  workspaceRollbackApplied: boolean;
  degraded: boolean;
  degradationReason?: string | null;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  messageDelta?: MessageStateDelta | null;
  messageRevision?: string | null;
  cursor: HistoryCursorState;
};

export type HistoryRestoreWireResult = {
  sessionId: string;
  branchId?: string | null;
  restoredNodeId?: string | null;
  transcriptRestoreApplied: boolean;
  workspaceRollbackCapable: boolean;
  workspaceRollbackApplied: boolean;
  degraded: boolean;
  degradationReason?: string | null;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  messageDelta?: MessageStateDelta | null;
  messageRevision?: string | null;
  cursor: HistoryCursorState;
};

export type HistoryForkWireResult = {
  sessionId: string;
  nodeId: string;
  branch: HistoryBranch;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  messageDelta?: MessageStateDelta | null;
  messageRevision?: string | null;
  cursor: HistoryCursorState;
};

export type HistoryBranchSwitchWireResult = {
  sessionId: string;
  branchId: string;
  nodeId?: string | null;
  historyStateEvidence?: HistoryStateHookEvidence[] | null;
  historyStateAuditSummary?: HistoryStateAuditSummary | null;
  messageDelta?: MessageStateDelta | null;
  messageRevision?: string | null;
  cursor: HistoryCursorState;
};

export function cloneHistoryNodes(nodes?: HistoryNode[] | null) {
  return (nodes ?? []).map((node) => ({
    ...node,
    workspaceRef: node.workspaceRef ? { ...node.workspaceRef } : node.workspaceRef ?? null,
    history: (node.history ?? []).map((message) => ({
      ...message,
      attachments: (message.attachments ?? []).map((attachment) => ({ ...attachment }))
    })),
    turnTraceHistory: (node.turnTraceHistory ?? []).map((trace) => normalizeTurnTraceRecord(trace)),
    turnTraceRefs: (node.turnTraceRefs ?? []).map((reference) => ({ ...reference }))
  }));
}

export function cloneHistoryBranches(branches?: HistoryBranch[] | null) {
  return (branches ?? []).map((branch) => ({ ...branch }));
}

export function cloneHistoryCursor(cursor?: HistoryCursorState | null) {
  return cursor ? { ...cursor } : null;
}

export function hasHostManagedHistoryState(runtimeView?: Pick<SessionRuntimeView, "historyCursor" | "authorityMode"> | null) {
  return (runtimeView?.authorityMode ?? runtimeView?.historyCursor?.authorityMode ?? "host_authoritative") === "host_authoritative";
}

export function previewHistoryMutationUnavailable() {
  return "当前为降级模式，不支持历史状态恢复操作。";
}

export function historyCursorVersion(cursor?: Pick<HistoryCursorState, "cursorVersion"> | null) {
  return typeof cursor?.cursorVersion === "number" && Number.isFinite(cursor.cursorVersion)
    ? cursor.cursorVersion
    : null;
}

export function normalizeHistoryCursorMode(mode?: string | null): HistoryCursorMode {
  if (mode === "historical" || mode === "historical_dirty") {
    return mode;
  }

  return "live";
}

export function resolveRuntimeViewHistoryProjection(
  runtimeView?:
    | Pick<
        SessionRuntimeView,
        | "historyCursor"
        | "resolvedVisibleNodeId"
        | "activeBranchHeadNodeId"
        | "isAtBranchHead"
        | "historyNodes"
        | "historyBranches"
      >
    | null
) {
  const runtimeHistoryCursor = cloneHistoryCursor(runtimeView?.historyCursor);
  if (runtimeHistoryCursor) {
    return runtimeHistoryCursor;
  }

  const visibleNodeId = runtimeView?.resolvedVisibleNodeId?.trim() || null;
  const activeBranchId =
    runtimeView?.historyBranches?.find((branch) => branch.headNodeId === runtimeView?.activeBranchHeadNodeId)?.branchId ?? null;
  const branchHeadNodeId = runtimeView?.activeBranchHeadNodeId?.trim() || null;
  if (!visibleNodeId && !branchHeadNodeId && !activeBranchId) {
    return null;
  }

  return {
    sessionId: "",
    visibleNodeId,
    activeBranchId,
    branchHeadNodeId,
    workspaceNodeId: visibleNodeId,
    mode: normalizeHistoryCursorMode(
      runtimeView?.isAtBranchHead === false ? "historical" : "live"
    )
  } satisfies Partial<HistoryCursorState>;
}

export function cloneHistoryStateEvidence(evidence?: HistoryStateHookEvidence[] | null) {
  return (evidence ?? []).map((item) => ({ ...item }));
}

export function cloneHistoryStateAuditSummary(
  summary?: HistoryStateAuditSummary | null
): HistoryStateAuditSummary | null {
  if (!summary) {
    return null;
  }

  return {
    action: { ...summary.action },
    currentContext: { ...summary.currentContext }
  };
}

export function cloneRunControlAuditSummary(
  summary?: RunControlAuditSummary | null
): RunControlAuditSummary | null {
  if (!summary) {
    return null;
  }

  return {
    actionEvidenceSummary: { ...summary.actionEvidenceSummary },
    currentContextProjection: { ...summary.currentContextProjection }
  };
}

export function normalizeHistoryCheckoutResult(
  payload: HistoryCheckoutWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryCheckoutResult {
  return {
    sessionId: payload.sessionId,
    nodeId: payload.nodeId,
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    requestedMode: payload.requestedMode,
    appliedMode: payload.appliedMode,
    transcriptRestoreApplied: payload.transcriptRestoreApplied,
    workspaceRollbackCapable: payload.workspaceRollbackCapable,
    workspaceRestoreCapable: payload.workspaceRollbackCapable,
    workspaceRollbackApplied: payload.workspaceRollbackApplied,
    workspaceRestoreApplied: payload.workspaceRollbackApplied,
    degraded: payload.degraded,
    degradedToTranscriptOnly: payload.degraded,
    degradationReason: payload.degradationReason ?? null,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

export function normalizeHistoryRestoreResult(
  payload: HistoryRestoreWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryRestoreResult {
  const legacyRestoredFromNodeId = (
    payload as HistoryRestoreWireResult & { restoredFromNodeId?: string | null }
  ).restoredFromNodeId;
  return {
    sessionId: payload.sessionId,
    branchId: payload.branchId ?? null,
    restoredNodeId: payload.restoredNodeId ?? null,
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    transcriptRestoreApplied: payload.transcriptRestoreApplied,
    workspaceRollbackCapable: payload.workspaceRollbackCapable,
    workspaceRestoreCapable: payload.workspaceRollbackCapable,
    workspaceRollbackApplied: payload.workspaceRollbackApplied,
    workspaceRestoreApplied: payload.workspaceRollbackApplied,
    degraded: payload.degraded,
    degradedToTranscriptOnly: payload.degraded,
    degradationReason: payload.degradationReason ?? null,
    restoredFromNodeId: legacyRestoredFromNodeId ?? payload.restoredNodeId ?? null,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

export function normalizeHistoryForkResult(
  payload: HistoryForkWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryForkResult {
  return {
    sessionId: payload.sessionId,
    nodeId: payload.nodeId,
    createdBranchId: payload.branch.branchId,
    branch: { ...payload.branch },
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

export function normalizeHistoryBranchSwitchResult(
  payload: HistoryBranchSwitchWireResult,
  historyNodes?: HistoryNode[] | null,
  historyBranches?: HistoryBranch[] | null
): HistoryBranchSwitchResult {
  return {
    sessionId: payload.sessionId,
    branchId: payload.branchId,
    nodeId: payload.nodeId ?? null,
    visibleNodeId: payload.cursor.visibleNodeId ?? null,
    activeBranchId: payload.cursor.activeBranchId ?? null,
    branchHeadNodeId: payload.cursor.branchHeadNodeId ?? null,
    workspaceNodeId: payload.cursor.workspaceNodeId ?? null,
    mode: payload.cursor.mode,
    historyStateEvidence: cloneHistoryStateEvidence(payload.historyStateEvidence),
    historyStateAuditSummary: cloneHistoryStateAuditSummary(payload.historyStateAuditSummary),
    historyNodes: cloneHistoryNodes(historyNodes),
    historyBranches: cloneHistoryBranches(historyBranches)
  };
}

export function resolveHistoryBranchHeadNodeId(branchId: string | null, branches: HistoryBranch[]) {
  if (!branchId) {
    return null;
  }

  return branches.find((branch) => branch.branchId === branchId)?.headNodeId ?? null;
}

export function historyNodeStableTurnId(node: HistoryNode | null | undefined) {
  const explicitTurnId = node?.turnId?.trim() || "";
  if (explicitTurnId) {
    return explicitTurnId;
  }

  const traceTurnId = node?.turnTraceHistory?.[node.turnTraceHistory.length - 1]?.turnId?.trim() || "";
  if (traceTurnId) {
    return traceTurnId;
  }

  return null;
}

export function buildConversationCheckpointEntries(
  historyNodes: HistoryNode[],
  historyBranches: HistoryBranch[],
  activeBranchId: string | null,
  visibleNodeId: string | null,
  branchHeadNodeId: string | null
): ConversationCheckpointEntry[] {
  const entriesByNodeId = new Map<string, ConversationCheckpointEntry>();
  const forkBranchesByNodeId = new Map<string, HistoryBranch[]>();

  for (const branch of historyBranches) {
    const sourceNodeId = branch.forkedFromNodeId?.trim() || "";
    if (!sourceNodeId) {
      continue;
    }

    const existing = forkBranchesByNodeId.get(sourceNodeId) ?? [];
    existing.push(branch);
    forkBranchesByNodeId.set(sourceNodeId, existing);
  }

  const latestNodeId = branchHeadNodeId?.trim() || null;

  for (const node of historyNodes) {
    const turnId = historyNodeStableTurnId(node);
    if (!turnId) {
      continue;
    }

    const workspaceRollbackCapable = Boolean(node.workspaceRef?.rollbackCapable);
    const forkTargets = (forkBranchesByNodeId.get(node.nodeId) ?? [])
      .map((branch) => {
        const targetNodeId = branch.headNodeId?.trim() || "";
        if (!targetNodeId) {
          return null;
        }

        const targetNode = historyNodes.find((item) => item.nodeId === targetNodeId) ?? null;
        return {
          branchId: branch.branchId,
          nodeId: targetNodeId,
          label: branch.label?.trim() || branch.branchId,
          summary: targetNode?.summary?.trim() || branch.label?.trim() || branch.branchId,
          isActive: branch.branchId === activeBranchId
        };
      })
      .filter((target): target is ConversationCheckpointEntry["forkTargets"][number] => Boolean(target));

    entriesByNodeId.set(node.nodeId, {
      nodeId: node.nodeId,
      turnId,
      branchId: node.branchId,
      summary: node.summary?.trim() || node.title?.trim() || node.nodeId,
      createdAtMs: node.createdAtMs,
      isLatest: latestNodeId != null && node.nodeId === latestNodeId,
      isVisible: visibleNodeId != null && node.nodeId === visibleNodeId,
      workspaceRollbackCapable,
      availableModes: workspaceRollbackCapable
        ? ["transcript_only", "transcript_and_workspace"]
        : ["transcript_only"],
      forkTargets
    });
  }

  return [...entriesByNodeId.values()].sort((left, right) => right.createdAtMs - left.createdAtMs);
}

export function isHistoricalRuntimeView(
  runtimeView?:
    | Pick<SessionRuntimeView, "historyCursor">
    | null
) {
  return normalizeHistoryCursorMode(runtimeView?.historyCursor?.mode) !== "live";
}

export function isHistoricalMode(mode?: HistoryCursorMode | null) {
  return normalizeHistoryCursorMode(mode) !== "live";
}
