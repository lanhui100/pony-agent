/**
 * P2-7 真实验收：失败轮「可重试」判定与续跑文案。
 *
 * 判定面只认稳定失败码（与后端 `retry::classify_error_class` 同口径），
 * 含畸形输入不得 throw 的红线。
 */
import { describe, expect, it } from "vitest";

import {
  classifyFailureText,
  failedTurnRetryHint,
  RETRY_RESUME_PROMPT,
} from "../src/lib/runtime/turn-retry";

describe("classifyFailureText", () => {
  it("把实测失败文本归到稳定失败码", () => {
    // pa_01a1e928：同步 followup 超时（elapsed=6424ms / operation timed out）。
    expect(classifyFailureText("provider_followup_failed: elapsed=6424ms；source=operation timed out")).toBe(
      "timeout"
    );
    // pa_01a1e908：CDN 断连（type=request，无 HTTP 状态码）。
    expect(
      classifyFailureText("调用 provider 失败：error sending request；type=request；elapsed=26951ms")
    ).toBe("network");
    expect(classifyFailureText("HTTP 429 rate limit exceeded")).toBe("rate_limited");
    expect(classifyFailureText("401 unauthorized")).toBe("auth");
    expect(classifyFailureText("quota exhausted: usage limit reached")).toBe("quota");
    expect(classifyFailureText("HTTP 503 service unavailable")).toBe("server_error");
  });

  it("畸形输入一律回落 other，绝不 throw", () => {
    expect(classifyFailureText(null)).toBe("other");
    expect(classifyFailureText(undefined)).toBe("other");
    expect(classifyFailureText(42)).toBe("other");
    expect(classifyFailureText("   ")).toBe("other");
  });
});

describe("failedTurnRetryHint", () => {
  it("瞬时故障 ⇒ 可重试标签（含「可重试」字样）", () => {
    const hint = failedTurnRetryHint("operation timed out");
    expect(hint).not.toBeNull();
    expect(hint?.retryable).toBe(true);
    expect(hint?.label).toBe("上游失败，可重试");
    expect(hint?.title).toContain("续跑");
  });

  it("配额/鉴权 ⇒ 不可重试且给出可行动原因", () => {
    const quota = failedTurnRetryHint("quota exhausted");
    expect(quota?.retryable).toBe(false);
    expect(quota?.label).toBe("上游失败，不可重试");
    expect(quota?.title).toContain("额度");

    const auth = failedTurnRetryHint("401 unauthorized");
    expect(auth?.retryable).toBe(false);
    expect(auth?.title).toContain("API Key");
  });

  it("无失败原因 ⇒ null（不渲染任何徽标）", () => {
    expect(failedTurnRetryHint("")).toBeNull();
    expect(failedTurnRetryHint(null)).toBeNull();
  });

  it("续跑文案与用户手打一致", () => {
    expect(RETRY_RESUME_PROMPT).toBe("继续");
  });
});
