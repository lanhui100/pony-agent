import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import type { GraphAskWait, PendingAsk } from "@/types/ask-plan";

export const ASK_POLL_INTERVAL_MS = 2500;

/**
 * User-facing conflict/expiry hint. ADR 0074: wall-clock TTL is no longer product semantics, so
 * this is **not** an "expired" verdict — it only says "the host no longer has this request as
 * pending, the list has been re-synced". Mirrors the TUI `ASK_STALE_NOTICE` wording so both
 * clients speak the same Chinese sentence and never leak host internals.
 */
export const ASK_STALE_MESSAGE = "该提问已失效，已同步最新待处理请求";

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
  /** Session scope the snapshot was listed for; `null` = all sessions (host default). */
  sessionId: string | null;
  polling: boolean;
  pollingDesired: boolean;
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
/**
 * Stable host error codes for the persisted-CAS family (contract §4). These five are the
 * **only** attributable surface: a plain network failure must never be mistaken for a dead Ask
 * (it would drop a still-pending card and lose the user's answer).
 */
export const ASK_STALE_ERROR_CODES = [
  "ask_state_conflict",
  "ask_version_mismatch",
  "ask_not_found",
  "ask_nonce_mismatch",
  "ask_scope_required"
] as const;

/**
 * Detect a compare-and-swap failure from the host Ask adapter. These are not
 * retryable by resubmitting the same version — the pending request was already
 * consumed, cancelled, superseded, orphaned or asked out of scope — so callers
 * should reconcile the list instead of retrying.
 *
 * The persisted-CAS codes above are checked first; the dispatcher-era prose below is kept only
 * for the compatibility window, where older host builds still answer with free text.
 */
