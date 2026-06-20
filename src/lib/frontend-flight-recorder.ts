import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type {
  FrontendRecorderCapabilitySnapshot,
  FrontendRecorderStats,
  FrontendStallLevel,
  FrontendStallSnapshot,
  FrontendTraceData,
  FrontendTraceEvent,
  FrontendTraceExportPayload,
  FrontendTraceQuery,
  FrontendTraceQueryResult
} from "@/types/runtime";

type RecorderConfig = {
  stallThresholdLightMs: number;
  stallThresholdMediumMs: number;
  stallThresholdHeavyMs: number;
  timerDriftTickMs: number;
  timerDriftThresholdMs: number;
  flushIntervalMs: number;
  flushTimeoutMs: number;
  ringBufferCapacity: number;
  snapshotBufferCapacity: number;
  snapshotCooldownMs: number;
  sampleCooldownMs: number;
  maxDataEntries: number;
  maxStringLength: number;
};

type StallSnapshotBuilder = () => FrontendTraceData;

type RecorderState = {
  initialized: boolean;
  sessionId: string | null;
  turnId: string | null;
  seq: number;
  events: FrontendTraceEvent[];
  stallSnapshots: FrontendStallSnapshot[];
  flushTimerId: number | null;
  flushing: boolean;
  flushCount: number;
  flushFailureCount: number;
  droppedEventCount: number;
  droppedSnapshotCount: number;
  stallCount: number;
  lastStallGapMs: number | null;
  lastFlushDurationMs: number | null;
  lastFlushAtMs: number | null;
  lastExportAtMs: number | null;
  lastError: string | null;
  lastSnapshotAtMsByKey: Record<string, number>;
  lastSampleAtMsByKey: Record<string, number>;
  capability: FrontendRecorderCapabilitySnapshot;
  rafHandle: number | null;
  rafLastAtMs: number | null;
  timerHandle: number | null;
  timerExpectedAtMs: number | null;
  longTaskObserver: PerformanceObserver | null;
  snapshotBuilder: StallSnapshotBuilder | null;
};

const DEFAULT_CONFIG: RecorderConfig = {
  stallThresholdLightMs: 250,
  stallThresholdMediumMs: 500,
  stallThresholdHeavyMs: 1500,
  timerDriftTickMs: 100,
  timerDriftThresholdMs: 180,
  flushIntervalMs: 500,
  flushTimeoutMs: 2500,
  ringBufferCapacity: 2000,
  snapshotBufferCapacity: 100,
  snapshotCooldownMs: 1500,
  sampleCooldownMs: 150,
  maxDataEntries: 48,
  maxStringLength: 512
};

function nowWallMs() {
  return Date.now();
}

