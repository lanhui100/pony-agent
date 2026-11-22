// D6 历史/分支/检查点（13 键 / 2 getters / 4 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**，语义零改动。键清单与签名真源见 `./types.ts`，
// 域归属与切分口径见 `.dev-team/runtime-split/fact-map.md`。与原实现的机械差异：import 路径按新层级
// 调整；action 方法补 `this: RuntimeStoreThis` 形参（Pinia options 式 `this` 口径）；类型
// `RuntimeStateW59` 改名 `RuntimeStoreState`（types.ts 的等价迁出物）。
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type {
  ConversationCheckpointEntry,
  ExecutionCheckpoint,
  HistoryBranch,
  HistoryCursorState,
  HistoryNode,
  HistoryStateAuditSummary,
  MessageStateDelta,
  SessionRuntimeView,
} from "@/types/runtime";
import {
  buildConversationCheckpointEntries,
  cloneHistoryBranches,
  cloneHistoryNodes,
  cloneHistoryStateAuditSummary,
  cloneRunControlAuditSummary,
  hasHostManagedHistoryState,
  historyCursorVersion,
  normalizeHistoryCursorMode,
  resolveHistoryBranchHeadNodeId,
} from "@/lib/runtime/history";
import {
  buildRuntimeViewFromPersistedState,
  cloneGraphRunControlBoundaryEvidence,
  loadPersistedRuntimeState,
} from "@/lib/runtime/sessions";
import type { HistoryActions, HistoryGetters, RuntimeStoreThis } from "./types";

/** D6 历史/分支/检查点（13 键 / 2 getters / 4 actions） getters（键清单见 `types.ts` 的 `HistoryGetters` 与事实图 §2）。 */
export const historyGetters = {
  isHistoricalMode(state): boolean {
    return state.historyCursorMode !== "live";
  },
  conversationCheckpointEntries(state): ConversationCheckpointEntry[] {
    const entries = buildConversationCheckpointEntries(
      state.historyNodes,
      state.historyBranches,
      state.activeBranchId,
      state.visibleNodeId,
      state.branchHeadNodeId
    );

    // Synthesize entries for completed turns in messages that lack history nodes.
    // This ensures rollback buttons work even before backend checkpoints arrive.
    const coveredTurnIds = new Set(entries.map((e) => e.turnId));
    const turnUserContent = new Map<string, string>();
    for (const msg of state.messages) {
      if (msg.role === "user" && msg.content && !turnUserContent.has(msg.turnId)) {
        turnUserContent.set(msg.turnId, msg.content);
      }
    }

    const allTurnIds = [...turnUserContent.keys()];
    let hasSynthetic = false;

    for (let index = 0; index < allTurnIds.length; index++) {
      const turnId = allTurnIds[index]!;
      if (coveredTurnIds.has(turnId)) continue;

      entries.push({
        nodeId: `synthetic-${turnId}`,
        turnId,
        branchId: state.activeBranchId ?? "branch-main",
        summary: turnUserContent.get(turnId)!.slice(0, 120) || turnId,
        createdAtMs: Date.now() - (allTurnIds.length - index),
        isLatest: false,
        isVisible: false,
        workspaceRollbackCapable: false,
        availableModes: ["transcript_only"],
        forkTargets: []
      });
      hasSynthetic = true;
    }

    if (hasSynthetic) {
      entries.sort((left, right) => right.createdAtMs - left.createdAtMs);
      const hasRealLatest =
        state.branchHeadNodeId != null && entries.some((e) => e.nodeId === state.branchHeadNodeId);
      if (!hasRealLatest && entries.length > 0) {
        entries[0] = { ...entries[0], isLatest: true };
      }
    }

    return entries;
  },
} as HistoryGetters;

