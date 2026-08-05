import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import type { GraphAskWait, PendingAsk } from "@/types/ask-plan";

export const ASK_POLL_INTERVAL_MS = 2500;

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
    /** Refresh the pending-Ask snapshot. Optionally filters by session. */
    async refresh(options?: { sessionId?: string | null }): Promise<PendingAsk[]> {
      if (!isTauriAvailable()) {
        this.pendingAsks = [];
        this.loading = false;
        return [];
      }

      this.loading = true;
      this.error = null;
      try {
        const args: Record<string, unknown> = {};
        const sessionId = options?.sessionId?.trim();
        if (sessionId) {
          args.sessionId = sessionId;
        }
        const asks = await safeInvoke<PendingAsk[] | null>("ask_list_pending", args);
        this.pendingAsks = Array.isArray(asks) ? asks : [];
        this.lastRefreshMs = Date.now();
        return this.pendingAsks;
      } catch (error) {
        this.error = `刷新待处理请求失败：${String(error)}`;
        return [];
      } finally {
        this.loading = false;
      }
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

      this.answeringRequestId = request.requestId;
      this.error = null;
      try {
        const result = await safeInvoke<unknown>("ask_answer", {
          requestId: request.requestId,
          expectedVersion: listed.version,
          answer: answerValue ?? null
        });
        this.removePendingAsk(request.requestId);
        // PA-076 phase-4 P0: `ask_answer` CAS-consumed the dispatcher request; the graph run is
        // still bound at `WaitingUser`. Resume the graph so the terminal result (the answer) is
        // injected into the next turn and the run can continue.
        await this.resumeGraphAsk(request, answerValue ?? null);
        return result;
      } catch (error) {
        const message = `应答失败：${String(error)}`;
        if (isStaleAskError(error)) {
          // Reconcile the snapshot after a CAS failure, then surface the
          // mutation error (the reconcile would otherwise clear it).
          await this.refresh().catch(() => {});
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

      this.cancellingRequestId = request.requestId;
      this.error = null;
      try {
        const result = await safeInvoke<unknown>("ask_cancel", {
          requestId: request.requestId,
          expectedVersion: listed.version
        });
        this.removePendingAsk(request.requestId);
        return result;
      } catch (error) {
        const message = `取消失败：${String(error)}`;
        if (isStaleAskError(error)) {
          await this.refresh().catch(() => {});
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
     * pending asks remain, so it never keeps timers alive in an idle app.
     */
    startPolling(intervalMs: number = ASK_POLL_INTERVAL_MS): void {
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
          this.stopPolling();
          return;
        }
        this.schedulePoll(intervalMs);
      }, intervalMs);
    },

    stopPolling(): void {
      this.polling = false;
      if (this.pollTimer != null) {
        clearTimeout(this.pollTimer);
        this.pollTimer = null;
      }
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
