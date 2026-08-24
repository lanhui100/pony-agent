/**
 * PA-099：GitHub 发版检测纯逻辑层。
 * spec：openspec/changes/2026-08-24-add-app-update-check/proposal.md
 *
 * 信任边界约定（审核定稿）：
 * - 网络响应与 localStorage 缓存都按"不可信输入"处理，逐字段防御性校验；
 * - 跳转 URL 一律由通过格式验证的 tagName 构造（buildReleasePageUrl），
 *   API 的 html_url 全程不进入存储/状态/跳转链；
 * - 无自动重试、无定时器：自动检查只发生在 store.initialize() 的单次判定里。
 */

import type { AppReleaseInfo, UpdateCacheFile, UpdatePrefs } from "@/types/update";

export const GITHUB_OWNER = "lanhui100";
export const GITHUB_REPO = "pony-agent";
/** GitHub REST v3 latest release（语义上排除 draft/prerelease），无鉴权。 */
export const RELEASES_LATEST_URL = `https://api.github.com/repos/${GITHUB_OWNER}/${GITHUB_REPO}/releases/latest`;

/** 自动检查最小间隔；手动检查恒绕过该节流。 */
export const UPDATE_CHECK_MIN_INTERVAL_MS = 24 * 60 * 60 * 1000;
/** 单次请求超时（弱网下防止 checking 永久死锁）。 */
export const UPDATE_CHECK_TIMEOUT_MS = 8_000;

/** 符合仓库 `pony-agent.<domain>.v<N>` 惯例的 storage key。 */
export const UPDATE_CHECK_STORAGE_KEY = "pony-agent.update-check.v1";
export const UPDATE_PREFS_STORAGE_KEY = "pony-agent.update-prefs.v1";

const VERSION_TAG_PATTERN = /^[vV]?(\d+)\.(\d+)\.(\d+)$/;

export type UpdateCheckErrorCode =
  | "unpublished" // 404：仓库暂无发布版本
  | "rate-limited" // 403：匿名限流（60 req/h/IP）
  | "http-error" // 其他非 2xx
  | "malformed" // 响应不是 JSON / 字段缺失 / tag 不可解析
  | "timeout" // AbortController 超时
  | "network"; // 其余网络层失败

export class UpdateCheckError extends Error {
  readonly code: UpdateCheckErrorCode;

  constructor(code: UpdateCheckErrorCode, message: string) {
    super(message);
    this.name = "UpdateCheckError";
    this.code = code;
  }
}

/** 面向 UI 的错误文案映射（细节进 console，界面只给人话）。 */
export function describeUpdateCheckError(error: unknown): string {
  if (error instanceof UpdateCheckError) {
    switch (error.code) {
      case "unpublished":
        return "仓库暂无发布版本。";
      case "rate-limited":
        return "检查过于频繁被 GitHub 限流，请稍后再试。";
      case "timeout":
        return "检查超时，请检查网络后重试。";
      case "malformed":
        return "更新服务返回了无法识别的内容。";
      default:
        return "网络异常，暂时无法检查更新。";
    }
  }

  return "网络异常，暂时无法检查更新。";
}

/** 解析 `vX.Y.Z` 形态的 tag；预发布后缀、空白、畸形一律 null（宁漏报不误报）。 */
export function parseVersionTag(tag: unknown): [number, number, number] | null {
  if (typeof tag !== "string") {
    return null;
  }

  const match = VERSION_TAG_PATTERN.exec(tag.trim());
  if (!match) {
    return null;
  }

  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

/**
 * 三元组数值比较；任一侧不可解析返回 false（不可解析的候选绝不触发角标，
 * 当前版本不可解析则整体放弃比较——两处都是 fail-closed）。
 */
export function isNewerVersion(candidate: string, current: string): boolean {
  const candidateTriple = parseVersionTag(candidate);
  const currentTriple = parseVersionTag(current);
  if (!candidateTriple || !currentTriple) {
    return false;
  }

  for (let index = 0; index < candidateTriple.length; index += 1) {
    if (candidateTriple[index] !== currentTriple[index]) {
      return candidateTriple[index]! > currentTriple[index]!;
    }
  }

  return false;
}

/** 由已验证格式的 tagName 构造规范发布页地址；不可解析返回 null。 */
export function buildReleasePageUrl(tagName: unknown): string | null {
  if (parseVersionTag(tagName) === null) {
    return null;
  }

  const encodedTag = encodeURIComponent(String(tagName).trim());
  return `https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}/releases/tag/${encodedTag}`;
}

interface GitHubLatestReleasePayload {
  tag_name?: unknown;
  name?: unknown;
  published_at?: unknown;
}

function toFiniteMs(value: unknown): number | null {
  if (typeof value !== "string") {
    return null;
  }

  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * 拉取并校验 latest release。请求不带凭据、不走缓存、8s 超时。
 * 失败以 UpdateCheckError 分类抛出，调用方决定 UI 呈现路径。
 */
export async function fetchLatestRelease(): Promise<AppReleaseInfo> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), UPDATE_CHECK_TIMEOUT_MS);

  // 双审定稿（PA-099 code review P2）：8s 死线必须同时覆盖 fetch 与 body 读取——
  // 否则 header 到达后 body 流被代理挂起时 json() 永不 settle，checking 态永久卡死。
  try {
    let response: Response;
    try {
      response = await fetch(RELEASES_LATEST_URL, {
        method: "GET",
        headers: { Accept: "application/vnd.github+json" },
        credentials: "omit",
        cache: "no-store",
        signal: controller.signal
      });
    } catch (error) {
      if (controller.signal.aborted) {
        throw new UpdateCheckError("timeout", "update check aborted by timeout");
      }
      throw new UpdateCheckError("network", `update check fetch failed: ${String(error)}`);
    }

    if (response.status === 404) {
      throw new UpdateCheckError("unpublished", "repository has no releases");
    }
    if (response.status === 403) {
      throw new UpdateCheckError("rate-limited", `github api rate limited (${response.status})`);
    }
    if (!response.ok) {
      throw new UpdateCheckError("http-error", `unexpected status ${response.status}`);
    }

    let payload: GitHubLatestReleasePayload;
    try {
      payload = (await response.json()) as GitHubLatestReleasePayload;
    } catch (error) {
      // body 阶段到点 abort → timeout；body 被中途掐断（TypeError）→ network；
      // 其余（SyntaxError 等）才是内容畸形。
      if (controller.signal.aborted) {
        throw new UpdateCheckError("timeout", "release body read aborted by timeout");
      }
      if (error instanceof TypeError) {
        throw new UpdateCheckError("network", `release body read failed: ${String(error)}`);
      }
      throw new UpdateCheckError("malformed", `release payload is not json: ${String(error)}`);
    }

    // 某些代理/拦截器会返回字面量 null 或原始值：在访问字段前守卫。
    if (payload === null || typeof payload !== "object") {
      throw new UpdateCheckError("malformed", `release payload is not an object: ${String(payload)}`);
    }

    const tagName = typeof payload.tag_name === "string" ? payload.tag_name : null;
    if (!tagName || parseVersionTag(tagName) === null) {
      throw new UpdateCheckError("malformed", `release tag is missing or malformed: ${String(payload.tag_name)}`);
    }

    return {
      tagName,
      name: typeof payload.name === "string" && payload.name.trim().length > 0 ? payload.name : null,
      publishedAtMs: toFiniteMs(payload.published_at)
    };
  } finally {
    clearTimeout(timer);
  }
}

