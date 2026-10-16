import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import type { GraphAskWait, PendingAsk } from "@/types/ask-plan";

export const ASK_POLL_INTERVAL_MS = 2500;

/** User-facing hint for a locally expired Ask (never the bare host CAS text). */
export const ASK_EXPIRED_MESSAGE = "该请求已过期，已同步最新待处理列表";

/**
 * Local TTL guard. The host keeps an expired request in `Pending` state (no `ask_expire` call
 * site), so submitting it is doomed to fail the dispatcher CAS (`now_ms <= expires_at_ms`) with
 * the generic `cannot be consumed` text. `expiresAtMs <= 0` means "no TTL", not "expired".
 *
 * The boundary mirrors the host check exactly: `can_consume` accepts while
 * `now_ms <= expires_at_ms`, so this guard reports expired only for `expiresAtMs < nowMs` — the
 * one instant where `expiresAtMs == nowMs` is still host-consumable must not be pre-emptively
 * blocked, or a live answer would be silently dropped.
 *
 * The reference clock is the submit instant (`Date.now()`), never the listing snapshot
 * (`lastRefreshMs`): polling self-terminates on an empty list, so a later Ask may be listed only
 * once by a `turn:suspended` refresh and the snapshot clock then freezes. Judging by that frozen
 * clock misses a request whose TTL elapsed after listing and leaks a doomed CAS (C2-7).
 */
export function isAskExpired(
  ask: Pick<PendingAsk, "expiresAtMs"> | null | undefined,
  nowMs: number = Date.now()
): boolean {
  const expiresAtMs = ask?.expiresAtMs;
  return typeof expiresAtMs === "number" && expiresAtMs > 0 && expiresAtMs < nowMs;
}

/**
 * Single list entry point for the pending-Ask snapshot. `sessionId` is only sent when non-empty so
 * the host keeps its "all sessions" default for an empty scope.
 */
async function listPendingAsks(sessionId?: string): Promise<PendingAsk[]> {
  const args: Record<string, unknown> = {};
  const scopedSessionId = sessionId?.trim();
  if (scopedSessionId) {
    args.sessionId = scopedSessionId;
  }
  const asks = await safeInvoke<PendingAsk[] | null>("ask_list_pending", args);
  return Array.isArray(asks) ? asks : [];
}

// `turn:suspended` must be included so a freshly-paused run (Ask pending) surfaces its card even
// when polling self-terminated with no pending asks (PA-076 phase-4 P1-1).
const ASK_REFRESH_EVENTS = [
  "turn:completed",
  "turn:failed",
  "turn:cancelled",
  "turn:suspended"
] as const;

type AskState = {
  pendingAsks: PendingAsk[];
  loading: boolean;
  error: string | null;
  answeringRequestId: string | null;
  cancellingRequestId: string | null;
  lastRefreshMs: number | null;
  polling: boolean;
  pollingDesired: boolean;
  reconcilingExpired: boolean;
  pollTimer: ReturnType<typeof setTimeout> | null;
  watchingEvents: boolean;
  unlistenFns: Array<() => void>;
};

/**
 * Detect a compare-and-swap failure from the host Ask adapter. These are not
 * retryable by resubmitting the same version — the pending request was already
 * consumed, cancelled, expired, or otherwise mutated — so callers should
 * reconcile the list instead of retrying.
 */
export function isStaleAskError(error: unknown): boolean {
  const message = String(error ?? "").toLowerCase();
  return (
    message.includes("cannot be consumed") ||
    message.includes("replay") ||
    message.includes("expired") ||
    message.includes("expiry") ||
    message.includes("version/nonce mismatch") ||
    message.includes("stale mutation rejected") ||
    message.includes("unknown control request") ||
    message.includes("kind mismatch") ||
    message.includes("not found")
  );
}

