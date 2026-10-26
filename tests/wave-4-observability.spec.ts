import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  buildIngestPayload,
  getBeforeSend,
  setBeforeSend,
  reportError,
  type ReportErrorOptions,
  type IngestPayload,
} from "@/lib/telemetry";

describe("Wave-4 Frontend Observability Governance Contract Suite", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  describe("C2 契约：buildIngestPayload event_id 与 trace_id 契约", () => {
    it("产出的 payload 必须含 32 位小写十六进制 event_id", () => {
      const options: ReportErrorOptions = {
        errorType: "TypeError",
        message: "cannot read properties of undefined",
      };
      const payload = buildIngestPayload(options);

      expect(payload.event_id).toBeDefined();
      expect(typeof payload.event_id).toBe("string");
      expect(payload.event_id).toMatch(/^[0-9a-f]{32}$/);
    });

    it("传入 trace_id 时原样保全，未传时必须为 undefined/不出现（严禁占位假值）", () => {
      // 1. 显式传入有效 trace_id
      const payloadWithTrace = buildIngestPayload({
        message: "error with active trace",
        trace_id: "4bf92f3577b34da6a3ce929d0e0e4736",
      } as any);
      expect(payloadWithTrace.trace_id).toBe("4bf92f3577b34da6a3ce929d0e0e4736");

      // 2. 未传 trace_id 时，必须为 undefined 或 null，禁止填充假值如 '00000000000000000000000000000000' 或 'dummy'
      const payloadWithoutTrace = buildIngestPayload({
        message: "error without trace",
      });
      expect(payloadWithoutTrace.trace_id).toBeUndefined();
    });
  });

  describe("C4 契约：默认 beforeSend 注册与凭据清洗截断治理", () => {
    it("模块初始化时必须默认注册全局 beforeSend 钩子（getBeforeSend 缺省非 null）", () => {
      // 契约：src/lib/telemetry.ts 模块加载后，必须具有默认的 beforeSend 治理钩子
      const hook = getBeforeSend();
      expect(hook).not.toBeNull();
      expect(typeof hook).toBe("function");
    });

    it("默认 beforeSend 必须对 extra 中的 Authorization Bearer token 深度脱敏并裁剪超长字段", async () => {
      const hook = getBeforeSend();
      expect(hook).not.toBeNull();

      const rawPayload: IngestPayload = {
        platform: "vue",
        release: "0.1.155",
        environment: "production",
        event_id: "1234567890abcdef1234567890abcdef",
        message: "Sensitive network dump",
        extra: {
          authHeader: "Bearer sk-test-dummy-token-0000",
          nested: {
            token: "secret-token-xyz-123456789",
          },
          hugePayload: "A".repeat(5000), // 超长字段应被安全裁剪
        },
      };

      const sanitized = await hook!(rawPayload);
      expect(sanitized).not.toBeNull();
      expect(sanitized!.extra).toBeDefined();

      const extra = sanitized!.extra as Record<string, any>;
      // Authorization token 必须被脱敏
      expect(extra.authHeader).not.toContain("sk-test-dummy-token-0000");
      expect(extra.nested.token).toBe("[REDACTED_SECRET]");
      // 超长字段必须被裁剪截断，防止单次上报 payload 过大
      expect(typeof extra.hugePayload).toBe("string");
      expect(extra.hugePayload.length).toBeLessThanOrEqual(2048);
    });
  });
});
