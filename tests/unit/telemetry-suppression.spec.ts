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

  it("prunes bulky input history in safeInvoke error extra args", async () => {
    const { sanitizeIpcArgsForTelemetry } = await import("@/lib/tauri");
    const hugeArgs = {
      turnId: "123",
      runId: "run-abc",
      input: {
        message: "hello",
        history: [
          { role: "user", content: "hi", tokenCount: 5 },
          { role: "assistant", content: "hello world".repeat(100), reasoningContent: "long reasoning".repeat(50) }
        ],
        images: ["data:image/png;base64,1234567890"]
      }
    };

    const pruned = sanitizeIpcArgsForTelemetry(hugeArgs) as any;
    expect(pruned.turnId).toBe("123");
    expect(pruned.runId).toBe("run-abc");
    expect(pruned.input.message).toBe("hello");
    expect(pruned.input.history).toEqual("[PRUNED_HISTORY: 2 turns]");
    expect(pruned.input.images).toEqual("[PRUNED_IMAGES: 1 items]");
  });

  it("suppresses DIAG and DIAG_STUB diagnostic probe errors in reportError", async () => {
    const { shouldIgnoreTelemetryError } = await import("@/lib/telemetry");
    expect(shouldIgnoreTelemetryError("Error: DIAG_STUB_UNIMPLEMENTED:ask_list_pending")).toBe(true);
    expect(shouldIgnoreTelemetryError("Error: DIAG: sessions.db read failed")).toBe(true);
    expect(shouldIgnoreTelemetryError("IPC command \"health_check\" failed: Error: DIAG_STUB_UNIMPLEMENTED:health_check")).toBe(true);
    expect(shouldIgnoreTelemetryError("Network timeout during provider request")).toBe(false);
    expect(shouldIgnoreTelemetryError("Something crashed")).toBe(false);
  });

  it("suppresses benign ask replay/stale/expiry mutation errors in shouldIgnoreTelemetryError", async () => {
    const { shouldIgnoreTelemetryError } = await import("@/lib/telemetry");
    expect(
      shouldIgnoreTelemetryError(
        "IPC command \"ask_cancel\" failed: control request `control-000000000000` cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch"
      )
    ).toBe(true);
    expect(
      shouldIgnoreTelemetryError(
        "control request `control-000000000001` cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch"
      )
    ).toBe(true);
    expect(
      shouldIgnoreTelemetryError(
        "Ask `ask-1` expected version 1 but current version is 2; stale mutation rejected."
      )
    ).toBe(true);
    expect(
      shouldIgnoreTelemetryError("unknown control request `control-000000000000`")
    ).toBe(true);
  });
});