function nowPerfMs() {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

function createCapabilitySnapshot(): FrontendRecorderCapabilitySnapshot {
  const performanceObserverAvailable = typeof PerformanceObserver !== "undefined";
  const longtaskAvailable =
    performanceObserverAvailable &&
    typeof PerformanceObserver.supportedEntryTypes !== "undefined" &&
    PerformanceObserver.supportedEntryTypes.includes("longtask");

  return {
    tauriAvailable: isTauriAvailable(),
    persistenceAvailable: isTauriAvailable(),
    performanceObserverAvailable,
    longtaskAvailable,
    requestIdleCallbackAvailable:
      typeof window !== "undefined" &&
      typeof (window as Window & { requestIdleCallback?: unknown }).requestIdleCallback === "function",
    initializedAtMs: nowWallMs()
  };
}

function hasVisualMainThread() {
  if (typeof window === "undefined") {
    return false;
  }
  return typeof document === "undefined" || document.visibilityState === "visible";
}

function refreshCapabilitySnapshot() {
  recorderState.capability = createCapabilitySnapshot();
  return recorderState.capability;
}

// Module-level state init must NOT call isTauriAvailable() (avoid circular
// dependency with tauri.ts when integrated). Capability is lazily refreshed
// on first init() or flush() call.
const recorderState: RecorderState = {
  initialized: false,
  sessionId: null,
  turnId: null,
  seq: 0,
  events: [],
  stallSnapshots: [],
  flushTimerId: null,
  flushing: false,
  flushCount: 0,
  flushFailureCount: 0,
  droppedEventCount: 0,
  droppedSnapshotCount: 0,
  stallCount: 0,
  lastStallGapMs: null,
  lastFlushDurationMs: null,
  lastFlushAtMs: null,
  lastExportAtMs: null,
  lastError: null,
  lastSnapshotAtMsByKey: {},
  lastSampleAtMsByKey: {},
  capability: {
    tauriAvailable: typeof window !== "undefined" && typeof (window as Window & { __TAURI__?: unknown }).__TAURI__ !== "undefined",
    persistenceAvailable: false,
    performanceObserverAvailable: typeof PerformanceObserver !== "undefined",
    longtaskAvailable: false,
    requestIdleCallbackAvailable: false,
    initializedAtMs: 0
  },
  rafHandle: null,
  rafLastAtMs: null,
  timerHandle: null,
  timerExpectedAtMs: null,
  longTaskObserver: null,
  snapshotBuilder: null
};

let config: RecorderConfig = { ...DEFAULT_CONFIG };

function sanitizeTraceData(data?: FrontendTraceData | null) {
  if (!data) {
    return { data: null, truncated: false };
  }

  const sanitized: FrontendTraceData = {};
  let truncated = false;
  const entries = Object.entries(data).slice(0, config.maxDataEntries);
  if (entries.length < Object.keys(data).length) {
    truncated = true;
  }
  for (const [key, value] of entries) {
    if (typeof value === "string") {
      sanitized[key] = value.length > config.maxStringLength ? `${value.slice(0, config.maxStringLength)}…` : value;
      if (value.length > config.maxStringLength) {
        truncated = true;
      }
      continue;
    }
    sanitized[key] = value;
  }
  return { data: sanitized, truncated };
}

function nextSeq() {
  recorderState.seq += 1;
  return recorderState.seq;
}

function currentScope() {
  if (recorderState.turnId) {
    return "turn" as const;
  }
  if (recorderState.sessionId) {
    return "session" as const;
  }
  return recorderState.initialized ? "pre-session" as const : "global" as const;
}

function enqueueEvent(event: FrontendTraceEvent) {
  if (recorderState.events.length >= config.ringBufferCapacity) {
    recorderState.events.shift();
    recorderState.droppedEventCount += 1;
  }
  recorderState.events.push(event);
  scheduleFlush();
}

function enqueueSnapshot(snapshot: FrontendStallSnapshot) {
  if (recorderState.stallSnapshots.length >= config.snapshotBufferCapacity) {
    recorderState.stallSnapshots.shift();
    recorderState.droppedSnapshotCount += 1;
  }
  recorderState.stallSnapshots.push(snapshot);
  scheduleFlush();
}

function shouldRateLimitSample(key: string) {
  const currentAt = nowWallMs();
  const previousAt = recorderState.lastSampleAtMsByKey[key] ?? 0;
  if (currentAt - previousAt < config.sampleCooldownMs) {
    return true;
  }
  recorderState.lastSampleAtMsByKey[key] = currentAt;
  return false;
}

function scheduleFlush(overrideDelayMs?: number) {
  if (typeof window === "undefined" || recorderState.flushTimerId != null) {
    return;
  }
  const delayMs = overrideDelayMs ?? config.flushIntervalMs;
  recorderState.flushTimerId = window.setTimeout(() => {
    recorderState.flushTimerId = null;
    void flush();
  }, delayMs);
}

const MAX_RETRY_BACKOFF_MS = 30_000;

function retryBackoffMs(): number {
  // Exponential backoff: 500ms, 1s, 2s, 4s, up to 30s max
  const base = config.flushIntervalMs;
  const attempts = Math.min(recorderState.flushFailureCount, 6); // cap exponent
  return Math.min(base * Math.pow(2, attempts), MAX_RETRY_BACKOFF_MS);
}

function trimRetainedEvents(events: FrontendTraceEvent[]) {
  if (events.length <= config.ringBufferCapacity) {
    return events;
  }
  const droppedCount = events.length - config.ringBufferCapacity;
  recorderState.droppedEventCount += droppedCount;
  return events.slice(droppedCount);
}

function trimRetainedSnapshots(snapshots: FrontendStallSnapshot[]) {
  if (snapshots.length <= config.snapshotBufferCapacity) {
    return snapshots;
  }
  const droppedCount = snapshots.length - config.snapshotBufferCapacity;
  recorderState.droppedSnapshotCount += droppedCount;
  return snapshots.slice(droppedCount);
}

function requeueFlushBatch(events: FrontendTraceEvent[], stallSnapshots: FrontendStallSnapshot[]) {
  if (events.length > 0) {
    recorderState.events = trimRetainedEvents([...events, ...recorderState.events]);
  }
  if (stallSnapshots.length > 0) {
    recorderState.stallSnapshots = trimRetainedSnapshots([...stallSnapshots, ...recorderState.stallSnapshots]);
  }
  if (events.length > 0 || stallSnapshots.length > 0) {
    scheduleFlush(retryBackoffMs());
  }
}

async function flush(force = false) {
  // Don't flush unless the recorder was explicitly initialized.
  // Events recorded before init() (e.g. startup instrumentation) are
  // buffered but not sent — prevents retry storms when the backend command
  // isn't registered yet.
  if (!recorderState.initialized && !force) {
    return;
  }
  const capability = refreshCapabilitySnapshot();
  if (!capability.persistenceAvailable) {
    recorderState.lastError = "frontend trace persistence unavailable";
    return;
  }
  if (recorderState.flushing) {
    return;
  }
  if (!force && recorderState.events.length === 0 && recorderState.stallSnapshots.length === 0) {
    return;
  }

  recorderState.flushing = true;
  const flushStartedAt = nowPerfMs();
  const events = recorderState.events.splice(0, recorderState.events.length);
  const stallSnapshots = recorderState.stallSnapshots.splice(0, recorderState.stallSnapshots.length);
  try {
    await Promise.race([
      safeInvoke<void>("append_frontend_trace_events", {
        events,
        stallSnapshots
      }),
      new Promise((_, reject) => {
        window.setTimeout(() => reject(new Error("frontend trace flush timeout")), config.flushTimeoutMs);
      })
    ]);
    recorderState.flushCount += 1;
    recorderState.lastFlushAtMs = nowWallMs();
    recorderState.lastFlushDurationMs = Math.round((nowPerfMs() - flushStartedAt) * 100) / 100;
    recorderState.lastError = null;
  } catch (error) {
    recorderState.flushFailureCount += 1;
    recorderState.lastError = String(error);
    recorderState.lastFlushDurationMs = Math.round((nowPerfMs() - flushStartedAt) * 100) / 100;
    requeueFlushBatch(events, stallSnapshots);
  } finally {
    recorderState.flushing = false;
  }
}

function levelFromGap(gapMs: number): FrontendStallLevel | null {
  if (gapMs >= config.stallThresholdHeavyMs) {
    return "heavy";
  }
  if (gapMs >= config.stallThresholdMediumMs) {
    return "medium";
  }
  if (gapMs >= config.stallThresholdLightMs) {
    return "light";
  }
  return null;
}

function recordStallGap(triggerKind: string, gapMs: number, extra?: FrontendTraceData | null) {
  const stallLevel = levelFromGap(gapMs);
  if (!stallLevel) {
    return;
  }
  recorderState.stallCount += 1;
  recorderState.lastStallGapMs = gapMs;
  recordEvent("frontend.stall", "detected", "stall", {
    stallGapMs: Math.round(gapMs),
    stallLevel,
    triggerKind,
    ...(extra ?? {})
  }, { stallLevel, triggerKind });
  captureStallSnapshot(triggerKind, stallLevel, gapMs);
  if (stallLevel === "heavy") {
    void flush(true);
  }
}

function startRafDetector() {
  if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
    return;
  }
  const tick = (timestamp: number) => {
    if (recorderState.rafLastAtMs != null) {
      const gapMs = timestamp - recorderState.rafLastAtMs;
      recordStallGap("raf-gap", gapMs);
    }
    recorderState.rafLastAtMs = timestamp;
    recorderState.rafHandle = window.requestAnimationFrame(tick);
  };
  recorderState.rafHandle = window.requestAnimationFrame(tick);
}