export const useAskStore = defineStore("ask", {
  state: (): AskState => ({
    pendingAsks: [],
    loading: false,
    error: null,
    answeringRequestId: null,
    cancellingRequestId: null,
    lastRefreshMs: null,
    polling: false,
    pollingDesired: false,
    reconcilingExpired: false,
    pollTimer: null,
    watchingEvents: false,
    unlistenFns: []
  }),
  getters: {
    pendingCount(state): number {
      return state.pendingAsks.length;
    },
    hasPending(state): boolean {
      return state.pendingAsks.length > 0;
    },
    isAnswering(state) {
      return (requestId: string) => state.answeringRequestId === requestId;
    },
    isCancelling(state) {
      return (requestId: string) => state.cancellingRequestId === requestId;
    }
  },
  actions: {
    /**
     * Refresh the pending-Ask snapshot. Optionally filters by session.
     *
     * Two self-healing duties hook onto the single list entry point:
     * - a snapshot holding a locally expired Ask is auto-reconciled (silently, single-flight), since
     *   the panel disables every control of such a card and the click-path guard is unreachable (C2-6);
     * - a non-empty snapshot restarts a poll that self-terminated on an empty list, so the snapshot
     *   clock keeps advancing and reconciliation keeps being driven (C2-8).
     */
    async refresh(options?: { sessionId?: string | null }): Promise<PendingAsk[]> {
      if (!isTauriAvailable()) {
        this.pendingAsks = [];
        this.loading = false;
        return [];
      }

      this.loading = true;
      this.error = null;
      const sessionId = options?.sessionId?.trim();
      try {
        this.pendingAsks = await listPendingAsks(sessionId);
        this.lastRefreshMs = Date.now();
        if (!this.reconcilingExpired && this.pendingAsks.some((ask) => isAskExpired(ask))) {
          await this.reconcileExpiredAsks({ sessionId, surfaceError: false });
        }
        if (this.pendingAsks.length > 0 && this.pollingDesired && !this.polling) {
          this.startPolling();
        }
        return this.pendingAsks;
      } catch (error) {
        this.error = `刷新待处理请求失败：${String(error)}`;
        return [];
      } finally {
        this.loading = false;
      }
    },

    /**
     * Optimistic-state protection for a reconcile re-list: never re-populate a request that is
     * currently being answered/cancelled locally, or the re-list would undo the optimistic removal
     * and re-enable a doomed CAS (C2-9b).
     */
    filterInFlight(asks: PendingAsk[]): PendingAsk[] {
      return asks.filter(
        (ask) =>
          ask.requestId !== this.answeringRequestId && ask.requestId !== this.cancellingRequestId
      );
    },

    /**
     * Answer a pending Ask. The compare-and-swap `expectedVersion` is the
     * `version` captured at listing time — the request is consumed locally on
     * success and never re-fetched (a re-fetched version would already be
     * bumped and stale resubmits are rejected by the host). Answering is only
     * allowed for a request still present in the local snapshot, which prevents
     * stale resubmits after the request was already consumed or removed.
     */
    async answer(request: PendingAsk, answerValue: unknown): Promise<unknown | null> {
      if (!request || request.requestKind !== "interaction") {
        return null;
      }
      if (
        this.answeringRequestId === request.requestId ||
        this.cancellingRequestId === request.requestId
      ) {
        return null;
      }
      const listed = this.pendingAsks.find((ask) => ask.requestId === request.requestId);
      if (!listed) {
        return null;
      }
      // A locally expired Ask must never reach the host CAS: reconcile instead of submitting.
      // Judged at the submit instant (C2-7), not against the listing snapshot.
      if (isAskExpired(listed)) {
        return this.reconcileExpiredAsk();
      }

      // Optimistically remove from pending asks to prevent duplicate submissions
      // and immediately disable UI buttons while IPC is inflight.
      this.removePendingAsk(request.requestId);
      this.answeringRequestId = request.requestId;
      this.error = null;
      try {
        const result = await safeInvoke<unknown>("ask_answer", {
          requestId: request.requestId,
          expectedVersion: listed.version,
          version: listed.version,
          answer: answerValue ?? null
        });
        // PA-076 phase-4 P0: `ask_answer` CAS-consumed the dispatcher request; the graph run is
        // still bound at `WaitingUser`. Resume the graph so the terminal result (the answer) is
        // injected into the next turn and the run can continue.
        await this.resumeGraphAsk(request, answerValue ?? null);
        return result;
      } catch (error) {
        const message = `应答失败：${String(error)}`;
        if (isStaleAskError(error)) {
          // Reconcile the snapshot after a CAS / replay / expiry failure.
          // Since the request is already dead/consumed, reconcile from host.
          await this.refresh().catch(() => {});
        } else {
          // If it was a network/internal error rather than a stale CAS, restore the ask
          if (!this.pendingAsks.some((a) => a.requestId === request.requestId)) {
            this.pendingAsks.push(listed);
          }
        }
        this.error = message;
        return null;
      } finally {
        this.answeringRequestId = null;
      }
    },

    /**
     * Consume the graph Ask wait binding after the dispatcher request was CAS-consumed, so the
     * bound run moves back to `Ready` and the terminal tool result is injected for the original
     * call id. Non-fatal: the answer itself already succeeded, so a resume failure is surfaced
     * for observability rather than rolled back.
     */
    async resumeGraphAsk(request: PendingAsk, answerValue: unknown): Promise<void> {
      const runId = request.runId;
      if (!runId || !request.requestKind || request.requestKind !== "interaction") {
        return;
      }
      try {
        const waits = await safeInvoke<GraphAskWait[] | null>("graph_list_ask_waits", { runId });
        const wait = (Array.isArray(waits) ? waits : []).find(
          (item) => item.requestId === request.requestId
        );
        if (!wait) {
          // The binding was already consumed elsewhere; nothing to resume.
          return;
        }
        await safeInvoke("graph_resume_ask", {
          runId,
          requestId: request.requestId,
          version: wait.expectedVersion,
          answer: answerValue ?? null
        });
      } catch (error) {
        this.error = `恢复运行失败：${String(error)}`;
      }
    },

    /** Cancel a pending Ask with the listed compare-and-swap version. */
    async cancel(request: PendingAsk): Promise<unknown | null> {
      if (!request) {
        return null;
      }
      if (
        this.answeringRequestId === request.requestId ||
        this.cancellingRequestId === request.requestId
      ) {
        return null;
      }
      const listed = this.pendingAsks.find((ask) => ask.requestId === request.requestId);
      if (!listed) {
        return null;
      }
      // Same local TTL guard as `answer`: reconcile, never submit a doomed CAS (submit instant).
      if (isAskExpired(listed)) {
        return this.reconcileExpiredAsk();
      }

      // Optimistically remove from pending asks to prevent duplicate cancellations
      this.removePendingAsk(request.requestId);
      this.cancellingRequestId = request.requestId;
      this.error = null;
      try {
        const result = await safeInvoke<unknown>("ask_cancel", {
          requestId: request.requestId,
          expectedVersion: listed.version,
          version: listed.version
        });
        return result;
      } catch (error) {
        const message = `取消失败：${String(error)}`;
        if (isStaleAskError(error)) {
          await this.refresh().catch(() => {});
        } else {
          if (!this.pendingAsks.some((a) => a.requestId === request.requestId)) {
            this.pendingAsks.push(listed);
          }
        }
        this.error = message;
        return null;
      } finally {
        this.cancellingRequestId = null;
      }
    },

    /** Drop a request from the local snapshot without a host round-trip. */
    removePendingAsk(requestId: string): void {
      this.pendingAsks = this.pendingAsks.filter((ask) => ask.requestId !== requestId);
    },

    /**
     * Reconcile a snapshot holding locally expired Ask(s): expire them host-side, then re-list once
     * and keep the fresh truth. `refresh()` drives this silently (C2-6/§7.1); the click-path guard
     * (`answer()`/`cancel()`) drives it with an explicit expiry hint (C2-2).
     *
     * Single-flight: concurrent triggers (double click, poll/click overlap) coalesce into one
     * `ask_expire` (C2-9). The re-list is inlined instead of `refresh()` to prevent recursion.
     */
    async reconcileExpiredAsks(options?: {
      sessionId?: string;
      surfaceError?: boolean;
    }): Promise<void> {
      if (this.reconcilingExpired) {
        return;
      }
      this.reconcilingExpired = true;
      try {
        try {
          await safeInvoke("ask_expire");
        } catch (error) {
          // Visible failure, but never a reason to skip the re-list that is the point of reconcile.
          this.error = `过期处理失败：${String(error)}`;
        }
        try {
          this.pendingAsks = this.filterInFlight(await listPendingAsks(options?.sessionId));
          this.lastRefreshMs = Date.now();
        } catch (error) {
          this.error = `刷新待处理请求失败：${String(error)}`;
        }
        // `surfaceError` marks the click path: only a *successful* reconcile earns the plain expiry
        // hint, so a concrete failure is never masked (C2-2) — and an automatic refresh-driven
        // reconcile stays silent, or the text would outlive the emptied list (§7.1).
        if (options?.surfaceError && !this.error) {
          this.error = ASK_EXPIRED_MESSAGE;
        }
      } finally {
        this.reconcilingExpired = false;
      }
    },

    /**
     * Reconcile an Ask whose local TTL already passed. Kept as its own action so `answer()` and
     * `cancel()` share one closure that never submits a doomed CAS.
     */
    async reconcileExpiredAsk(): Promise<null> {
      await this.reconcileExpiredAsks({ surfaceError: true });
      if (!this.error) {
        this.error = ASK_EXPIRED_MESSAGE;
      }
      return null;
    },

    /** Transition expired pending requests and reconcile the snapshot. */
    async expire(): Promise<number | null> {
      if (!isTauriAvailable()) {
        return null;
      }

      try {
        const result = await safeInvoke<{ expired?: number } | null>("ask_expire");
        await this.refresh();
        return result?.expired ?? null;
      } catch (error) {
        this.error = `过期处理失败：${String(error)}`;
        return null;
      }
    },

    /**
     * Begin lightweight periodic polling. Polling self-terminates once no
     * pending asks remain, so it never keeps timers alive in an idle app; the
     * "desired" flag survives that self-termination so `refresh()` can re-arm
     * the poll when a snapshot becomes non-empty again (C2-8).
     */
    startPolling(intervalMs: number = ASK_POLL_INTERVAL_MS): void {
      this.pollingDesired = true;
      if (this.polling) {
        return;
      }
      this.polling = true;
      this.schedulePoll(intervalMs);
    },

    schedulePoll(intervalMs: number): void {
      if (!this.polling || this.pollTimer != null) {
        return;
      }
      this.pollTimer = setTimeout(async () => {
        this.pollTimer = null;
        await this.refresh();
        if (this.pendingAsks.length === 0) {
          this.haltPolling();
          return;
        }
        this.schedulePoll(intervalMs);
      }, intervalMs);
    },

    /** Stop the timer but keep `pollingDesired`, so a later non-empty snapshot can re-arm it. */
    haltPolling(): void {
      this.polling = false;
      if (this.pollTimer != null) {
        clearTimeout(this.pollTimer);
        this.pollTimer = null;
      }
    },

    /** Host teardown semantics: this caller no longer wants polling at all. */
    stopPolling(): void {
      this.pollingDesired = false;
      this.haltPolling();
    },

    /** Listen to terminal turn events so a freshly-paused run surfaces its ask. */
    async startWatchingEvents(): Promise<void> {
      if (this.watchingEvents || !isTauriAvailable()) {
        return;
      }
      this.watchingEvents = true;
      for (const event of ASK_REFRESH_EVENTS) {
        try {
          const unlisten = await safeListen(event, () => {
            void this.refresh();
          });
          this.unlistenFns.push(unlisten);
        } catch {
          // Individual listener registration failures are non-fatal.
        }
      }
    },

    stopWatchingEvents(): void {
      this.watchingEvents = false;
      for (const unlisten of this.unlistenFns) {
        try {
          unlisten();
        } catch {
          // Ignore teardown failures.
        }
      }
      this.unlistenFns = [];
    },

    reset(): void {
      this.stopPolling();
      this.pollingDesired = false;
      this.reconcilingExpired = false;
      this.stopWatchingEvents();
      this.pendingAsks = [];
      this.loading = false;
      this.error = null;
      this.answeringRequestId = null;
      this.cancellingRequestId = null;
      this.lastRefreshMs = null;
    }
  }
});
