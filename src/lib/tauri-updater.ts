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

type DownloadEvent =
  | { event: "started"; data?: { contentLength?: unknown } }
  | { event: "progress"; data?: { chunkLength?: unknown } }
  | { event: "finished"; data?: unknown };

function unavailable(): Error {
  return new Error("签名更新未配置，已安全禁用。");
}

function asPositiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0 ? value : null;
}

function asOptionalString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function parseCandidate(value: unknown): SignedUpdateCandidate | null {
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
      target: null,
      allowDowngrades: false
    });
    return parseCandidate(metadata);
  },

  async downloadAndInstall(candidate, onProgress) {
    if (!SIGNED_UPDATER_ENABLED || !import.meta.env.PROD || !isTauriAvailable()) {
      throw unavailable();
    }

    let contentLength: number | null = null;
    let downloaded = 0;
    const channel = new Channel<DownloadEvent>();
    channel.onmessage = event => {
      if (event.event === "started") {
        contentLength = asPositiveInteger(event.data?.contentLength);
      } else if (event.event === "progress") {
        const chunk = asPositiveInteger(event.data?.chunkLength) ?? 0;
        downloaded += chunk;
        onProgress?.({ downloaded, contentLength });
      }
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