/** D6 历史/分支/检查点（13 键 / 2 getters / 4 actions） actions（键清单见 `types.ts` 的 `HistoryActions` 与事实图 §3）。 */
export const historyActions = {
  async performHistoryOperation<TInvokeArgs extends Record<string, unknown>, TResult extends HistoryCursorState & { historyStateAuditSummary?: HistoryStateAuditSummary | null }>(this: RuntimeStoreThis, 
    options: {
      cmd: string;
      invokeArgs: TInvokeArgs;
      normalize: (payload: any, nodes: HistoryNode[], branches: HistoryBranch[]) => TResult;
      selectNodeId?: (result: any) => string | null;
      errorMessagePrefix?: string;
    }
  ): Promise<TResult | null> {
    try {
      const payload = await safeInvoke(options.cmd, {
        sessionId: this.sessionId,
        expectedCursorVersion: this.cursorVersion,
        ...options.invokeArgs
      });
      const messageDelta = (payload as { messageDelta?: MessageStateDelta | null }).messageDelta ?? null;
      if (!this.applyMessageStateDelta(messageDelta)) {
        const preservedDraft = this.draftMessage;
        await this.loadSessionState(this.sessionId, {
          refreshCatalog: false,
          nodeId: options.selectNodeId?.(payload) ?? null
        });
        this.draftMessage = preservedDraft;
      }
      const result = options.normalize(payload, this.historyNodes, this.historyBranches);
      this.initialRollbackActive = false;
      this.applyHistoryState(this.sessionId, result);
      this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
        result.historyStateAuditSummary ?? null
      );
      return result;
    } catch (error) {
      this.sessionError = `${options.errorMessagePrefix ?? "操作"}失败：${String(error)}`;
      return null;
    }
  },
  applyHistoryState(this: RuntimeStoreThis, 
    _sessionId: string,
    payload?:
      | (Partial<HistoryCursorState> & {
          historyNodes?: HistoryNode[] | null;
          historyBranches?: HistoryBranch[] | null;
        })
      | null
  ) {
    const historyNodes = cloneHistoryNodes(payload?.historyNodes);
    const historyBranches = cloneHistoryBranches(payload?.historyBranches);
    const activeBranchId = payload?.activeBranchId?.trim() || null;
    const explicitHeadNodeId = payload?.branchHeadNodeId?.trim() || null;
    const branchHeadNodeId =
      explicitHeadNodeId || resolveHistoryBranchHeadNodeId(activeBranchId, historyBranches);
    const visibleNodeId = payload?.visibleNodeId?.trim() || null;

    this.historyNodes = historyNodes;
    this.historyBranches = historyBranches;
    this.activeBranchId = activeBranchId;
    this.branchHeadNodeId = branchHeadNodeId;
    this.visibleNodeId = visibleNodeId;
    this.cursorVersion = historyCursorVersion(payload);
    this.historyCursorMode = normalizeHistoryCursorMode(
      payload?.mode ?? (visibleNodeId && branchHeadNodeId && visibleNodeId !== branchHeadNodeId ? "historical" : "live")
    );
  },
  async loadSessionState(this: RuntimeStoreThis, 
    nextSessionId: string,
    options?: {
      refreshCatalog?: boolean;
      executionCheckpoint?: ExecutionCheckpoint | null;
      runtimeView?: SessionRuntimeView | null;
      nodeId?: string | null;
      preserveMessages?: boolean;
    }
  ) {
    const refreshCatalog = options?.refreshCatalog ?? true;
    const persistedState = loadPersistedRuntimeState(nextSessionId);
    const shouldPreferLocalInitialRollback = Boolean(persistedState?.initialRollbackActive) && !options?.nodeId;
    const persistedVisibleNodeId = persistedState?.visibleNodeId?.trim() || null;
    const fallbackNodeId =
      options?.nodeId ??
      (!isTauriAvailable()
        ? persistedVisibleNodeId
        : null);
    const runtimeView =
      options?.runtimeView ?? (shouldPreferLocalInitialRollback
        ? buildRuntimeViewFromPersistedState(nextSessionId, persistedState, fallbackNodeId)
        : await this.loadSessionRuntimeViewState(nextSessionId, fallbackNodeId));
    const snapshot = runtimeView.session;
    const retrieved = runtimeView.retrieved;
    const persisted = persistedState;
    const runtimeViewIsHistorical =
      !hasHostManagedHistoryState(runtimeView)
        ? normalizeHistoryCursorMode(runtimeView.historyCursor?.mode ?? null) !== "live"
        : normalizeHistoryCursorMode(runtimeView.historyCursor?.mode ?? null) !== "live" ||
          runtimeView.isAtBranchHead === false;
    const shouldApplyRuntimeViewCheckpoint =
      !runtimeViewIsHistorical && (!options?.nodeId || !runtimeViewIsHistorical);
    const hasCheckpointOverride =
      options != null && Object.prototype.hasOwnProperty.call(options, "executionCheckpoint");
    this.applySessionSnapshot(nextSessionId, snapshot, retrieved, runtimeView, { preserveMessages: options?.preserveMessages });
    this.applyExecutionCheckpoint(
      hasCheckpointOverride
        ? (options?.executionCheckpoint ?? null)
        : shouldApplyRuntimeViewCheckpoint
          ? (runtimeView.checkpoint ?? null)
          : null,
      persisted?.messages
    );
    this.latestGraphRunSubmissionPlan = runtimeView.submissionPlan ? { ...runtimeView.submissionPlan } : null;
    this.latestGraphRunControlBoundaryEvidence = cloneGraphRunControlBoundaryEvidence(
      runtimeView.controlBoundaryEvidence
    );
    this.latestRunControlAuditSummary = cloneRunControlAuditSummary(
      runtimeView.runControlAuditSummary ?? snapshot.runControlAuditSummary ?? null
    );
    this.latestHistoryStateAuditSummary = cloneHistoryStateAuditSummary(
      runtimeView.historyStateAuditSummary ?? snapshot.historyStateAuditSummary ?? null
    );

    if (refreshCatalog) {
      await this.loadSessionCatalog();
    }
  },
  findCheckpointTurnIdByNodeId(this: RuntimeStoreThis, nodeId: string): string | null {
    const entry = this.conversationCheckpointEntries.find((e) => e.nodeId === nodeId);
    return entry?.turnId?.trim() || null;
  },
} as HistoryActions;
