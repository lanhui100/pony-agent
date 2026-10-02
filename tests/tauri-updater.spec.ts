import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useUpdateStore } from "@/stores/update";
import {
  mapDownloadEvent,
  parseSignedCandidate,
  SIGNED_UPDATER_ENABLED,
  isSignedUpdaterAvailable
} from "@/lib/tauri-updater";

describe("PA-103 signed updater local contract", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.stubGlobal("fetch", vi.fn());
  });

  it("fails closed when release-owner endpoint and public key are unavailable", async () => {
    const store = useUpdateStore();

    expect(SIGNED_UPDATER_ENABLED).toBe(false);
    expect(isSignedUpdaterAvailable()).toBe(false);
    expect(store.signedStatus).toBe("disabled");

    await store.checkSignedUpdate();
    await store.installSignedUpdate();

    expect(store.signedStatus).toBe("disabled");
    expect(store.signedCandidate).toBeNull();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("does not convert GitHub metadata into an installable candidate", async () => {
    const store = useUpdateStore();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ tag_name: "v99.0.0", name: null }), {
          status: 200,
          headers: { "content-type": "application/json" }
        })
      )
    );
    await store.checkForUpdates(true);

    expect(store.latest).not.toBeNull();
    expect(store.signedCandidate).toBeNull();
    expect(store.signedStatus).toBe("disabled");
  });
});

describe("mapDownloadEvent (plugin channel mapping)", () => {
  it("maps capitalised Started/Progress/Finished events emitted by the Tauri v2 plugin", () => {
    expect(mapDownloadEvent({ event: "Started", data: { contentLength: 100 } })).toEqual({
      kind: "started",
      contentLength: 100
    });
    expect(mapDownloadEvent({ event: "Progress", data: { chunkLength: 25 } })).toEqual({
      kind: "progress",
      chunkLength: 25
    });
    expect(mapDownloadEvent({ event: "Finished" })).toEqual({ kind: "finished" });
  });

  it("also maps lowercase events defensively", () => {
    expect(mapDownloadEvent({ event: "started", data: { contentLength: 10 } })).toEqual({
      kind: "started",
      contentLength: 10
    });
    expect(mapDownloadEvent({ event: "progress", data: { chunkLength: 3 } })).toEqual({
      kind: "progress",
      chunkLength: 3
    });
  });

  it("treats unknown events, missing payloads and non-objects as unknown", () => {
    expect(mapDownloadEvent({ event: "bogus" })).toEqual({ kind: "unknown" });
    expect(mapDownloadEvent({ event: "Progress", data: { chunkLength: -5 } })).toEqual({
      kind: "progress",
      chunkLength: 0
    });
    expect(mapDownloadEvent({ event: "Started", data: { contentLength: 0 } })).toEqual({
      kind: "started",
      contentLength: null
    });
    // @ts-expect-error deliberate malformed input
    expect(mapDownloadEvent(null)).toEqual({ kind: "unknown" });
  });
});

describe("parseSignedCandidate (check metadata parsing)", () => {
  it("accepts a valid candidate with an opaque resource id", () => {
    const candidate = parseSignedCandidate({ rid: 7, version: "v1.2.3", date: "2026-01-01", body: "x" });
    expect(candidate).toEqual({
      version: "v1.2.3",
      date: "2026-01-01",
      body: "x",
      handle: { resourceId: 7, version: "v1.2.3" }
    });
  });

  it("rejects missing/invalid rid or version (fail closed)", () => {
    expect(parseSignedCandidate({ rid: 0, version: "v1.2.3" })).toBeNull();
    expect(parseSignedCandidate({ rid: 1.5, version: "v1.2.3" })).toBeNull();
    expect(parseSignedCandidate({ rid: 1 })).toBeNull();
    expect(parseSignedCandidate({ rid: 1, version: "" })).toBeNull();
    expect(parseSignedCandidate({ rid: 1, version: 42 })).toBeNull();
    expect(parseSignedCandidate(null)).toBeNull();
    expect(parseSignedCandidate("x")).toBeNull();
  });
});
