import { Channel } from "@tauri-apps/api/core";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";

/**
 * PA-103 local contract.
 *
 * This checkout deliberately has no Pony Agent updater endpoint or verification key.
 * Keep this constant false until the release-owner gate provisions both values and CI evidence.
 * It is not configurable at runtime, so an environment variable cannot weaken the trust boundary.
 */
export const SIGNED_UPDATER_ENABLED = false;

export type SignedUpdaterHandle = Readonly<{
  /** Opaque resource id owned by the Tauri updater plugin; never a URL. */
  resourceId: number;
  version: string;
}>;

export type SignedUpdateCandidate = Readonly<{
  version: string;
  date: string | null;
  body: string | null;
  handle: SignedUpdaterHandle;
}>;

export type UpdaterDownloadProgress = Readonly<{
  downloaded: number;
  contentLength: number | null;
}>;

export interface TauriUpdaterAdapter {
  check(): Promise<SignedUpdateCandidate | null>;
  downloadAndInstall(
    candidate: SignedUpdateCandidate,
    onProgress?: (progress: UpdaterDownloadProgress) => void
  ): Promise<void>;
  relaunch(): Promise<void>;
}

type CheckMetadata = {
  rid?: unknown;
  version?: unknown;
  date?: unknown;
  body?: unknown;
};

/**
 * Raw events delivered over the plugin IPC channel. The Tauri v2 updater plugin emits
 * `Started` / `Progress` / `Finished` (capitalized); matching is normalized to
 * lowercase so the mapping is robust against any casing drift.
 */
export type RawDownloadEvent = Readonly<{
  event: unknown;
  data?: Readonly<{ contentLength?: unknown; chunkLength?: unknown }>;
}>;

export type MappedDownloadEvent =
  | { kind: "started"; contentLength: number | null }
  | { kind: "progress"; chunkLength: number }
  | { kind: "finished" }
  | { kind: "unknown" };

/** Pure mapper from a raw plugin channel event to a typed download step. */
export function mapDownloadEvent(raw: RawDownloadEvent): MappedDownloadEvent {
  if (!raw || typeof raw !== "object") {
    return { kind: "unknown" };
  }
  const name = typeof raw.event === "string" ? raw.event.toLowerCase() : "";
  const data = raw.data && typeof raw.data === "object" ? raw.data : undefined;
  if (name === "started") {
    return { kind: "started", contentLength: asPositiveInteger(data?.contentLength) };
  }
  if (name === "progress") {
    return { kind: "progress", chunkLength: asPositiveInteger(data?.chunkLength) ?? 0 };
  }
  if (name === "finished") {
    return { kind: "finished" };
  }
  return { kind: "unknown" };
}

function unavailable(): Error {
  return new Error("签名更新未配置，已安全禁用。");
}

function asPositiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0 ? value : null;
}

function asOptionalString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/**
 * Defensively parse the `plugin:updater|check` result into a typed candidate.
 * Any missing/invalid trust field rejects the whole candidate (fail closed).
 */
export function parseSignedCandidate(value: unknown): SignedUpdateCandidate | null {
  if (!value || typeof value !== "object") return null;
  const metadata = value as CheckMetadata;
  const resourceId = asPositiveInteger(metadata.rid);
  const version = asOptionalString(metadata.version);
  if (resourceId === null || version === null) return null;
  return {
    version,
    date: asOptionalString(metadata.date),
    body: asOptionalString(metadata.body),
    handle: Object.freeze({ resourceId, version })
  };
}

const productionTauriAdapter: TauriUpdaterAdapter = {
  async check() {
    if (!SIGNED_UPDATER_ENABLED || !import.meta.env.PROD || !isTauriAvailable()) {
      return null;
    }

    const metadata = await safeInvoke<CheckMetadata | null>("plugin:updater|check", {
      headers: null,
      timeout: 10_000,
      proxy: null,
      target: null
    });
    return parseSignedCandidate(metadata);
  },

  async downloadAndInstall(candidate, onProgress) {
    if (!SIGNED_UPDATER_ENABLED || !import.meta.env.PROD || !isTauriAvailable()) {
      throw unavailable();
    }

    let contentLength: number | null = null;
    let downloaded = 0;
    const channel = new Channel<RawDownloadEvent>();
    channel.onmessage = rawEvent => {
      const mapped = mapDownloadEvent(rawEvent);
      if (mapped.kind === "started") {
        contentLength = mapped.contentLength;
      } else if (mapped.kind === "progress") {
        downloaded += mapped.chunkLength;
        onProgress?.({ downloaded, contentLength });
      }
      // finished/unknown: no per-chunk progress to report.
    };

    await safeInvoke("plugin:updater|download_and_install", {
      rid: candidate.handle.resourceId,
      onEvent: channel,
      headers: null,
      timeout: 120_000
    });
  },

  async relaunch() {
    if (!SIGNED_UPDATER_ENABLED || !import.meta.env.PROD || !isTauriAvailable()) {
      throw unavailable();
    }
    await safeInvoke("plugin:process|restart");
  }
};

/** Production adapter is intentionally inert until release-owner enablement. */
export function getTauriUpdaterAdapter(): TauriUpdaterAdapter {
  return productionTauriAdapter;
}

export function isSignedUpdaterAvailable(): boolean {
  return SIGNED_UPDATER_ENABLED && import.meta.env.PROD && isTauriAvailable();
}