function startTimerDriftDetector() {
  if (typeof window === "undefined") {
    return;
  }
  const schedule = () => {
    recorderState.timerExpectedAtMs = nowPerfMs() + config.timerDriftTickMs;
    recorderState.timerHandle = window.setTimeout(() => {
      const actualAt = nowPerfMs();
      const driftMs = recorderState.timerExpectedAtMs == null ? 0 : actualAt - recorderState.timerExpectedAtMs;
      if (driftMs >= config.timerDriftThresholdMs) {
        recordStallGap("timer-drift", driftMs, {
          timerDriftMs: Math.round(driftMs),
          tickMs: config.timerDriftTickMs
        });
      }
      schedule();
    }, config.timerDriftTickMs);
  };
  schedule();
}

function startLongTaskDetector() {
  if (!recorderState.capability.longtaskAvailable || typeof PerformanceObserver === "undefined") {
    return;
  }
  try {
    recorderState.longTaskObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        recordStallGap("longtask", entry.duration, {
          longtaskName: entry.name,
          longtaskDurationMs: Math.round(entry.duration)
        });
      }
    });
    recorderState.longTaskObserver.observe({ entryTypes: ["longtask"] });
  } catch (error) {
    recorderState.lastError = String(error);
  }
}

function captureStallSnapshot(triggerKind: string, stallLevel: FrontendStallLevel, stallGapMs: number) {
  const currentAt = nowWallMs();
  const cooldownKey = `${triggerKind}:${stallLevel}`;
  const previousAt = recorderState.lastSnapshotAtMsByKey[cooldownKey] ?? 0;
  if (currentAt - previousAt < config.snapshotCooldownMs) {
    return;
  }
  recorderState.lastSnapshotAtMsByKey[cooldownKey] = currentAt;
  const snapshotPayload = recorderState.snapshotBuilder?.() ?? {};
  const sanitized = sanitizeTraceData({
    triggerKind,
    stallLevel,
    stallGapMs: Math.round(stallGapMs),
    ...snapshotPayload
  });
  const snapshot: FrontendStallSnapshot = {
    sessionId: recorderState.sessionId,
    turnId: recorderState.turnId,
    tsWallMs: currentAt,
    tsPerfMs: nowPerfMs(),
    stallLevel,
    triggerKind,
    stallGapMs: Math.round(stallGapMs),
    snapshot: sanitized.data ?? {},
    truncated: sanitized.truncated
  };
  enqueueSnapshot(snapshot);
}