export function isStaleAskError(error: unknown): boolean {
  const message = String(error ?? "").toLowerCase();
  if (ASK_STALE_ERROR_CODES.some((code) => message.includes(code))) {
    return true;
  }
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

/**
 * Session scope carried with every mutating Ask call.
 *
 * The host's persisted CAS is scoped (`answer/cancel_in_session`): a mutation arriving without a
 * session scope fails closed with `ask_scope_required`, because `request_id` alone cannot prove the
 * caller is looking at the same conversation.
 *
 * The scope is the **open** session (`openSession`), i.e. what the user is actually looking at —
 * not whatever `sessionId` a listed record happens to carry, which could belong to another
 * conversation. When no session is open the argument is omitted entirely (the host then answers
 * `ask_scope_required` and the UI surfaces the shared Chinese notice).
 */
function askSessionScope(storeScope: string | null): string | undefined {
  const scoped = storeScope?.trim();
  return scoped ? scoped : undefined;
}

export const useAskStore = defineStore("ask", {
  state: (): AskState => ({
    pendingAsks: [],
    loading: false,
    error: null,
    answeringRequestId: null,
    cancellingRequestId: null,
    lastRefreshMs: null,
    sessionId: null,
    polling: false,
    pollingDesired: false,
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
     * Refresh the pending-Ask snapshot. Optionally filters by session (defaults to the scope last
     * set by `openSession`).
     *
     * ADR 0074: the host list is the only truth about "still answerable". There is no local
     * wall-clock guard any more — a request that is gone from the list was answered, ignored,
     * superseded by a newer user message, or orphaned, and its card simply disappears.
     *
     * Poll re-arm stays here: a non-empty snapshot restarts a poll that self-terminated on an
     * empty list (contract §6.6), while session switches always re-list through `openSession`.
     */
    async refresh(options?: { sessionId?: string | null }): Promise<PendingAsk[]> {
      if (!isTauriAvailable()) {
        this.pendingAsks = [];
        this.loading = false;
        return [];
      }

      this.loading = true;
      this.error = null;
      const explicitSessionId = options?.sessionId?.trim();
      const sessionId = explicitSessionId || this.sessionId || undefined;
      try {
        this.pendingAsks = await listPendingAsks(sessionId);
        this.lastRefreshMs = Date.now();
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
     * Open (or switch to) a session: re-list its pending Asks unconditionally and re-arm polling.
     *
     * Contract §6.2/§6.6: opening a session must rebuild the panel from persisted records — a
     * poll that self-terminated on an empty list must never be the reason a restart-hidden Ask
     * stays invisible. Passing `null`/empty keeps the host's "all sessions" default.
     */
    async openSession(sessionId?: string | null): Promise<PendingAsk[]> {
      this.sessionId = sessionId?.trim() || null;
      this.pollingDesired = true;
      // No eager `pendingAsks = []` here: `refresh()` replaces the snapshot atomically. Clearing
      // first would flash an empty panel (and discard any list already synced for this session),
      // while `activeAsk` in the composer already hides cards from another session id.
      return this.refresh({ sessionId: this.sessionId });
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
      // ADR 0074: no local wall-clock verdict. If the host no longer holds the request as
      // pending, its CAS rejects and we re-sync; there is nothing to pre-judge on the client.

      // Optimistically remove from pending asks to prevent duplicate submissions
      // and immediately disable UI buttons while IPC is inflight.
      this.removePendingAsk(request.requestId);
      this.answeringRequestId = request.requestId;
      this.error = null;
      const scope = askSessionScope(this.sessionId);
      try {
        const result = await safeInvoke<unknown>("ask_answer", {
          requestId: request.requestId,
          expectedVersion: listed.version,
          version: listed.version,
          ...(scope ? { sessionId: scope } : {}),
          answer: answerValue ?? null
        });
        // PA-076 phase-4 P0: `ask_answer` CAS-consumed the dispatcher request; the graph run is
        // still bound at `WaitingUser`. Resume the graph so the terminal result (the answer) is
        // injected into the next turn and the run can continue.
        await this.resumeGraphAsk(request, answerValue ?? null);
        return result;
      } catch (error) {
        if (isStaleAskError(error)) {
          // The request is already gone from the host's pending set: re-sync the list, then show
          // the shared Chinese notice — never the bare host CAS sentence (contract §6.5).
          // Order matters: `refresh()` resets `error`, so the notice must be set *after* it.
          await this.refresh().catch(() => {});
          this.error = ASK_STALE_MESSAGE;
        } else {
          // Network/internal failure: restore the ask so the user can retry it.
          if (!this.pendingAsks.some((a) => a.requestId === request.requestId)) {
            this.pendingAsks.push(listed);
          }
          this.error = "应答失败，请重试";
        }
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
        // The answer itself already landed; a resume failure is surfaced for observability only.
        // Same rule as everywhere else: host English stays out of the UI (contract §6.5).
        this.error = isStaleAskError(error) ? ASK_STALE_MESSAGE : "恢复运行失败，请稍后重试";
      }
    },

    /**
     * Ignore a pending Ask — the explicit "忽略此提问" action (contract §6.3).
     *
     * Transport is the existing `ask_cancel` command, which the host maps to the `Cancelled`
     * lifecycle state (a deliberate dismissal, distinct from `Superseded`). The card is removed
     * optimistically and never comes back, because the host no longer lists the request as
     * `Pending`.
     */
    async ignore(request: PendingAsk): Promise<unknown | null> {
      return this.cancel(request);
    },

    /**
     * Cancel/ignore a pending Ask with the listed compare-and-swap version.
     *
     * ADR 0074: no local TTL judgement — a request the host still holds as `Pending` may always
     * be cancelled; a stale one is rejected by CAS and re-synced into the shared Chinese notice.
     */
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

      // Optimistically remove from pending asks to prevent duplicate cancellations
      this.removePendingAsk(request.requestId);
      this.cancellingRequestId = request.requestId;
      this.error = null;
      const scope = askSessionScope(this.sessionId);
      try {
        const result = await safeInvoke<unknown>("ask_cancel", {
          requestId: request.requestId,
          expectedVersion: listed.version,
          version: listed.version,
          ...(scope ? { sessionId: scope } : {})
        });
        return result;
      } catch (error) {
        if (isStaleAskError(error)) {
          // Same ordering rule as `answer()`: re-list first, then surface the Chinese notice.
          await this.refresh().catch(() => {});
          this.error = ASK_STALE_MESSAGE;
        } else {
          if (!this.pendingAsks.some((a) => a.requestId === request.requestId)) {
            this.pendingAsks.push(listed);
          }
          this.error = "忽略失败，请重试";
        }
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
     * Housekeeping only (ADR 0074 / contract §5): `ask_expire` stopped being an answerability
     * verdict. It now reclaims records of archived sessions (`Orphaned`) and runs the retention
     * GC; it must **never** turn a `Pending` Ask into a dead one. Kept under the same command name
     * for IPC compatibility.
     */
    async expire(): Promise<number | null> {
      if (!isTauriAvailable()) {
        return null;
      }

      try {
        const result = await safeInvoke<{ expired?: number } | null>("ask_expire");
        await this.refresh();
        return result?.expired ?? null;
      } catch (error) {
        this.error = `清理待处理请求失败：${String(error)}`;
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
      this.stopWatchingEvents();
      this.pendingAsks = [];
      this.loading = false;
      this.error = null;
      this.answeringRequestId = null;
      this.cancellingRequestId = null;
      this.lastRefreshMs = null;
      this.sessionId = null;
    }
  }
});