function readStorage(storage: Storage | null): string | null {
  if (!storage) {
    return null;
  }

  try {
    return storage.getItem(UPDATE_CHECK_STORAGE_KEY);
  } catch {
    return null;
  }
}

function resolveStorage(explicit?: Storage | null): Storage | null {
  if (explicit !== undefined) {
    return explicit;
  }

  // 隐私模式下访问 window.localStorage 本身就可能抛 SecurityError。
  if (typeof window === "undefined") {
    return null;
  }

  try {
    return window.localStorage ?? null;
  } catch {
    return null;
  }
}

function sanitizeRelease(raw: unknown): AppReleaseInfo | null {
  if (typeof raw !== "object" || raw === null) {
    return null;
  }

  const record = raw as Record<string, unknown>;
  const tagName = record.tagName;
  if (parseVersionTag(tagName) === null) {
    return null;
  }

  return {
    tagName: String(tagName),
    // 与 fetch 路径同口径：trim 后非空才算有效名。
    name: typeof record.name === "string" && record.name.trim().length > 0 ? record.name : null,
    publishedAtMs:
      typeof record.publishedAtMs === "number" && Number.isFinite(record.publishedAtMs)
        ? record.publishedAtMs
        : null
  };
}

/**
 * 读取缓存并做防御性校验；任何结构异常整体丢弃（返回 null）。
 * checkedAtMs 在此不钳制——store 水合时统一 Math.min(now)，同时约束节流判定与展示。
 */
export function loadUpdateCache(explicitStorage?: Storage | null): UpdateCacheFile | null {
  const storage = resolveStorage(explicitStorage);
  const raw = readStorage(storage);
  if (!raw) {
    return null;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  if (typeof parsed !== "object" || parsed === null) {
    return null;
  }

  const record = parsed as Record<string, unknown>;
  if (typeof record.checkedAtMs !== "number" || !Number.isFinite(record.checkedAtMs)) {
    return null;
  }

  const release = record.release === null ? null : sanitizeRelease(record.release);
  if (record.release !== null && release === null) {
    return null;
  }

  return { checkedAtMs: record.checkedAtMs, release };
}

/** 写缓存；storage 异常静默降级（配额/隐私模式不应影响主流程）。 */
export function saveUpdateCache(cache: UpdateCacheFile, explicitStorage?: Storage | null): void {
  const storage = resolveStorage(explicitStorage);
  if (!storage) {
    return;
  }

  try {
    storage.setItem(UPDATE_CHECK_STORAGE_KEY, JSON.stringify(cache));
  } catch {
    // 私有模式/配额满等场景：放弃持久化即可。
  }
}

const DEFAULT_PREFS: UpdatePrefs = { autoCheck: true };

export function loadUpdatePrefs(explicitStorage?: Storage | null): UpdatePrefs {
  const storage = resolveStorage(explicitStorage);
  if (!storage) {
    return { ...DEFAULT_PREFS };
  }

  let raw: string | null = null;
  try {
    raw = storage.getItem(UPDATE_PREFS_STORAGE_KEY);
  } catch {
    return { ...DEFAULT_PREFS };
  }

  if (!raw) {
    return { ...DEFAULT_PREFS };
  }

  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    return {
      autoCheck:
        typeof parsed.autoCheck === "boolean" ? parsed.autoCheck : DEFAULT_PREFS.autoCheck
    };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

export function saveUpdatePrefs(prefs: UpdatePrefs, explicitStorage?: Storage | null): void {
  const storage = resolveStorage(explicitStorage);
  if (!storage) {
    return;
  }

  try {
    storage.setItem(UPDATE_PREFS_STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // 同上：持久化失败不影响功能。
  }
}