function recordEvent(
  category: string,
  name: string,
  kind: FrontendTraceEvent["kind"],
  data?: FrontendTraceData | null,
  options?: {
    durationMs?: number | null;
    stallLevel?: FrontendStallLevel | null;
    triggerKind?: string | null;
  }
) {
  const sanitized = sanitizeTraceData(data);
  enqueueEvent({
    seq: nextSeq(),
    tsWallMs: nowWallMs(),
    tsPerfMs: nowPerfMs(),
    sessionId: recorderState.sessionId,
    turnId: recorderState.turnId,
    scope: currentScope(),
    category,
    name,
    kind,
    durationMs: options?.durationMs ?? null,
    stallLevel: options?.stallLevel ?? null,
    triggerKind: options?.triggerKind ?? null,
    data: sanitized.data,
    truncated: sanitized.truncated
  });
}

export function initFrontendFlightRecorder(partialConfig?: Partial<RecorderConfig>) {
  if (recorderState.initialized) {
    refreshCapabilitySnapshot();
    return;
  }
  config = {
    ...DEFAULT_CONFIG,
    ...(partialConfig ?? {})
  };
  refreshCapabilitySnapshot();
  recorderState.initialized = true;
  recordEvent("frontend.recorder", "init", "instant", {
    tauriAvailable: recorderState.capability.tauriAvailable,
    persistenceAvailable: recorderState.capability.persistenceAvailable,
    longtaskAvailable: recorderState.capability.longtaskAvailable
  });
  if (hasVisualMainThread() && !import.meta.env.TEST) {
    startRafDetector();
    startTimerDriftDetector();
    startLongTaskDetector();
  }
}

