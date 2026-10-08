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
          args,
          error: String(err),
        },
        tags: {
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
