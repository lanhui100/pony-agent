import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  reportError,
  buildIngestPayload,
  parseTraceParent,
  formatTraceParent,
  isNetworkNoise,
  setBeforeSend,
  getBeforeSend,
  type ReportErrorOptions,
  type IngestPayload,
  type TraceParentContext,
} from "@/lib/telemetry";
import * as tauriLib from "@/lib/tauri";

describe("Telemetry Governance & TraceParent Contract Suite", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setBeforeSend(null);
  });

  describe("1. L2-T: reportError & IngestPayload Contract", () => {
    it("generates a valid 32-character hexadecimal event_id and ensures idempotency format", () => {
      const options: ReportErrorOptions = {
        errorType: "ContractError",
        message: "event_id format check",
      };
      const payload = buildIngestPayload(options);

      expect(payload.event_id).toBeDefined();
      expect(typeof payload.event_id).toBe("string");
      expect(payload.event_id).toMatch(/^[0-9a-f]{32}$/i);
    });

    it("ensures tags.project defaults to 'pony-agent' and merges custom tags", () => {
      const options: ReportErrorOptions = {
        errorType: "CustomError",
        message: "custom tag merge test",
        tags: { module: "auth", stage: "wave-1" },
      };
      const payload = buildIngestPayload(options);

      expect(payload.tags).toBeDefined();
      expect(payload.tags?.project).toBe("pony-agent");
      expect(payload.tags?.module).toBe("auth");
      expect(payload.tags?.stage).toBe("wave-1");
    });

    it("preserves trace_id in payload when provided in options or context", () => {
      const options: ReportErrorOptions & { trace_id?: string } = {
        errorType: "TracedError",
        message: "error with active trace",
        trace_id: "4bf92f3577b34da6a3ce929d0e0e4736",
      };
      const payload = buildIngestPayload(options as any);

      expect(payload.trace_id).toBe("4bf92f3577b34da6a3ce929d0e0e4736");
    });
  });

  describe("2. L2-T: W3C traceparent parse & format round-trip", () => {
    it("correctly parses standard W3C traceparent header", () => {
      const header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
      const ctx = parseTraceParent(header);

      expect(ctx).not.toBeNull();
      expect(ctx).toEqual({
        version: "00",
        traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
        parentId: "00f067aa0ba902b7",
        traceFlags: "01",
      });
    });

    it("correctly formats TraceParentContext back to standard header", () => {
      const ctx: TraceParentContext = {
        version: "00",
        traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
        parentId: "00f067aa0ba902b7",
        traceFlags: "01",
      };
      const header = formatTraceParent(ctx);
      expect(header).toBe("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01");
    });

    it("round-trips parseTraceParent and formatTraceParent symmetrically", () => {
      const original = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
      const parsed = parseTraceParent(original);
      expect(parsed).not.toBeNull();
      const formatted = formatTraceParent(parsed!);
      expect(formatted).toBe(original);
    });

    it("returns null for invalid or malformed traceparent headers", () => {
      expect(parseTraceParent(null)).toBeNull();
      expect(parseTraceParent(undefined)).toBeNull();
      expect(parseTraceParent("")).toBeNull();
      expect(parseTraceParent("invalid-traceparent")).toBeNull();
      expect(parseTraceParent("00-4bf92f3577b34da6a3ce929d0e0e473-00f067aa0ba902b7-01")).toBeNull(); // 31 hex trace_id
      expect(parseTraceParent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b-01")).toBeNull(); // 15 hex parent_id
      expect(parseTraceParent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1")).toBeNull(); // 1 hex flags
    });
  });

  describe("3. L2-AT: Network Noise Filtering Adversarial Scenarios", () => {
    it("accurately detects browser network errors (Failed to fetch, NetworkError, net::ERR_)", () => {
      expect(isNetworkNoise("TypeError: Failed to fetch")).toBe(true);
      expect(isNetworkNoise("NetworkError when attempting to fetch resource.")).toBe(true);
      expect(isNetworkNoise("net::ERR_CONNECTION_REFUSED at https://example.com")).toBe(true);
      expect(isNetworkNoise("net::ERR_NAME_NOT_RESOLVED")).toBe(true);
      expect(isNetworkNoise("Load failed")).toBe(true);
    });

    it("accurately detects CORS cross-origin blocking errors", () => {
      expect(isNetworkNoise("Cross-Origin Request Blocked: The Same Origin Policy disallows reading the remote resource at ...")).toBe(true);
      expect(isNetworkNoise("CORS request did not succeed")).toBe(true);
    });

    it("accurately detects dynamic script / webpack chunk load failures", () => {
      expect(isNetworkNoise("Loading chunk 842 failed.")).toBe(true);
      expect(isNetworkNoise("ChunkLoadError: Loading chunk vendors failed")).toBe(true);
    });

    it("does NOT classify application domain crashes or logic exceptions as network noise", () => {
      expect(isNetworkNoise("TypeError: Cannot read properties of undefined (reading 'token')")).toBe(false);
      expect(isNetworkNoise("RangeError: Maximum call stack size exceeded")).toBe(false);
      expect(isNetworkNoise("RuntimeError: unreachable executed")).toBe(false);
      expect(isNetworkNoise("Error: invariant violation")).toBe(false);
      expect(isNetworkNoise("")).toBe(false);
      expect(isNetworkNoise(undefined)).toBe(false);
    });
  });

  describe("4. L2-AT: beforeSend Hook Contract & Adversarial Resilience", () => {
    it("cancels error ingestion when beforeSend returns null", async () => {
      const origVitest = process.env.VITEST;
      delete process.env.VITEST;
      try {
        const safeInvokeSpy = vi.spyOn(tauriLib, "safeInvoke").mockResolvedValue(null);
        const isTauriSpy = vi.spyOn(tauriLib, "isTauriAvailable").mockReturnValue(true);

        setBeforeSend((payload) => {
          if (payload.message?.includes("drop-me")) {
            return null;
          }
          return payload;
        });

        await reportError({
          message: "this error should drop-me",
        });

        expect(safeInvokeSpy).not.toHaveBeenCalled();
        isTauriSpy.mockRestore();
      } finally {
        process.env.VITEST = origVitest;
      }
    });

    it("allows beforeSend to mutate or enrich the payload before dispatch", async () => {
      const origVitest = process.env.VITEST;
      delete process.env.VITEST;
      try {
        let capturedPayload: IngestPayload | null = null;
        const safeInvokeSpy = vi.spyOn(tauriLib, "safeInvoke").mockImplementation(async (cmd, args: any) => {
          if (cmd === "ponysentry_capture") {
            capturedPayload = args.payload;
          }
          return null;
        });
        const isTauriSpy = vi.spyOn(tauriLib, "isTauriAvailable").mockReturnValue(true);

        setBeforeSend((payload) => {
          return {
            ...payload,
            tags: {
              ...payload.tags,
              enriched_by_hook: "true",
            },
          };
        });

        await reportError({
          message: "error to be enriched",
        });

        expect(safeInvokeSpy).toHaveBeenCalled();
        expect(capturedPayload).not.toBeNull();
        expect(capturedPayload?.tags?.enriched_by_hook).toBe("true");
        isTauriSpy.mockRestore();
      } finally {
        process.env.VITEST = origVitest;
      }
    });

    it("adversarial injection: beforeSend throwing an exception does NOT crash or break reportError dispatch", async () => {
      const origVitest = process.env.VITEST;
      delete process.env.VITEST;
      try {
        const safeInvokeSpy = vi.spyOn(tauriLib, "safeInvoke").mockResolvedValue(null);
        const isTauriSpy = vi.spyOn(tauriLib, "isTauriAvailable").mockReturnValue(true);

        setBeforeSend(() => {
          throw new Error("Malicious or buggy hook failure");
        });

        // reportError 必须保持 fire-and-forget，永不抛错，且即便 hook 崩溃也不应阻断基础错误上报主链路
        await expect(
          reportError({
            message: "critical system failure with broken hook",
          })
        ).resolves.not.toThrow();

        expect(safeInvokeSpy).toHaveBeenCalled();
        isTauriSpy.mockRestore();
      } finally {
        process.env.VITEST = origVitest;
      }
    });
  });
});
