import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { addBreadcrumb, reportError } from "./telemetry";

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

export function sanitizeIpcArgsForTelemetry(args?: Record<string, unknown>): unknown {
  if (!args || typeof args !== "object") {
    return args;
  }

  const sanitized: Record<string, unknown> = { ...args };

  // 剪裁大型嵌套对象，如 input.history 和 input.images
  if (sanitized.input && typeof sanitized.input === "object") {
    const inputObj = { ...(sanitized.input as Record<string, unknown>) };
    if (Array.isArray(inputObj.history)) {
      inputObj.history = `[PRUNED_HISTORY: ${inputObj.history.length} turns]`;
    }
    if (Array.isArray(inputObj.images)) {
      inputObj.images = `[PRUNED_IMAGES: ${inputObj.images.length} items]`;
    }
    sanitized.input = inputObj;
  }

  return sanitized;
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
      addBreadcrumb("ipc", `safeInvoke:${command} failed`, { error: String(err) });
      // 捕获 IPC 失败异常到 PonySentry (fire-and-forget)
      reportError({
        errorType: "IpcCommandError",
        message: `IPC command "${command}" failed: ${String(err)}`,
        extra: {
          command,
          args: sanitizeIpcArgsForTelemetry(args) as Record<string, unknown>,
          error: String(err),
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