export function bindFrontendRecorderSession(sessionId: string | null) {
  recorderState.sessionId = sessionId?.trim() || null;
  recordEvent("frontend.recorder", "bind-session", "instant", {
    sessionId: recorderState.sessionId
  });
}

export function bindFrontendRecorderTurn(turnId: string | null) {
  recorderState.turnId = turnId?.trim() || null;
  recordEvent("frontend.recorder", "bind-turn", "instant", {
    turnId: recorderState.turnId
  });
}

export function setFrontendRecorderSnapshotBuilder(builder: StallSnapshotBuilder | null) {
  recorderState.snapshotBuilder = builder;
}

export function recordFrontendInstant(category: string, name: string, data?: FrontendTraceData | null) {
  recordEvent(category, name, "instant", data);
}

export function recordFrontendCounter(category: string, name: string, data?: FrontendTraceData | null) {
  recordEvent(category, name, "counter", data);
}

export function recordFrontendSample(category: string, name: string, data?: FrontendTraceData | null) {
  const rateKey = `${category}:${name}`;
  if (shouldRateLimitSample(rateKey)) {
    return;
  }
  recordEvent(category, name, "sample", data);
}

export function recordFrontendSnapshot(category: string, name: string, data?: FrontendTraceData | null) {
  recordEvent(category, name, "snapshot", data);
}

export function recordFrontendStall(category: string, name: string, gapMs: number, triggerKind: string, data?: FrontendTraceData | null) {
  const stallLevel = levelFromGap(gapMs);
  if (!stallLevel) {
    return;
  }
  recordEvent(category, name, "stall", {
    stallGapMs: Math.round(gapMs),
    triggerKind,
    ...(data ?? {})
  }, {
    stallLevel,
    triggerKind
  });
}

export function injectFrontendDiagnosticStall(durationMs = 1800, label = "manual-smoke") {
  if (!import.meta.env.DEV && !import.meta.env.TEST) {
    return 0;
  }
  const boundedDurationMs = Math.max(50, Math.round(durationMs));
  recordEvent("frontend.stall", "manual-injection-start", "instant", {
    label,
    durationMs: boundedDurationMs
  });
  const startAt = nowPerfMs();
  const wallStartAt = nowWallMs();
  while (nowPerfMs() - startAt < boundedDurationMs) {
    if (nowWallMs() - wallStartAt > boundedDurationMs * 2) {
      break;
    }
    // Busy-loop intentionally blocks the main thread so detectors can capture a stall.
  }
  const actualDurationMs = Math.round((nowPerfMs() - startAt) * 100) / 100;
  recordEvent("frontend.stall", "manual-injection-end", "instant", {
    label,
    durationMs: actualDurationMs
  });
  return actualDurationMs;
}

export function startFrontendSpan(category: string, name: string, data?: FrontendTraceData | null) {
  const startedPerfMs = nowPerfMs();
  return {
    end(extra?: FrontendTraceData | null) {
      const durationMs = Math.round((nowPerfMs() - startedPerfMs) * 100) / 100;
      recordEvent(category, name, "span", {
        ...(data ?? {}),
        ...(extra ?? {})
      }, { durationMs });
      return durationMs;
    }
  };
}

