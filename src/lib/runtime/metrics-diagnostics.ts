import { debugLog } from "@/lib/runtime/utils";
import { addBreadcrumb, reportError } from "@/lib/telemetry";

export interface MetricsSnapshot {
  turnId?: string | null;
  mode: string;
  speed: number | null;
  inputTokens: number | null;
  contextWindow: number;
  contextRatio?: number | null;
  outputTokens?: number | null;
  durationMs?: number | null;
  ttftMs?: number | null;
}

let lastMetricsSignature: string | null = null;
let metricsBreadcrumbCount = 0;
const MAX_METRICS_BREADCRUMBS = 8;

const anomalyThrottleMap = new Map<string, number>();
const ANOMALY_THROTTLE_WINDOW_MS = 60_000;

export function recordMetricsSnapshot(snapshot: MetricsSnapshot): void {
  const roundedSpeed = snapshot.speed != null ? Math.round(snapshot.speed) : "null";
  const signature = `${snapshot.mode}|${roundedSpeed}|${snapshot.inputTokens ?? "null"}|${snapshot.contextWindow}`;

  if (signature === lastMetricsSignature) {
    return;
  }
  lastMetricsSignature = signature;

  debugLog("metrics:token-speed", {
    turnId: snapshot.turnId ?? undefined,
    mode: snapshot.mode,
    speed: snapshot.speed ?? undefined,
    inputTokens: snapshot.inputTokens ?? undefined,
    contextWindow: snapshot.contextWindow,
    contextRatio: snapshot.contextRatio ?? undefined,
    outputTokens: snapshot.outputTokens ?? undefined,
    durationMs: snapshot.durationMs ?? undefined,
    ttftMs: snapshot.ttftMs ?? undefined,
  });

  if (metricsBreadcrumbCount < MAX_METRICS_BREADCRUMBS) {
    try {
      addBreadcrumb("metrics", "token_speed", {
        turnId: snapshot.turnId ?? undefined,
        mode: snapshot.mode,
        speed: snapshot.speed != null ? Math.round(snapshot.speed * 100) / 100 : undefined,
        inputTokens: snapshot.inputTokens ?? undefined,
        contextWindow: snapshot.contextWindow,
        contextRatio: snapshot.contextRatio ?? undefined,
        outputTokens: snapshot.outputTokens ?? undefined,
        durationMs: snapshot.durationMs ?? undefined,
        ttftMs: snapshot.ttftMs ?? undefined,
      });
      metricsBreadcrumbCount++;
    } catch {
      // 忽略面包屑副作用异常
    }
  }
}

export function reportMetricsAnomaly(
  code: "invalid_duration" | "negative_speed" | "context_overflow",
  detail: Record<string, unknown>
): void {
  const now = Date.now();
  const lastEmitted = anomalyThrottleMap.get(code);
  if (lastEmitted && now - lastEmitted < ANOMALY_THROTTLE_WINDOW_MS) {
    return;
  }
  anomalyThrottleMap.set(code, now);

  try {
    reportError({
      errorType: "TelemetryMetricsAnomaly",
      message: `[metrics-anomaly] ${code}`,
      extra: detail,
      tags: {
        category: "metrics_governance",
        severity: "warning",
      },
    });
  } catch {
    // 忽略异常上报副作用，确保绝不阻断主链路
  }
}
