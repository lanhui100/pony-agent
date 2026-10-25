import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { addBreadcrumb, reportError, sanitizeString, shouldIgnoreTelemetryError } from "./telemetry";

declare global {
  interface Window {
    __TAURI__?: unknown;
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isTauriAvailable() {
  if (typeof window === "undefined") {
    return false;
  }

  return Boolean(window.__TAURI__ || window.__TAURI_INTERNALS__);
}

const SENSITIVE_IPC_KEYS =
  /(token|secret|password|credential|authorization|api[_-]?key)/i;

export function redactIpcArgs(obj: unknown, depth = 0): unknown {
  if (depth > 16) return "[MAX_DEPTH_EXCEEDED]";
  if (obj === null || obj === undefined) return obj;

  if (typeof obj === "string") {
    return sanitizeString(obj);
  }

  if (typeof obj === "number" || typeof obj === "boolean") {
    return obj;
  }

  if (Array.isArray(obj)) {
    return obj.map((item) => redactIpcArgs(item, depth + 1));
  }

  if (typeof obj === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
      if (SENSITIVE_IPC_KEYS.test(k)) {
        out[k] = "[REDACTED]";
      } else {
        out[k] = redactIpcArgs(v, depth + 1);
      }
    }
    return out;
  }

  return sanitizeString(String(obj));
}

export function sanitizeIpcArgsForTelemetry(args?: Record<string, unknown>): unknown {
  if (!args || typeof args !== "object") {
    return args;
  }

  const redacted = redactIpcArgs(args) as Record<string, unknown>;

  // 剪裁大型嵌套对象，如 input.history 和 input.images
  if (redacted.input && typeof redacted.input === "object") {
    const inputObj = { ...(redacted.input as Record<string, unknown>) };
    if (Array.isArray(inputObj.history)) {
      inputObj.history = `[PRUNED_HISTORY: ${inputObj.history.length} turns]`;
    }
    if (Array.isArray(inputObj.images)) {
      inputObj.images = `[PRUNED_IMAGES: ${inputObj.images.length} items]`;
    }
    redacted.input = inputObj;
  }

  return redacted;
}

export async function safeInvoke<T>(command: string, args?: Record<string, unknown>) {
  if (!isTauriAvailable()) {
    throw new Error("当前运行在浏览器预览模式，Tauri 后端不可用。");
  }

  if (command !== "ponysentry_capture") {
    addBreadcrumb("ipc", `safeInvoke:${command}`);
  }

  try {
    return await invoke<T>(command, args);
  } catch (err) {
    if (command !== "ponysentry_capture") {
      const errStr = String(err);
      addBreadcrumb("ipc", `safeInvoke:${command} failed`, { error: errStr });

      // 诊断与测试桩错误直接静音，不生成 Sentry 垃圾 Issue
      if (shouldIgnoreTelemetryError(errStr)) {
        throw err;
      }

      let activeTraceId: string | undefined;
      try {
        const { useRuntimeStore } = await import("@/stores/runtime");
        activeTraceId = useRuntimeStore().activeTraceId;
      } catch {
        // ignore
      }

      // 捕获 IPC 失败异常到 PonySentry (fire-and-forget)
      reportError({
        errorType: "IpcCommandError",
        message: `IPC command "${command}" failed: ${errStr}`,
        trace_id: activeTraceId,
        extra: {
          command,
          args: sanitizeIpcArgsForTelemetry(args) as Record<string, unknown>,
          error: errStr,
        },
        tags: {
          project: "pony-agent",
          source: "tauri_safe_invoke",
          command,
        },
      });
    }
    throw err;
  }
}

export async function safeListen<T>(
  event: string,
  handler: Parameters<typeof listen<T>>[1]
) {
  if (!isTauriAvailable()) {
    return () => {};
  }

  return listen<T>(event, handler);
}
