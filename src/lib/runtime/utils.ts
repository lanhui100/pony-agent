// 通用运行时工具：日志、超时、定时调度。不依赖任何 domain 模块。

export function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`${label} 超时 (${ms}ms)`));
    }, ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      }
    );
  });
}

const DEBUG_RUNTIME_LOGS_KEY = "pony-agent.debug.runtime-logs";
let debugLoggingEnabled: boolean | null = null;

export function isDebugLoggingEnabled() {
  if (debugLoggingEnabled !== null) {
    return debugLoggingEnabled;
  }
  debugLoggingEnabled =
    typeof window !== "undefined" && window.localStorage.getItem(DEBUG_RUNTIME_LOGS_KEY) === "true";
  return debugLoggingEnabled;
}

export function debugLog(event: string, payload?: Record<string, unknown>) {
  if (!isDebugLoggingEnabled()) {
    return;
  }
  const message = {
    event,
    payload: payload ?? {},
    ts: new Date().toISOString()
  };
  console.info(`[pony-agent][runtime] ${JSON.stringify(message)}`);
}

export function errorLog(event: string, payload?: Record<string, unknown>) {
  if (!isDebugLoggingEnabled()) {
    return;
  }
  const message = {
    event,
    payload: payload ?? {},
    ts: new Date().toISOString()
  };
  console.error(`[pony-agent][runtime] ${JSON.stringify(message)}`);
}

export function reportSwitchPerf(stage: string, payload: Record<string, unknown>) {
  if (typeof window === "undefined") {
    return;
  }

  const perfWindow = window as Window & {
    __ponySwitchPerf?: Array<Record<string, unknown>>;
  };
  const entry = {
    stage,
    at: Date.now(),
    ...payload
  };
  perfWindow.__ponySwitchPerf = [...(perfWindow.__ponySwitchPerf ?? []), entry].slice(-50);
  const elapsedMs = typeof payload.elapsedMs === "number" ? payload.elapsedMs : 0;
  if (elapsedMs >= 120) {
    console.warn("[pony-agent][perf] session-switch", entry);
  }
}

export async function measureHostRead<T>(
  label: string,
  payload: Record<string, unknown>,
  run: () => Promise<T>
): Promise<T> {
  const startedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
  try {
    const result = await run();
    const elapsedMs = (typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt;
    if (elapsedMs >= 120) {
      console.warn("[pony-agent][perf] host-read", {
        label,
        elapsedMs,
        ...payload
      });
    }
    return result;
  } catch (error) {
    const elapsedMs = (typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt;
    console.warn("[pony-agent][perf] host-read-error", {
      label,
      elapsedMs,
      error: String(error),
      ...payload
    });
    throw error;
  }
}

function resolveBrowserWindow(): Window | null {
  return typeof window === "undefined" ? null : window;
}

export function safeSetTimeout(callback: () => void, delay: number): ReturnType<typeof setTimeout> {
  const browserWindow = resolveBrowserWindow();
  if (browserWindow) {
    return browserWindow.setTimeout(callback, delay);
  }

  return globalThis.setTimeout(callback, delay);
}

export function wait(ms: number) {
  return new Promise<void>((resolve) => safeSetTimeout(() => resolve(), ms));
}

export function waitForNextPaint() {
  return new Promise<void>((resolve) => {
    const browserWindow = resolveBrowserWindow();
    if (!browserWindow || typeof browserWindow.requestAnimationFrame !== "function") {
      resolve();
      return;
    }
    browserWindow.requestAnimationFrame(() => resolve());
  });
}

const LOW_PRIORITY_TURN_WORK_DELAY_MS = 800;
const LOW_PRIORITY_TURN_WORK_IDLE_TIMEOUT_MS = 2500;

export function runLowPriorityTurnWork(callback: () => void) {
  const browserWindow = resolveBrowserWindow();
  if (!browserWindow || typeof browserWindow.requestAnimationFrame !== "function") {
    callback();
    return;
  }

  safeSetTimeout(() => {
    const idleWindow = resolveBrowserWindow();
    if (!idleWindow) {
      callback();
      return;
    }

    const requestIdleCallback = (idleWindow as Window & {
      requestIdleCallback?: (handler: IdleRequestCallback, options?: IdleRequestOptions) => number;
    }).requestIdleCallback;

    if (typeof requestIdleCallback === "function") {
      requestIdleCallback(() => callback(), {
        timeout: LOW_PRIORITY_TURN_WORK_IDLE_TIMEOUT_MS
      });
      return;
    }

    safeSetTimeout(callback, 0);
  }, LOW_PRIORITY_TURN_WORK_DELAY_MS);
}
