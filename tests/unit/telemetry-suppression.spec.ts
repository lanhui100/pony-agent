import { describe, it, expect, vi, beforeEach } from "vitest";
import { reportError } from "@/lib/telemetry";
import * as tauriLib from "@/lib/tauri";

describe("telemetry reportError test environment suppression", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("suppresses reportError calls during vitest / test execution", async () => {
    const safeInvokeSpy = vi.spyOn(tauriLib, "safeInvoke").mockResolvedValue(null);
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response());

    await reportError({
      errorType: "TurnExecutionFailed",
      message: "tool chain exploded"
    });

    expect(safeInvokeSpy).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("injects project: 'pony-agent' into reportError payload tags via buildIngestPayload", async () => {
    const { buildIngestPayload } = await import("@/lib/telemetry");
    const payload = buildIngestPayload({
      errorType: "CustomError",
      message: "something happened",
      tags: { custom_tag: "foo" }
    });

    expect(payload).toBeDefined();
    expect(payload.tags).toBeDefined();
    expect(payload.tags?.project).toBe("pony-agent");
    expect(payload.tags?.custom_tag).toBe("foo");
  });
});
