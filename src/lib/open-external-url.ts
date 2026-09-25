/**
 * PA-101 F1：外部 URL 统一出口。
 * spec：openspec/changes/pa101-update-followups/design.md（F1）+ delta spec
 * （`specs/update-security-and-release-followups/spec.md`）。
 *
 * 约束（双审定稿）：
 * - 前端所有 `open_url` 调用必须经此出口（禁直接 `safeInvoke("open_url")`，grep 门禁）；
 * - Rust 白名单拒绝（`url_allowlist_rejected:*`）fail-closed：禁止 `window.open`
 *   回退（否则 fail-closed 变 fail-open），仅告警；
 * - 仅旧二进制缺命令等其他错误允许 `window.open` 回退（判空 warn，弹窗拦截可观测）。
 */

import { isTauriAvailable, safeInvoke } from "@/lib/tauri";

export type OpenExternalUrlOutcome = "tauri" | "browser-fallback" | "blocked";

export interface OpenExternalUrlResult {
  opened: OpenExternalUrlOutcome;
  reason?: string;
}

/** Rust 白名单拒绝错误码口径（fail-closed，不可回退）。 */
const ALLOWLIST_REJECTED_CODE = "url_allowlist_rejected";

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return String(error);
  } catch {
    return "unknown error";
  }
}

/** 浏览器回退打开；返回 false 表示疑似弹窗拦截（已 warn）。 */
function openViaBrowser(url: string): boolean {
  const opened = window.open(url, "_blank", "noopener,noreferrer");
  if (!opened) {
    console.warn("[pony-agent][open-external-url] popup was blocked", url);
    return false;
  }
  return true;
}

/**
 * 打开外部 URL 的唯一出口。
 * - 非 Tauri：`window.open`（判空 warn），返回 browser-fallback；
 * - Tauri 成功：返回 tauri；
 * - Tauri 白名单拒绝：禁止回退，仅 warn，返回 blocked；
 * - Tauri 其他错误（旧二进制缺命令等）：`window.open` 回退（判空 warn），返回 browser-fallback。
 */
export async function openExternalUrl(url: string): Promise<OpenExternalUrlResult> {
  if (!isTauriAvailable()) {
    openViaBrowser(url);
    return { opened: "browser-fallback" };
  }

  try {
    await safeInvoke<void>("open_url", { url });
    return { opened: "tauri" };
  } catch (error) {
    const reason = errorText(error);
    if (reason.includes(ALLOWLIST_REJECTED_CODE)) {
      // 白名单拒绝 fail-closed：禁止 window.open 回退，仅告警。
      console.warn("[pony-agent][open-external-url] allowlist rejected, fallback refused", reason);
      return { opened: "blocked", reason };
    }
    // 旧二进制缺命令等场景：告警并回退浏览器打开。
    console.warn("[pony-agent][open-external-url] open_url failed, falling back to window.open", error);
    openViaBrowser(url);
    return { opened: "browser-fallback", reason };
  }
}
