import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

  return invoke<T>(command, args);
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
