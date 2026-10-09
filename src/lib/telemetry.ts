/**
 * PonySentry 前端错误采集与上报模块 (Vue 3 + TypeScript)
 * 遵循 Sentry 最佳实践与零信任脱敏原则 (checklist A1-G3)
 */

import { isTauriAvailable, safeInvoke } from "./tauri";

import packageJson from "../../package.json";

export const INGEST_URL =
  (import.meta.env?.VITE_PONYSENTRY_INGEST_URL as string) ||
  "https://sentry.ponyjob.top";
export const CLIENT_TOKEN =
  (import.meta.env?.VITE_PONYSENTRY_CLIENT_TOKEN as string) ||
  "6aa12e9e4294ddef559fd8f0d74626be9a313fad23a53868d5b07a88363c5d24";
export const APP_RELEASE =
  (import.meta.env?.VITE_APP_RELEASE as string) || packageJson.version || "0.1.118";
export const APP_ENV =
  (import.meta.env?.VITE_APP_ENV as string) ||
  (import.meta.env?.DEV ? "dev" : "production");

export interface Frame {
  filename?: string;
  function?: string;
  lineno?: number;
  colno?: number;
  in_app?: boolean;
}

export interface Exception {
  error_type: string;
  value?: string;
  stacktrace?: Frame[];
}

export interface Breadcrumb {
  category: string;
  message: string;
  data?: Record<string, unknown>;
  timestamp?: number;
}

export interface IngestPayload {
  platform: string;
  release: string;
  environment: string;
  message?: string;
  exception?: Exception;
  tags?: Record<string, string>;
  extra?: Record<string, unknown>;
  breadcrumbs?: Breadcrumb[];
}

// ---- 零信任脱敏规则 (客户端脱敏双保险) ----
const SENSITIVE_KEYS =
  /^(token|password|passwd|secret|api_key|apikey|access_token|refresh_token|authorization|cookie|private_key|credential)/i;

