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
});