export function getFrontendRecorderStats(): FrontendRecorderStats {
  return {
    initialized: recorderState.initialized,
    activeSessionId: recorderState.sessionId,
    activeTurnId: recorderState.turnId,
    seq: recorderState.seq,
    bufferedEventCount: recorderState.events.length,
    bufferedSnapshotCount: recorderState.stallSnapshots.length,
    flushCount: recorderState.flushCount,
    flushFailureCount: recorderState.flushFailureCount,
    droppedEventCount: recorderState.droppedEventCount,
    droppedSnapshotCount: recorderState.droppedSnapshotCount,
    stallCount: recorderState.stallCount,
    lastStallGapMs: recorderState.lastStallGapMs,
    lastFlushDurationMs: recorderState.lastFlushDurationMs,
    lastFlushAtMs: recorderState.lastFlushAtMs,
    lastError: recorderState.lastError,
    lastExportAtMs: recorderState.lastExportAtMs
  };
}

export function getFrontendRecorderCapabilitySnapshot() {
  return { ...recorderState.capability };
}

export async function flushAndStopFrontendFlightRecorder() {
  if (typeof window !== "undefined") {
    if (recorderState.flushTimerId != null) {
      window.clearTimeout(recorderState.flushTimerId);
      recorderState.flushTimerId = null;
    }
    if (recorderState.timerHandle != null) {
      window.clearTimeout(recorderState.timerHandle);
      recorderState.timerHandle = null;
    }
    if (recorderState.rafHandle != null) {
      window.cancelAnimationFrame(recorderState.rafHandle);
      recorderState.rafHandle = null;
    }
  }
  recorderState.longTaskObserver?.disconnect();
  recorderState.longTaskObserver = null;
  recordEvent("frontend.recorder", "stop", "instant", {
    bufferedEventCount: recorderState.events.length,
    bufferedSnapshotCount: recorderState.stallSnapshots.length
  });
  await flush(true);
}

export function __resetFrontendFlightRecorderForTests() {
  if (typeof window !== "undefined") {
    if (recorderState.flushTimerId != null) {
      window.clearTimeout(recorderState.flushTimerId);
    }
    if (recorderState.timerHandle != null) {
      window.clearTimeout(recorderState.timerHandle);
    }
    if (recorderState.rafHandle != null) {
      window.cancelAnimationFrame(recorderState.rafHandle);
    }
  }
  recorderState.longTaskObserver?.disconnect();
  recorderState.initialized = false;
  recorderState.sessionId = null;
  recorderState.turnId = null;
  recorderState.seq = 0;
  recorderState.events = [];
  recorderState.stallSnapshots = [];
  recorderState.flushTimerId = null;
  recorderState.flushing = false;
  recorderState.flushCount = 0;
  recorderState.flushFailureCount = 0;
  recorderState.droppedEventCount = 0;
  recorderState.droppedSnapshotCount = 0;
  recorderState.stallCount = 0;
  recorderState.lastStallGapMs = null;
  recorderState.lastFlushDurationMs = null;
  recorderState.lastFlushAtMs = null;
  recorderState.lastExportAtMs = null;
  recorderState.lastError = null;
  recorderState.lastSnapshotAtMsByKey = {};
  recorderState.lastSampleAtMsByKey = {};
  recorderState.capability = createCapabilitySnapshot();
  recorderState.rafHandle = null;
  recorderState.rafLastAtMs = null;
  recorderState.timerHandle = null;
  recorderState.timerExpectedAtMs = null;
  recorderState.longTaskObserver = null;
  recorderState.snapshotBuilder = null;
  config = { ...DEFAULT_CONFIG };
}

export async function queryFrontendTraceWindow(query: FrontendTraceQuery) {
  return await safeInvoke<FrontendTraceQueryResult>("query_frontend_trace_window", query);
}

export async function queryFrontendStallSnapshots(query: FrontendTraceQuery) {
  return await safeInvoke<FrontendStallSnapshot[]>("query_frontend_stall_snapshots", query);
}

export async function exportFrontendTraceJson(query: FrontendTraceQuery) {
  const result = await safeInvoke<FrontendTraceExportPayload>("export_frontend_trace_json", query);
  recorderState.lastExportAtMs = nowWallMs();
  return result;
}

export async function exportFrontendTraceChromeTrace(query: FrontendTraceQuery) {
  const result = await safeInvoke<FrontendTraceExportPayload>("export_frontend_trace_chrome_trace", query);
  recorderState.lastExportAtMs = nowWallMs();
  return result;
}