export function sanitizeString(value: string): string {
  if (typeof value !== "string") return String(value);

  let s = value
    .replace(/\/home\/[^/\s"']+/g, "[USER_HOME]")
    .replace(/\/Users\/[^/\s"']+/g, "[USER_HOME]")
    .replace(
      /[A-Za-z]:\\(?:Users|Documents and Settings)\\[^\\\s"']+/g,
      "[USER_HOME]"
    );

  // Bearer / Basic token / 常见 api key 签名
  s = s.replace(
    /(bearer\s+[a-zA-Z0-9_.\-]{10,}|basic\s+[a-zA-Z0-9+/=]{10,})/gi,
    "[REDACTED_SECRET]"
  );
  s = s.replace(/(sk-[a-zA-Z0-9_-]{16,})/gi, "[REDACTED_SECRET]");

  return s;
}

export function deepSanitize(obj: unknown, depth = 0): unknown {
  if (depth > 32) return "[MAX_DEPTH_EXCEEDED]";
  if (obj === null || obj === undefined) return obj;

  if (typeof obj === "string") {
    return sanitizeString(obj);
  }

  if (typeof obj === "number" || typeof obj === "boolean") {
    return obj;
  }

  if (Array.isArray(obj)) {
    return obj.map((item) => deepSanitize(item, depth + 1));
  }

  if (typeof obj === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
      if (SENSITIVE_KEYS.test(k)) {
        out[k] = "[REDACTED_SECRET]";
      } else {
        out[k] = deepSanitize(v, depth + 1);
      }
    }
    return out;
  }

  return String(obj);
}

// ---- 面包屑队列 (上限 64 条环形缓冲) ----
const MAX_BREADCRUMBS = 64;
const breadcrumbsQueue: Breadcrumb[] = [];

export function addBreadcrumb(
  category: string,
  message: string,
  data?: Record<string, unknown>
): void {
  const item: Breadcrumb = {
    category: sanitizeString(category),
    message: sanitizeString(message),
    timestamp: Date.now(),
  };

  if (data) {
    item.data = deepSanitize(data) as Record<string, unknown>;
  }

  breadcrumbsQueue.push(item);
  if (breadcrumbsQueue.length > MAX_BREADCRUMBS) {
    breadcrumbsQueue.shift();
  }
}

export function getBreadcrumbs(): Breadcrumb[] {
  return [...breadcrumbsQueue];
}

export function clearBreadcrumbs(): Breadcrumb[] {
  return breadcrumbsQueue.splice(0, breadcrumbsQueue.length);
}

// ---- 堆栈解析工具 ----
export function parseStackTrace(stack?: string): Frame[] {
  if (!stack || typeof stack !== "string") return [];

  const lines = stack.split("\n");
  const frames: Frame[] = [];

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("at ") && !trimmed.includes("@")) continue;

    // Chrome/V8: at functionName (filename:line:col) or at filename:line:col
    const v8Match = trimmed.match(/^at\s+(?:(.+?)\s+\((.+?):(\d+):(\d+)\)|(.+?):(\d+):(\d+))$/);
    if (v8Match) {
      const fn = v8Match[1] || undefined;
      const file = v8Match[2] || v8Match[5];
      const lineno = parseInt(v8Match[3] || v8Match[6], 10);
      const colno = parseInt(v8Match[4] || v8Match[7], 10);

      frames.push({
        filename: sanitizeString(file || ""),
        function: fn ? sanitizeString(fn) : undefined,
        lineno: isNaN(lineno) ? undefined : lineno,
        colno: isNaN(colno) ? undefined : colno,
        in_app: !file?.includes("node_modules"),
      });
      continue;
    }

    // Firefox/Safari: fn@filename:line:col
    const ffMatch = trimmed.match(/^(?:(.*?)@)?(.*?):(\d+):(\d+)$/);
    if (ffMatch) {
      const fn = ffMatch[1] || undefined;
      const file = ffMatch[2];
      const lineno = parseInt(ffMatch[3], 10);
      const colno = parseInt(ffMatch[4], 10);

      frames.push({
        filename: sanitizeString(file || ""),
        function: fn ? sanitizeString(fn) : undefined,
        lineno: isNaN(lineno) ? undefined : lineno,
        colno: isNaN(colno) ? undefined : colno,
        in_app: !file?.includes("node_modules"),
      });
    }
  }

  return frames;
}

export interface ReportErrorOptions {
  errorType?: string;
  message: string;
  stack?: string;
  frames?: Frame[];
  tags?: Record<string, string>;
  extra?: Record<string, unknown>;
  platform?: string;
}

/**
 * 核心上报方法 (Fire-and-Forget，永不阻塞或抛出异常)
 */
export async function reportError(options: ReportErrorOptions): Promise<void> {
  try {
    const errorType = options.errorType || "Error";
    const rawMessage = options.message || "Unknown error";
    const sanitizedMsg = sanitizeString(rawMessage);

    const stacktrace =
      options.frames ||
      (options.stack ? parseStackTrace(options.stack) : undefined);

    const payload: IngestPayload = {
      platform: options.platform || (isTauriAvailable() ? "tauri" : "vue"),
      release: APP_RELEASE,
      environment: APP_ENV,
      message: sanitizedMsg,
      exception: {
        error_type: sanitizeString(errorType),
        value: sanitizedMsg,
        stacktrace,
      },
      tags: options.tags
        ? (deepSanitize(options.tags) as Record<string, string>)
        : undefined,
      extra: options.extra
        ? (deepSanitize(options.extra) as Record<string, unknown>)
        : undefined,
      breadcrumbs: clearBreadcrumbs(),
    };

    // 1. 若处于桌面端环境，优先通过 Tauri IPC 委托给 core 客户端上报
    if (isTauriAvailable()) {
      try {
        await safeInvoke("ponysentry_capture", { payload });
        return;
      } catch (ipcErr) {
        // 如果 IPC 上报失败，降级到直接 fetch
        console.warn("[ponysentry] IPC capture fallback to fetch:", ipcErr);
      }
    }

    // 2. 浏览器环境或 IPC 降级：直接 HTTP POST 到 Ingest
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (CLIENT_TOKEN) {
      headers["X-Client-Token"] = CLIENT_TOKEN;
    }

    fetch(`${INGEST_URL}/api/v1/ingest`, {
      method: "POST",
      headers,
      body: JSON.stringify(payload),
      keepalive: true,
    }).catch((fetchErr) => {
      console.warn("[ponysentry] fetch ingest silent fail:", fetchErr);
    });
  } catch (err) {
    // 捕获所有潜在异常，保证遥测永远不影响前端运行
    console.warn("[ponysentry] reportError internal silent error:", err);
  }
}
