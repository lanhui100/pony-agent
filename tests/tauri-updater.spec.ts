import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useUpdateStore } from "@/stores/update";
import { SIGNED_UPDATER_ENABLED, isSignedUpdaterAvailable } from "@/lib/tauri-updater";

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
